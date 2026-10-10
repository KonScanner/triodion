use pyo3::{exceptions::PyTypeError, prelude::*, types::PyDict};
use triodion_cli::Args;

/// Build the `Args` of a run from the keyword arguments of a Python call.
///
/// Every keyword names an `Args` field. A field that is not named keeps the
/// default of the CLI, so a Python run and a `triodion` run start from the
/// same defaults, and a new CLI flag reaches Python with no change here.
///
/// The values travel as JSON, because that is the form `Args` already
/// deserializes from; each one is a string, a number, a bool, a list or
/// `None`.
///
/// # Errors
///
/// Raises `TypeError` for a keyword that names no field and for a value of
/// the wrong type, as a Python function with a fixed signature does.
pub(crate) fn args_from_kwargs(kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Args> {
    let overrides = match kwargs {
        Some(kwargs) => {
            let json: String =
                kwargs.py().import("json")?.call_method1("dumps", (kwargs,))?.extract()?;
            serde_json::from_str(&json).map_err(|e| PyTypeError::new_err(e.to_string()))?
        }
        None => serde_json::Map::new(),
    };
    Args::from_overrides(overrides).map_err(|e| PyTypeError::new_err(e.to_string()))
}
