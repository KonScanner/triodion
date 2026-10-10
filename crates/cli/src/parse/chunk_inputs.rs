//! The values of `--blocks` and `--timestamps`.
//!
//! Both flags take the same two kinds of value: a parquet file on disk, or an
//! expression in the range grammar of the flag. This module owns what the two
//! flags share: the split between files and expressions, the column reader,
//! the labels, and the post-processing of the chunks. The grammar of each unit
//! stays in [`blocks`] and [`timestamps`], because the two grammars differ on
//! purpose (a block range takes a step, a time offset takes units).

use std::sync::Arc;

use polars::prelude::*;
use triodion_core::{BlockChunk, ParseError, Source};

use super::{blocks, timestamps};
use crate::args::Args;

/// One label for each chunk: the name of its file, or `None` for a chunk
/// from an expression.
pub(crate) type ChunkLabels = Vec<Option<String>>;

/// The unit of the values of `--blocks` or `--timestamps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChunkUnit {
    /// Block numbers.
    Block,
    /// Unix timestamps, in seconds, converted to block numbers.
    Timestamp,
}

impl ChunkUnit {
    /// The column read from a file.
    fn default_column(self) -> &'static str {
        match self {
            Self::Block => "block_number",
            Self::Timestamp => "timestamp",
        }
    }

    /// The name of one value, for error messages.
    fn noun(self) -> &'static str {
        match self {
            Self::Block => "block number",
            Self::Timestamp => "timestamp",
        }
    }
}

/// Where a reference sits in a range. It decides what an empty reference
/// means: the start of the chain for the first, the head for the last.
#[derive(Clone, Debug)]
pub(crate) enum RangePosition {
    First,
    Last,
    None,
}

/// Parse the values of `--blocks` or `--timestamps` into block chunks.
///
/// A value that names a file on disk gives one chunk with the values of the
/// column of that file; timestamps are converted to block numbers. Every other
/// value is an expression in the grammar of `unit`, and its chunks are
/// aligned, split, and filtered by the reorg buffer. File chunks come first.
///
/// Returns `(None, None)` when `values` is `None`. The labels are `Some` only
/// when at least one value is a file.
pub(crate) async fn parse_chunk_inputs(
    values: Option<&[String]>,
    unit: ChunkUnit,
    args: &Args,
    source: Arc<Source>,
) -> Result<(Option<ChunkLabels>, Option<Vec<BlockChunk>>), ParseError> {
    let Some(values) = values else { return Ok((None, None)) };
    let (files, expressions): (Vec<&String>, Vec<&String>) =
        values.iter().partition(|value| std::path::Path::new(value).exists());

    let mut file_labels = Vec::with_capacity(files.len());
    let mut chunks = Vec::with_capacity(values.len());
    for path in &files {
        let column = if path.contains(':') {
            path.split(':')
                .next_back()
                .ok_or(ParseError::ParseError("could not parse path column".to_string()))?
        } else {
            unit.default_column()
        };
        let values = read_integer_column(path, column, unit)?;
        let numbers = match unit {
            ChunkUnit::Block => values,
            ChunkUnit::Timestamp => {
                timestamps::timestamps_to_block_numbers(values, source.clone()).await?
            }
        };
        chunks.push(BlockChunk::Numbers(numbers));
        file_labels.push(
            path.split("__").last().and_then(|s| s.strip_suffix(".parquet")).map(String::from),
        );
    }

    let mut expression_chunks = Vec::new();
    for expression in &expressions {
        expression_chunks.extend(match unit {
            ChunkUnit::Block => blocks::parse_block_inputs(expression, source.clone()).await?,
            ChunkUnit::Timestamp => {
                timestamps::parse_timestamp_inputs(expression, source.clone()).await?
            }
        });
    }
    if !expressions.is_empty() {
        expression_chunks =
            blocks::postprocess_block_chunks(expression_chunks, args, source).await?;
    }

    let labels = (!files.is_empty()).then(|| {
        let mut labels = file_labels;
        labels.resize(labels.len() + expression_chunks.len(), None);
        labels
    });
    chunks.extend(expression_chunks);
    Ok((labels, Some(chunks)))
}

/// Read the distinct values of an integer column of a parquet file.
fn read_integer_column(path: &str, column: &str, unit: ChunkUnit) -> Result<Vec<u64>, ParseError> {
    let file = std::fs::File::open(path)
        .map_err(|_e| ParseError::ParseError("could not open file path".to_string()))?;

    let df = ParquetReader::new(file)
        .with_columns(Some(vec![column.to_string()]))
        .finish()
        .map_err(|_e| ParseError::ParseError("could not read data from column".to_string()))?;

    let series = df
        .column(column)
        .map_err(|_e| ParseError::ParseError("could not get column".to_string()))?
        .unique()
        .map_err(|_e| ParseError::ParseError("could not get column".to_string()))?;

    let missing = || ParseError::ParseError(format!("{} missing", unit.noun()));
    match series.u32() {
        Ok(ca) => ca.iter().map(|v| v.map(u64::from).ok_or_else(missing)).collect(),
        Err(_e) => match series.u64() {
            Ok(ca) => ca.iter().map(|v| v.ok_or_else(missing)).collect(),
            Err(_e) => {
                Err(ParseError::ParseError("could not convert to integer column".to_string()))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `column` to a parquet file and read it back through
    /// `read_integer_column`. This covers the polars 0.55 change where
    /// `&ChunkedArray<UInt32Type>` and `&ChunkedArray<UInt64Type>` stopped
    /// implementing `IntoIterator`.
    fn read_integer_column_helper(name: &str, column: Column) {
        let path = std::env::temp_dir().join(format!("triodion_{}.parquet", name));
        let mut df = DataFrame::new(3, vec![column]).unwrap();

        let file = std::fs::File::create(&path).unwrap();
        ParquetWriter::new(file).finish(&mut df).unwrap();

        let mut read =
            read_integer_column(path.to_str().unwrap(), "number", ChunkUnit::Block).unwrap();
        std::fs::remove_file(&path).unwrap();

        read.sort_unstable();
        assert_eq!(read, vec![10u64, 20, 30]);
    }

    #[test]
    fn read_integer_column_reads_u32() {
        let column = Column::new("number".into(), vec![10u32, 20, 30]);
        read_integer_column_helper("chunk_inputs_u32", column);
    }

    #[test]
    fn read_integer_column_reads_u64() {
        let column = Column::new("number".into(), vec![10u64, 20, 30]);
        read_integer_column_helper("chunk_inputs_u64", column);
    }

    #[tokio::test]
    async fn a_block_file_gives_one_labelled_chunk_with_no_rpc() {
        let path = std::env::temp_dir().join("triodion_chunk_inputs__labelled.parquet");
        let column = Column::new("block_number".into(), vec![7u64]);
        let mut df = DataFrame::new(1, vec![column]).unwrap();
        ParquetWriter::new(std::fs::File::create(&path).unwrap()).finish(&mut df).unwrap();

        // The mock has no answers queued, so any request would fail the test.
        let provider = alloy::providers::ProviderBuilder::default()
            .connect_mocked_client(alloy::transports::mock::Asserter::new());
        let config = triodion_core::SourceConfig::new(String::new());
        let source = Arc::new(Source::from_provider(provider, 1, &config));
        let values = vec![path.to_str().unwrap().to_string()];

        let parsed =
            parse_chunk_inputs(Some(&values), ChunkUnit::Block, &Args::cli_defaults(), source)
                .await;
        std::fs::remove_file(&path).unwrap();

        let (labels, chunks) = parsed.expect("a block file needs no rpc");
        assert_eq!(labels, Some(vec![Some("labelled".to_string())]));
        let Some([BlockChunk::Numbers(numbers)]) = chunks.as_deref() else {
            panic!("expected one chunk of numbers, got {chunks:?}")
        };
        assert_eq!(numbers, &vec![7]);
    }

    /// The smallest pre-London block that the provider deserializes.
    fn block(number: u64, timestamp: u64) -> serde_json::Value {
        let word = format!("0x{}", "00".repeat(32));
        serde_json::json!({
            "hash": word,
            "parentHash": word,
            "sha3Uncles": word,
            "miner": format!("0x{}", "00".repeat(20)),
            "stateRoot": word,
            "transactionsRoot": word,
            "receiptsRoot": word,
            "logsBloom": format!("0x{}", "00".repeat(256)),
            "difficulty": "0x0",
            "number": format!("{number:#x}"),
            "gasLimit": "0x0",
            "gasUsed": "0x0",
            "timestamp": format!("{timestamp:#x}"),
            "extraData": "0x",
            "mixHash": word,
            "nonce": "0x0000000000000000",
            "transactions": [],
            "uncles": [],
        })
    }

    #[tokio::test]
    async fn a_timestamp_file_gives_block_numbers_not_timestamps() {
        let path = std::env::temp_dir().join("triodion_chunk_inputs__timestamps.parquet");
        let column = Column::new("timestamp".into(), vec![112u64]);
        let mut df = DataFrame::new(1, vec![column]).unwrap();
        ParquetWriter::new(std::fs::File::create(&path).unwrap()).finish(&mut df).unwrap();

        // Blocks 0, 1, 2 at 100, 112, 124: the head, then the one probe the
        // search needs, block 1, which matches exactly.
        let asserter = alloy::transports::mock::Asserter::new();
        asserter.push_success(&alloy::primitives::U64::from(2));
        asserter.push_success(&block(1, 112));
        let provider = alloy::providers::ProviderBuilder::default().connect_mocked_client(asserter);
        let config = triodion_core::SourceConfig::new(String::new());
        let source = Arc::new(Source::from_provider(provider, 1, &config));
        let values = vec![path.to_str().unwrap().to_string()];

        let parsed =
            parse_chunk_inputs(Some(&values), ChunkUnit::Timestamp, &Args::cli_defaults(), source)
                .await;
        std::fs::remove_file(&path).unwrap();

        let (labels, chunks) = parsed.expect("the mock answers every request");
        assert_eq!(labels, Some(vec![Some("timestamps".to_string())]));
        let Some([BlockChunk::Numbers(numbers)]) = chunks.as_deref() else {
            panic!("expected one chunk of numbers, got {chunks:?}")
        };
        assert_eq!(numbers, &vec![1], "the timestamp 112 is block 1");
    }
}
