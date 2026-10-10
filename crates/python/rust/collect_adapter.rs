use polars::prelude::*;
use pyo3::{
    exceptions::{PyRuntimeError, PyTypeError},
    prelude::*,
    types::PyDict,
};
use pyo3_polars::PyDataFrame;

use crate::kwargs::args_from_kwargs;
use triodion_cli::{Args, parse_args};
use triodion_core::collect;

#[pyfunction(signature = (datatype = None, blocks = None, *, command = None, **kwargs))]
pub fn _collect<'py>(
    py: Python<'py>,
    datatype: Option<String>,
    blocks: Option<Vec<String>>,
    command: Option<String>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    if let Some(command) = command {
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match run_execute(command).await {
                Ok(df) => Ok(PyDataFrame(df)),
                Err(e) => Err(PyErr::new::<PyRuntimeError, _>(format!("{e}"))),
            }
        })
    } else if let Some(datatype) = datatype {
        let args = Args { datatype: vec![datatype], blocks, ..args_from_kwargs(kwargs)? };
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match run_collect(args).await {
                Ok(df) => Ok(PyDataFrame(df)),
                Err(e) => Err(PyErr::new::<PyRuntimeError, _>(format!("{e}"))),
            }
        })
    } else {
        Err(PyErr::new::<PyTypeError, _>("must specify datatype or command"))
    }
}

async fn run_collect(args: Args) -> PolarsResult<DataFrame> {
    // Return the error, do not panic. This is a library boundary into Python:
    // a panic here crosses into pyo3 as an opaque
    // `RustPanic: rust future panicked: unknown error`, which stripped the
    // message the caller actually needs. Omitting `--rpc` reported
    // "unknown error" instead of
    // "must provide --rpc or setup MESC or set ETH_RPC_URL".
    // The `_collect` wrapper above already turns an `Err` into a
    // `PyRuntimeError` carrying the text.
    let (query, source, _sink, _env) = parse_args(&args)
        .await
        .map_err(|e| PolarsError::ComputeError(format!("error parsing opts: {e}").into()))?;
    collect(query.into(), source.into())
        .await
        .map_err(|e| PolarsError::ComputeError(format!("error collecting: {e}").into()))
}

async fn run_execute(command: String) -> PolarsResult<DataFrame> {
    let args = triodion_cli::parse_str(command.as_str())
        .await
        .map_err(|e| PolarsError::ComputeError(format!("error parsing opts: {e}").into()))?;
    run_collect(args).await
}
