use std::env;

use crate::args::Args;
use triodion_core::{CollectError, ParseError, Source, SourceConfig};

pub(crate) async fn parse_source(args: &Args) -> Result<Source, ParseError> {
    Source::connect(source_config(args)?).await.map_err(|e| match e {
        CollectError::ProviderError(e) => ParseError::ProviderError(e),
        CollectError::ParseError(e) => e,
        CollectError::CollectError(message) | CollectError::RPCError(message) => {
            ParseError::ParseError(message)
        }
        e => ParseError::ParseError(e.to_string()),
    })
}

/// Resolve the endpoints and the request limits of a run.
///
/// Each endpoint resolves in this order: flag, then environment variable, then
/// none. A limit that the user did not give takes the default of
/// [`SourceConfig::new`].
fn source_config(args: &Args) -> Result<SourceConfig, ParseError> {
    let defaults = SourceConfig::new(parse_rpc_url(args)?);
    let blob_archive_url =
        args.blob_archive.clone().or_else(|| env::var("BLOB_ARCHIVE_URL").ok()).map(|url| {
            if url == "default" { triodion_core::DEFAULT_BLOB_ARCHIVE.to_string() } else { url }
        });
    Ok(SourceConfig {
        l1_rpc_url: args.l1_rpc.clone().or_else(|| env::var("L1_RPC_URL").ok()),
        beacon_rpc_url: args.beacon_rpc.clone().or_else(|| env::var("BEACON_RPC_URL").ok()),
        blob_archive_url,
        max_retries: args.max_retries,
        initial_backoff: args.initial_backoff,
        compute_units_per_second: args.compute_units_per_second,
        max_concurrent_requests: args
            .max_concurrent_requests
            .unwrap_or(defaults.max_concurrent_requests),
        max_concurrent_chunks: args.max_concurrent_chunks.unwrap_or(defaults.max_concurrent_chunks),
        requests_per_second: args.requests_per_second.map_or(0, u64::from),
        inner_request_size: args.inner_request_size,
        ..defaults
    })
}

pub(crate) fn parse_rpc_url(args: &Args) -> Result<String, ParseError> {
    // get MESC url
    let mesc_url = if mesc::is_mesc_enabled() {
        let endpoint = match &args.rpc {
            Some(url) => mesc::get_endpoint_by_query(url, Some("triodion")),
            None => mesc::get_default_endpoint(Some("triodion")),
        };
        match endpoint {
            Ok(endpoint) => endpoint.map(|endpoint| endpoint.url),
            Err(e) => {
                eprintln!("Could not load MESC data: {}", e);
                None
            }
        }
    } else {
        None
    };

    // use ETH_RPC_URL if no MESC url found
    let url = if let Some(url) = mesc_url {
        url
    } else if let Some(url) = &args.rpc {
        url.clone()
    } else if let Ok(url) = env::var("ETH_RPC_URL") {
        url
    } else {
        let message = "must provide --rpc or setup MESC or set ETH_RPC_URL";
        return Err(ParseError::ParseError(message.to_string()));
    };

    Ok(triodion_core::with_default_scheme(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(flags: &[&str]) -> SourceConfig {
        let argv = ["triodion", "blocks", "--rpc", "http://localhost:8545"].iter().chain(flags);
        let args = Args::try_parse_from_cli(argv).expect("the argv parses");
        source_config(&args).expect("the rpc url resolves")
    }

    #[test]
    fn a_run_with_no_limit_flags_uses_the_source_defaults() {
        let config = config(&[]);
        // The endpoints can come from the environment, so only the limits
        // are compared.
        let expected = SourceConfig {
            rpc_url: config.rpc_url.clone(),
            l1_rpc_url: config.l1_rpc_url.clone(),
            beacon_rpc_url: config.beacon_rpc_url.clone(),
            blob_archive_url: config.blob_archive_url.clone(),
            ..SourceConfig::new(String::new())
        };
        assert_eq!(config, expected);
    }

    #[test]
    fn the_limit_flags_reach_the_config() {
        let config = config(&[
            "--max-concurrent-requests",
            "0",
            "--max-concurrent-chunks",
            "0",
            "--requests-per-second",
            "7",
            "--max-retries",
            "2",
            "--inner-request-size",
            "50",
        ]);
        assert_eq!(config.max_concurrent_requests, 0);
        assert_eq!(config.max_concurrent_chunks, 0);
        assert_eq!(config.requests_per_second, 7);
        assert_eq!(config.max_retries, 2);
        assert_eq!(config.inner_request_size, 50);
    }

    #[test]
    fn an_rpc_url_with_no_scheme_gets_http_and_ws_or_ipc_keep_theirs() {
        assert_eq!(
            triodion_core::with_default_scheme("localhost:8545".into()),
            "http://localhost:8545"
        );
        for url in ["https://x.io", "ws://x.io", "wss://x.io", "/tmp/geth.ipc"] {
            assert_eq!(triodion_core::with_default_scheme(url.into()), url);
        }
    }
}
