use std::collections::HashMap;
use triodion_core::ParseError;

/// Convert a non-negative, finite `f64` to `u64`, or say why it cannot be.
///
/// There is no `TryFrom<f64> for u64`, and `as` does not fail on a value the
/// target cannot hold -- it saturates. That silently mis-parsed user input:
/// `--blocks -5B` became block 0 rather than an error, and `1e30B` became
/// `u64::MAX`. Checking the range first makes the conversion total, so a bad
/// block or timestamp reference is reported instead of quietly substituted.
pub(crate) fn f64_to_u64(value: f64, context: &str) -> Result<u64, ParseError> {
    // `u64::MAX as f64` rounds UP to 2^64, so compare with `<` to keep the
    // cast below strictly in range.
    if !value.is_finite() || value < 0.0 || value >= u64::MAX as f64 {
        return Err(ParseError::ParseError(format!("{context} out of range: {value}")));
    }
    // Checked directly above: finite, non-negative, and below `u64::MAX`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

pub(crate) fn hex_string_to_binary(hex_string: &str) -> Result<Vec<u8>, ParseError> {
    let hex_string = hex_string.strip_prefix("0x").unwrap_or(hex_string);
    hex::decode(hex_string)
        .map_err(|_| ParseError::ParseError("could not parse data as hex".to_string()))
}

pub(crate) fn hex_strings_to_binary(hex_strings: &[String]) -> Result<Vec<Vec<u8>>, ParseError> {
    hex_strings
        .iter()
        .map(|x| {
            hex::decode(x.strip_prefix("0x").unwrap_or(x))
                .map_err(|_| ParseError::ParseError("could not parse data as hex".to_string()))
        })
        .collect::<Result<Vec<_>, _>>()
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub(crate) enum BinaryInputList {
    Explicit,
    ParquetColumn(String, String),
}

use std::path::Path;

impl BinaryInputList {
    /// convert to label
    pub(crate) fn to_label(&self) -> Option<String> {
        match self {
            BinaryInputList::Explicit => None,
            BinaryInputList::ParquetColumn(path, _) => Path::new(&path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem_str| stem_str.split("__").last().unwrap_or(stem_str))
                .map(|s| s.to_string()),
        }
    }
}

type ParsedBinaryArg = HashMap<BinaryInputList, Vec<Vec<u8>>>;

/// parse binary argument list
/// each argument can be a hex string or a parquet column reference
/// each parquet column is loaded into its own list, hex strings loaded into another
pub(crate) fn parse_binary_arg(
    inputs: &[String],
    default_column: &str,
) -> Result<ParsedBinaryArg, ParseError> {
    let mut parsed = HashMap::new();

    // separate into files vs explicit
    let (files, hex_strings): (Vec<&String>, Vec<&String>) = inputs.iter().partition(|tx| {
        // strip off column name if present
        match parse_file_column_reference(tx, default_column) {
            Ok(reference) => std::path::Path::new(&reference.path).exists(),
            _ => false,
        }
    });

    // files columns
    for path in files {
        let reference = parse_file_column_reference(path, default_column)?;
        let values = triodion_core::read_binary_column(&reference.path, &reference.column)
            .map_err(|_e| ParseError::ParseError("could not read input".to_string()))?;
        let key = BinaryInputList::ParquetColumn(reference.path, reference.column);
        parsed.insert(key, values);
    }

    // explicit binary strings
    if !hex_strings.is_empty() {
        let hex_strings: Vec<String> = hex_strings.into_iter().cloned().collect();
        let binary_vec = hex_strings_to_binary(&hex_strings)?;
        parsed.insert(BinaryInputList::Explicit, binary_vec);
    };

    Ok(parsed)
}

struct FileColumnReference {
    path: String,
    column: String,
}

fn parse_file_column_reference(
    path: &str,
    default_column: &str,
) -> Result<FileColumnReference, ParseError> {
    let (path, column) = if path.contains(':') {
        let pieces: Vec<&str> = path.split(':').collect();
        if pieces.len() == 2 {
            (pieces[0], pieces[1])
        } else {
            return Err(ParseError::ParseError("could not parse path column".to_string()));
        }
    } else {
        (path, default_column)
    };

    let parsed = FileColumnReference { path: path.to_string(), column: column.to_string() };

    Ok(parsed)
}

#[cfg(test)]
mod f64_to_u64_tests {
    use super::f64_to_u64;

    #[test]
    fn accepts_values_inside_the_range() {
        assert_eq!(f64_to_u64(0.0, "block ref").unwrap(), 0);
        assert_eq!(f64_to_u64(1.5e9, "block ref").unwrap(), 1_500_000_000);
    }

    #[test]
    fn rejects_a_negative_value_instead_of_reading_it_as_zero() {
        // `-5e9 as u64` saturates to 0, so `--blocks -5B` used to parse as
        // block 0 rather than being reported as bad input.
        assert!(f64_to_u64(-5e9, "block ref").is_err());
    }

    #[test]
    fn rejects_a_value_too_large_for_u64() {
        // `1e30 as u64` saturates to `u64::MAX`.
        assert!(f64_to_u64(1e30, "block ref").is_err());
    }

    #[test]
    fn rejects_values_that_are_not_finite() {
        assert!(f64_to_u64(f64::NAN, "block ref").is_err());
        assert!(f64_to_u64(f64::INFINITY, "block ref").is_err());
    }
}
