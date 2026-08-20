use crate::ParseError;
use polars::prelude::*;

/// read single binary column of parquet file as Vec<u8>
pub fn read_binary_column(path: &str, column: &str) -> Result<Vec<Vec<u8>>, ParseError> {
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

    let ca = series
        .binary()
        .map_err(|_e| ParseError::ParseError("could not convert to binary column".to_string()))?;

    ca.iter()
        .map(|value| {
            value
                .ok_or_else(|| ParseError::ParseError("transaction hash missing".to_string()))
                .map(|data| data.into())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip a binary column through parquet. This covers the
    /// `ChunkedArray` iteration in `read_binary_column`, which polars 0.55
    /// changed: `&ChunkedArray<BinaryType>` no longer implements `IntoIterator`.
    #[test]
    fn read_binary_column_round_trips_parquet() {
        let path = std::env::temp_dir().join("triodion_read_binary_column_test.parquet");
        let values: Vec<&[u8]> = vec![&[0x01, 0x02], &[0x03, 0x04]];
        let column = Column::new("hash".into(), values);
        let mut df = DataFrame::new(2, vec![column]).unwrap();

        let file = std::fs::File::create(&path).unwrap();
        ParquetWriter::new(file).finish(&mut df).unwrap();

        let mut read = read_binary_column(path.to_str().unwrap(), "hash").unwrap();
        std::fs::remove_file(&path).unwrap();

        read.sort();
        assert_eq!(read, vec![vec![0x01, 0x02], vec![0x03, 0x04]]);
    }
}
