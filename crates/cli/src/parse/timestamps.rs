use alloy::rpc::types::BlockTransactionsKind;
use polars::prelude::*;
use triodion_core::{BlockChunk, ParseError, Source};

use crate::parse::{
    blocks::block_range_to_block_chunk, chunk_inputs::RangePosition, parse_utils::f64_to_u64,
};

use super::blocks::get_latest_block_number;

/// parse timestamp numbers to freeze
pub(crate) async fn parse_timestamp_inputs(
    inputs: &str,
    source: Arc<Source>,
) -> Result<Vec<BlockChunk>, ParseError> {
    let parts: Vec<&str> = inputs.split(' ').collect();
    match parts.len() {
        1 => {
            let first_input = parts.first().ok_or_else(|| {
                ParseError::ParseError("Failed to get the first input".to_string())
            })?;
            parse_timestamp_token(first_input, true, source).await.map(|x| vec![x])
        }
        _ => {
            let mut chunks = Vec::new();
            for part in parts {
                chunks.push(parse_timestamp_token(part, false, source.clone()).await?);
            }
            Ok(chunks)
        }
    }
}
async fn parse_timestamp_token(
    s: &str,
    as_range: bool,
    source: Arc<Source>,
) -> Result<BlockChunk, ParseError> {
    let s = s.replace('_', "");

    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        [timestamp_ref] => {
            let timestamp =
                parse_timestamp_number(timestamp_ref, RangePosition::None, source.clone()).await?;
            let block = timestamp_to_block_number(timestamp, source).await?;

            Ok(BlockChunk::Numbers(vec![block]))
        }
        [first_ref, second_ref] => {
            let parts: Vec<_> = second_ref.split('/').collect();
            let (second_ref, n_keep) = if parts.len() == 2 {
                let n_keep = parts[1].parse::<u32>().map_err(|_| {
                    ParseError::ParseError("cannot parse timestamp interval size".to_string())
                })?;
                (parts[0], Some(n_keep))
            } else {
                (*second_ref, None)
            };

            let (start_timestamp, end_timestamp) =
                parse_timestamp_range(first_ref, second_ref, source.clone()).await?;
            let (start_block, end_block) = (
                timestamp_to_block_number(start_timestamp, source.clone()).await?,
                timestamp_to_block_number(end_timestamp, source).await?,
            );
            block_range_to_block_chunk(start_block, end_block, as_range, None, n_keep)
        }
        _ => Err(ParseError::ParseError(
            "timestamps must be in format timestamp or start_timestamp:end_timestamp".to_string(),
        )),
    }
}

async fn parse_timestamp_range(
    first_ref: &str,
    second_ref: &str,
    source: Arc<Source>,
) -> Result<(u64, u64), ParseError> {
    let (start_timestamp, end_timestamp) = match (first_ref, second_ref) {
        _ if first_ref.starts_with('-') => {
            let end_timestamp =
                parse_timestamp_number(second_ref, RangePosition::Last, source.clone()).await?;

            let start_timestamp = end_timestamp
                .checked_sub(
                    parse_timestamp_number(&first_ref[1..], RangePosition::None, source).await?,
                )
                .ok_or_else(|| ParseError::ParseError("start_timestamp underflow".to_string()))?;

            (start_timestamp, end_timestamp)
        }
        _ if second_ref.starts_with('+') => {
            let start_timestamp =
                parse_timestamp_number(first_ref, RangePosition::First, source.clone()).await?;

            let end_timestamp = start_timestamp
                .checked_add(
                    parse_timestamp_number(&second_ref[1..], RangePosition::None, source).await?,
                )
                .ok_or_else(|| ParseError::ParseError("end_timestamp overflow".to_string()))?;

            (start_timestamp, end_timestamp)
        }
        _ => {
            let start_timestamp =
                parse_timestamp_number(first_ref, RangePosition::First, source.clone()).await?;

            let end_timestamp =
                parse_timestamp_number(second_ref, RangePosition::Last, source).await?;

            (start_timestamp, end_timestamp)
        }
    };

    let end_timestamp =
        if second_ref != "latest" && !second_ref.is_empty() && !first_ref.starts_with('-') {
            // Checked for the reason given in `blocks::parse_block_range`:
            // `-t 0:0` panicked in debug and wrapped to `u64::MAX` in release.
            end_timestamp.checked_sub(1).ok_or_else(|| {
                ParseError::ParseError(
                    "end_timestamp should not be less than start_timestamp".to_string(),
                )
            })?
        } else {
            end_timestamp
        };

    Ok((start_timestamp, end_timestamp))
}

async fn parse_timestamp_number(
    timestamp_ref: &str,
    range_position: RangePosition,
    source: Arc<Source>,
) -> Result<u64, ParseError> {
    match (timestamp_ref, range_position) {
        ("latest", _) => get_latest_timestamp(source).await,
        ("", RangePosition::First) => Ok(0),
        ("", RangePosition::Last) => get_latest_timestamp(source).await,
        ("", RangePosition::None) => Err(ParseError::ParseError("invalid input".to_string())),
        _ if timestamp_ref.ends_with('m') => scale_timestamp_str_by_metric_unit(timestamp_ref, 60),
        _ if timestamp_ref.ends_with('h') => {
            scale_timestamp_str_by_metric_unit(timestamp_ref, 3600)
        }
        _ if timestamp_ref.ends_with('d') => {
            scale_timestamp_str_by_metric_unit(timestamp_ref, 86400)
        }
        _ if timestamp_ref.ends_with('w') => {
            scale_timestamp_str_by_metric_unit(timestamp_ref, 86400 * 7)
        }
        _ if timestamp_ref.ends_with('M') => {
            scale_timestamp_str_by_metric_unit(timestamp_ref, 86400 * 30)
        }
        _ if timestamp_ref.ends_with('y') => {
            scale_timestamp_str_by_metric_unit(timestamp_ref, 86400 * 365)
        }
        _ => timestamp_ref
            .parse::<f64>()
            .map_err(|_e| ParseError::ParseError("Error parsing timestamp ref".to_string()))
            .and_then(|x| f64_to_u64(x, "timestamp ref")),
    }
}

fn scale_timestamp_str_by_metric_unit(
    timestamp_ref: &str,
    metric_scale: u64,
) -> Result<u64, ParseError> {
    let s = &timestamp_ref[..timestamp_ref.len() - 1];
    s.parse::<f64>()
        .map_err(|_e| ParseError::ParseError("Error parsing timestamp ref".to_string()))
        .and_then(|n| f64_to_u64((metric_scale as f64 * n).round(), "timestamp ref"))
}

/// Convert the timestamps read from a file to the distinct block numbers
/// that were current at those times, in ascending order.
///
/// Each timestamp is a separate binary search, so a file of `n` timestamps
/// costs about `n * log2(head)` block reads.
pub(crate) async fn timestamps_to_block_numbers(
    timestamps: Vec<u64>,
    source: Arc<Source>,
) -> Result<Vec<u64>, ParseError> {
    let latest_block_number = get_latest_block_number(source.clone()).await?;
    let mut block_numbers = Vec::with_capacity(timestamps.len());
    for timestamp in timestamps {
        block_numbers.push(
            block_at_timestamp(timestamp, latest_block_number, |number| {
                block_timestamp(number, source.clone())
            })
            .await?,
        );
    }
    block_numbers.sort_unstable();
    block_numbers.dedup();
    Ok(block_numbers)
}

/// A block whose timestamp is at or before `timestamp`; see
/// [`block_at_timestamp`].
async fn timestamp_to_block_number(timestamp: u64, source: Arc<Source>) -> Result<u64, ParseError> {
    let latest_block_number = get_latest_block_number(source.clone()).await?;
    block_at_timestamp(timestamp, latest_block_number, |number| {
        block_timestamp(number, source.clone())
    })
    .await
}

/// Binary search over blocks `0..=latest` for the last block whose timestamp
/// is at or before `timestamp`. A timestamp before block 0 gives block 0.
///
/// When several blocks share exactly `timestamp`, as on chains with blocks
/// shorter than one second, the search returns the first of them that it
/// probes, which is not always the last.
///
/// `timestamp_of` reads the timestamp of one block, so the search can be
/// tested without a node.
async fn block_at_timestamp<F, Fut>(
    timestamp: u64,
    latest: u64,
    mut timestamp_of: F,
) -> Result<u64, ParseError>
where
    F: FnMut(u64) -> Fut,
    Fut: std::future::Future<Output = Result<u64, ParseError>>,
{
    let (mut low, mut high) = (0, latest);
    while low <= high {
        let mid = low + (high - low) / 2;
        match timestamp_of(mid).await?.cmp(&timestamp) {
            std::cmp::Ordering::Equal => return Ok(mid),
            std::cmp::Ordering::Less => low = mid + 1,
            // `mid - 1` was unchecked: a timestamp before block 0 panicked in
            // debug, and in release wrapped to `u64::MAX` and then panicked
            // on the missing block.
            std::cmp::Ordering::Greater => match mid.checked_sub(1) {
                Some(below) => high = below,
                None => return Ok(0),
            },
        }
    }
    Ok(high)
}

async fn block_timestamp(number: u64, source: Arc<Source>) -> Result<u64, ParseError> {
    source
        .get_block(number, BlockTransactionsKind::Hashes)
        .await
        .map_err(|_e| ParseError::ParseError("Error fetching block for timestamp".to_string()))?
        .map(|block| block.header.timestamp)
        .ok_or_else(|| ParseError::ParseError(format!("block {number} not found")))
}

async fn get_latest_timestamp(source: Arc<Source>) -> Result<u64, ParseError> {
    let latest_block_number = get_latest_block_number(source.clone()).await?;
    block_timestamp(latest_block_number, source).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use triodion_core::SourceConfig;

    /// Build a live `Source`, or `None` when no RPC endpoint is configured.
    ///
    /// This used to `std::process::exit(0)` when `ETH_RPC_URL` was unset. That
    /// does not skip the test — it terminates the whole test binary mid-run,
    /// with a success status. `cargo test` then printed "running 15 tests",
    /// never printed a `test result:` line, and exited 0, so CI reported green
    /// while all 15 triodion-cli tests — including the 10 that need no RPC at
    /// all — silently never ran.
    async fn setup_source() -> Option<Source> {
        let rpc_url = crate::parse::source::parse_rpc_url(&crate::Args::default()).ok()?;
        let config = SourceConfig { requests_per_second: 15, ..SourceConfig::new(rpc_url) };
        // A configured endpoint that cannot be reached is a failure, not a skip.
        Some(Source::connect(config).await.expect("ETH_RPC_URL is set but unreachable"))
    }

    /// Skip the calling test (returning from it) when no RPC is configured.
    ///
    /// Prints the reason so a skipped run is visible in `cargo test -- --nocapture`
    /// rather than looking like a pass.
    macro_rules! source_or_skip {
        () => {
            match setup_source().await {
                Some(source) => Arc::new(source),
                None => {
                    eprintln!("skipping {}: no RPC configured (set ETH_RPC_URL)", function_name!());
                    return
                }
            }
        };
    }

    /// Name of the enclosing test, for the skip message.
    macro_rules! function_name {
        () => {{
            fn f() {}
            let name = std::any::type_name_of_val(&f);
            name.strip_suffix("::f").unwrap_or(name)
        }};
    }

    #[tokio::test]
    async fn test_extrema_timestamp_to_block_number() {
        let source = source_or_skip!();

        // Before genesis block
        assert!(timestamp_to_block_number(1438260000, source).await.unwrap() == 0);
    }

    #[tokio::test]
    async fn test_latest_timestamp_to_block_number() {
        let source = source_or_skip!();
        let latest_block_number = get_latest_block_number(source.clone()).await.unwrap();
        let latest_block = source
            .get_block(latest_block_number, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();
        let latest_timestamp = latest_block.header.timestamp;

        assert_eq!(
            timestamp_to_block_number(latest_timestamp, source).await.unwrap(),
            latest_block_number
        );
    }

    #[tokio::test]
    async fn test_timestamp_between_blocks() {
        let source = source_or_skip!();

        // Block 1000, and the timestamp surrounding block 1020
        assert!(timestamp_to_block_number(1438272177, source.clone()).await.unwrap() == 1020);
        assert!(timestamp_to_block_number(1438272178, source.clone()).await.unwrap() == 1020);

        // Timestamp 1438272176 is 1 seconds after block 1019 and 1 second before block 1020. Lower
        // block is returned
        assert!(timestamp_to_block_number(1438272176, source.clone()).await.unwrap() == 1019);

        // Timestamp 1438272187 is 1 seconds after block 1024 and 1 second before block 1025. Lower
        // block is returned
        assert!(timestamp_to_block_number(1438272187, source.clone()).await.unwrap() == 1024);

        // Timestamp 1438272169 is 4 seconds after block 1016 and 4 seconds before block 1017. Lower
        // block is returned
        assert!(timestamp_to_block_number(1438272169, source.clone()).await.unwrap() == 1016);
    }

    #[tokio::test]
    async fn test_parse_timestamp_number() {
        let source = source_or_skip!();
        let latest_timestamp =
            parse_timestamp_number("latest", RangePosition::None, source.clone()).await.unwrap();
        assert_eq!(latest_timestamp, get_latest_timestamp(source.clone()).await.unwrap());

        assert_eq!(
            parse_timestamp_number("", RangePosition::First, source.clone()).await.unwrap(),
            0
        );

        assert_eq!(
            parse_timestamp_number("", RangePosition::Last, source.clone()).await.unwrap(),
            get_latest_timestamp(source.clone()).await.unwrap()
        );

        assert_eq!(
            parse_timestamp_number("1700000000", RangePosition::None, source.clone())
                .await
                .unwrap(),
            1700000000
        );

        assert_eq!(
            parse_timestamp_number("1m", RangePosition::None, source.clone()).await.unwrap(),
            60
        );

        assert_eq!(
            parse_timestamp_number("8760h", RangePosition::None, source.clone()).await.unwrap(),
            8760 * 3600
        );

        assert_eq!(
            parse_timestamp_number("365d", RangePosition::None, source.clone()).await.unwrap(),
            365 * 86400
        );

        assert_eq!(
            parse_timestamp_number("52w", RangePosition::None, source.clone()).await.unwrap(),
            52 * 86400 * 7
        );

        assert_eq!(
            parse_timestamp_number("12M", RangePosition::None, source.clone()).await.unwrap(),
            12 * 86400 * 30
        );

        assert_eq!(
            parse_timestamp_number("1y", RangePosition::None, source.clone()).await.unwrap(),
            86400 * 365
        );
    }

    #[tokio::test]
    async fn test_parse_timestamp_range_to_block_number_range() {
        let source = source_or_skip!();

        let (start_timestamp, end_timestamp) =
            parse_timestamp_range("1700000000", "1700000015", source.clone()).await.unwrap();
        assert_eq!(
            (
                timestamp_to_block_number(start_timestamp, source.clone()).await.unwrap(),
                timestamp_to_block_number(end_timestamp, source.clone()).await.unwrap()
            ),
            (18573050, 18573051)
        );

        let (start_timestamp, end_timestamp) =
            parse_timestamp_range("-15", "1700000015", source.clone()).await.unwrap();
        assert_eq!(
            (
                timestamp_to_block_number(start_timestamp, source.clone()).await.unwrap(),
                timestamp_to_block_number(end_timestamp, source.clone()).await.unwrap()
            ),
            (18573050, 18573052)
        );

        let (start_timestamp, end_timestamp) =
            parse_timestamp_range("1700000000", "+15", source.clone()).await.unwrap();
        assert_eq!(
            (
                timestamp_to_block_number(start_timestamp, source.clone()).await.unwrap(),
                timestamp_to_block_number(end_timestamp, source.clone()).await.unwrap()
            ),
            (18573050, 18573051)
        );
    }

    /// Block `n` has timestamp `stamps[n]`. Returns the answer and the number
    /// of blocks read.
    async fn search(stamps: &[u64], timestamp: u64) -> (u64, usize) {
        let reads = std::cell::Cell::new(0);
        let latest = stamps.len() as u64 - 1;
        let block = block_at_timestamp(timestamp, latest, |number| {
            reads.set(reads.get() + 1);
            let stamp = stamps[usize::try_from(number).unwrap()];
            async move { Ok(stamp) }
        })
        .await
        .unwrap();
        (block, reads.get())
    }

    #[tokio::test]
    async fn the_search_finds_the_last_block_at_or_before_a_timestamp() {
        let stamps = [100, 112, 124, 136, 148];
        assert_eq!(search(&stamps, 124).await.0, 2, "an exact match");
        assert_eq!(search(&stamps, 130).await.0, 2, "between two blocks");
        assert_eq!(search(&stamps, 500).await.0, 4, "after the head");
    }

    #[tokio::test]
    async fn a_timestamp_before_block_zero_gives_block_zero_not_a_panic() {
        assert_eq!(search(&[100, 112, 124], 0).await.0, 0);
        assert_eq!(search(&[100], 99).await.0, 0);
    }

    #[tokio::test]
    async fn the_search_reads_each_probed_block_once() {
        // 1024 blocks need at most 11 reads. The old search read the middle
        // block twice, once before the loop and once inside it.
        let stamps: Vec<u64> = (0..1024).map(|n| 1000 + 12 * n).collect();
        let (_, reads) = search(&stamps, 1000 + 12 * 700 + 5).await;
        assert!(reads <= 11, "{reads} reads");
    }

    #[tokio::test]
    async fn an_empty_timestamp_range_is_an_error_not_an_underflow() {
        // The mock has no answers queued: the range needs no request.
        let provider = alloy::providers::ProviderBuilder::default()
            .connect_mocked_client(alloy::transports::mock::Asserter::new());
        let source =
            Arc::new(Source::from_provider(provider, 1, &SourceConfig::new(String::new())));
        assert!(parse_timestamp_range("0", "0", source).await.is_err());
    }
}
