use std::sync::Arc;

use triodion_core::{ExecutionEnv, FileOutput, ParseError, Query, Source};

use crate::args::Args;

use super::{execution, file_output, query, source};

/// parse options for running freeze
pub async fn parse_args(
    args: &Args,
) -> Result<(Query, Source, FileOutput, ExecutionEnv), ParseError> {
    let source = source::parse_source(args).await?;
    let query = query::parse_query(args, Arc::new(source.clone())).await?;
    let sink = file_output::parse_file_output(args, &source)?;
    let env = execution::parse_execution_env(args, query.n_tasks() as u64)?;
    Ok((query, source, sink, env))
}

/// parse command string
///
/// Splits on whitespace, so a value containing a space cannot be expressed —
/// quote it and both halves become separate tokens. Callers that need one are
/// building an [`Args`] directly.
///
/// # Errors
///
/// Returns [`ParseError::ParseError`] when the command does not parse. This
/// goes through the fallible `try_parse_from_cli`, not `parse_from_cli`, on
/// purpose: the infallible twin calls `std::process::exit` on a bad argv, and
/// this function is reached from the Python extension module, where that would
/// kill the interpreter with no traceback.
#[allow(dead_code)]
pub async fn parse_str(command: &str) -> Result<Args, ParseError> {
    Args::try_parse_from_cli(command.split_whitespace())
        .map_err(|e| ParseError::ParseError(e.to_string()))
}
