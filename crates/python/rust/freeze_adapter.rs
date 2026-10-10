use pyo3::{
    IntoPyObjectExt,
    exceptions::{PyRuntimeError, PyTypeError},
    prelude::*,
    types::{IntoPyDict, PyDict},
};

use crate::kwargs::args_from_kwargs;
use triodion_cli::{Args, run};

#[pyfunction(signature = (datatype = None, blocks = None, *, command = None, **kwargs))]
pub fn _freeze<'py>(
    py: Python<'py>,
    datatype: Option<Vec<String>>,
    blocks: Option<Vec<String>>,
    command: Option<String>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    if let Some(command) = command {
        freeze_command(py, command)
    } else if let Some(datatype) = datatype {
        let args = Args { datatype, blocks, ..args_from_kwargs(kwargs)? };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match run(args).await {
                Ok(Some(result)) => Python::attach(|py| {
                    let dict = [
                        ("n_completed", result.completed.len().into_py_any(py)?),
                        ("n_skipped", result.skipped.len().into_py_any(py)?),
                        ("n_errored", result.errored.len().into_py_any(py)?),
                    ]
                    .into_py_dict(py)?;
                    Ok::<Py<PyAny>, PyErr>(dict.into_any().unbind())
                }),
                Ok(None) => Ok(Python::attach(|py| py.None())),
                Err(e) => Err(PyErr::new::<PyRuntimeError, _>(format!("{e}"))),
            }
        })
    } else {
        Err(PyErr::new::<PyTypeError, _>("must specify datatypes or command"))
    }
}

fn freeze_command(py: Python<'_>, command: String) -> PyResult<Bound<'_, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        // `.expect` here panicked across the pyo3 boundary, so a bad command
        // string reached Python as `RustPanic: unknown error` rather than as
        // the parse error explaining what was wrong with it.
        let args = triodion_cli::parse_str(command.as_str())
            .await
            .map_err(|e| PyErr::new::<PyRuntimeError, _>(format!("could not parse inputs: {e}")))?;
        match run(args).await {
            Ok(Some(result)) => Python::attach(|py| {
                let dict = [
                    ("n_completed", result.completed.len().into_py_any(py)?),
                    ("n_skipped", result.skipped.len().into_py_any(py)?),
                    ("n_errored", result.errored.len().into_py_any(py)?),
                ]
                .into_py_dict(py)?;
                Ok::<Py<PyAny>, PyErr>(dict.into_any().unbind())
            }),
            Ok(None) => Ok(Python::attach(|py| py.None())),
            Err(e) => Err(PyErr::new::<PyRuntimeError, _>(format!("{e}"))),
        }
    })
}
