use std::env;

use crate::args::Args;
use alloy::{
    providers::Provider,
    rpc::client::{BuiltInConnectionString, ClientBuilder, RpcClient},
    transports::layers::RetryBackoffLayer,
};
use governor::{Quota, RateLimiter};
use polars::prelude::*;
use std::num::NonZeroU32;
use triodion_core::{ParseError, Source, SourceLabels, TriodionProvider};

pub(crate) async fn parse_source(args: &Args) -> Result<Source, ParseError> {
    // parse network info
    let rpc_url = parse_rpc_url(args)?;
    let retry_layer = RetryBackoffLayer::new(
        args.max_retries,
        args.initial_backoff,
        args.compute_units_per_second,
    );
    let connect: BuiltInConnectionString = rpc_url.parse().map_err(ParseError::ProviderError)?;
    let client: RpcClient = ClientBuilder::default()
        .layer(retry_layer)
        .connect_with(connect)
        .await
        .map_err(ParseError::ProviderError)?;
    // `AnyNetwork`, not `Ethereum`: see `triodion_core::types::chains`. An
    // Ethereum-typed provider cannot deserialize a block from any OP-stack or
    // Arbitrum-stack chain.
    let provider = TriodionProvider::new(client);
    let chain_id = provider.get_chain_id().await.map_err(ParseError::ProviderError)?;
    let rate_limiter = match args.requests_per_second {
        Some(rate_limit) => match (NonZeroU32::new(1), NonZeroU32::new(rate_limit)) {
            (Some(one), Some(value)) => {
                let quota = Quota::per_second(value).allow_burst(one);
                Some(RateLimiter::direct(quota))
            }
            _ => None,
        },
        None => None,
    };

    // process concurrency info
    let max_concurrent_requests = args.max_concurrent_requests.unwrap_or(100);
    let max_concurrent_chunks = match args.max_concurrent_chunks {
        Some(0) => None,
        Some(max) => Some(max),
        None => Some(4),
    };

    // 0 means "no limit", the same convention `--max-concurrent-chunks` uses
    // just above. Building a zero-permit semaphore instead made every
    // `permit_request()` wait forever: the run printed "collecting data" and
    // hung with no output and no error.
    let semaphore = (max_concurrent_requests > 0)
        .then(|| tokio::sync::Semaphore::new(max_concurrent_requests as usize));
    let semaphore = Arc::new(semaphore);

    // Optional L1 (settlement) provider for L2-related datasets.
    //
    // Builds a separate provider — alloy's `RetryBackoffLayer` is not Clone in
    // the version we ship, so we construct a fresh layer for the L1 client.
    // L1 calls share the L2 source's semaphore + rate_limiter (see Source).
    //
    // Resolution order: `--l1-rpc <url>` flag → `L1_RPC_URL` env var → none.
    let l1_rpc_arg = args.l1_rpc.clone().or_else(|| env::var("L1_RPC_URL").ok());
    let (l1_provider, l1_chain_id, l1_rpc_url) = if let Some(url) = l1_rpc_arg {
        let l1_retry_layer = RetryBackoffLayer::new(
            args.max_retries,
            args.initial_backoff,
            args.compute_units_per_second,
        );
        let l1_connect: BuiltInConnectionString = url.parse().map_err(ParseError::ProviderError)?;
        let l1_client: RpcClient = ClientBuilder::default()
            .layer(l1_retry_layer)
            .connect_with(l1_connect)
            .await
            .map_err(ParseError::ProviderError)?;
        let l1_provider = TriodionProvider::new(l1_client);
        let l1_chain_id = l1_provider.get_chain_id().await.map_err(ParseError::ProviderError)?;
        (Some(l1_provider), Some(l1_chain_id), Some(url))
    } else {
        (None, None, None)
    };

    // Optional consensus-layer access. Only built when asked for: connecting
    // reads the node's genesis and spec, which is a round-trip no
    // execution-only run should pay.
    //
    // Resolution order for each url: flag -> env var -> none.
    let beacon_rpc = args.beacon_rpc.clone().or_else(|| env::var("BEACON_RPC_URL").ok());
    let blob_archive =
        args.blob_archive.clone().or_else(|| env::var("BLOB_ARCHIVE_URL").ok()).map(|url| {
            if url == "default" {
                triodion_core::DEFAULT_BLOB_ARCHIVE.to_string()
            } else {
                url
            }
        });
    let beacon = if beacon_rpc.is_some() || blob_archive.is_some() {
        Some(Arc::new(
            triodion_core::BeaconSource::connect(beacon_rpc, blob_archive, semaphore.clone())
                .await
                .map_err(|e| ParseError::ParseError(format!("{e}")))?,
        ))
    } else {
        None
    };

    let output = Source {
        chain_id,
        inner_request_size: args.inner_request_size,
        max_concurrent_chunks,
        semaphore,
        rate_limiter: rate_limiter.into(),
        rpc_url,
        provider,
        labels: SourceLabels {
            max_concurrent_requests: args.max_concurrent_requests,
            max_requests_per_second: args.requests_per_second.map(|x| x as u64),
            max_retries: Some(args.max_retries),
            initial_backoff: Some(args.initial_backoff),
        },
        l1_provider,
        l1_chain_id,
        l1_rpc_url,
        beacon,
    };

    Ok(output)
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

    // prepend http or https if need be
    if !url.starts_with("http") & !url.starts_with("ws") & !url.ends_with(".ipc") {
        Ok("http://".to_string() + url.as_str())
    } else {
        Ok(url)
    }
}
