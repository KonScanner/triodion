//! The universal batching floor: many identical JSON-RPC calls per HTTP request.
//!
//! [State overrides](crate::types::state_override) are the fastest way to read
//! bulk state, and [Multicall3](crate::types::multicall) the fastest way to
//! batch contract calls — but both ask something of the node. Overrides need an
//! endpoint that honours the third `eth_call` parameter; Multicall3 needs the
//! aggregator deployed at the block being read. Plenty of endpoints offer
//! neither, and some methods have no override path at all no matter how
//! cooperative the node is.
//!
//! `eth_getTransactionCount` is the clean example. There is no `NONCE` opcode:
//! the EVM exposes an account's balance (`BALANCE`), its code (`EXTCODECOPY`,
//! `EXTCODESIZE`, `EXTCODEHASH`) and its storage (`SLOAD` from inside it), but
//! nothing reads another account's nonce. No injected bytecode can batch nonce
//! reads, on any node, ever. The only lever left is the transport.
//!
//! JSON-RPC has always allowed an array of requests in one HTTP body, and
//! [`Source::send_batch`] already implements it with the size negotiation real
//! providers demand (OP Mainnet answers `413` above ten calls; Base returns
//! `-32014 "maximum 10 calls in 1 batch"`). It needs nothing from the node
//! beyond JSON-RPC itself, so it is the rung every dataset can stand on. Dedaub
//! measured batched `eth_getStorageAt` at 57× sequential — most of the win,
//! with none of the requirements.
//!
//! This module turns that into an opt-in for `CollectByBlock` datasets, the
//! same shape [`MulticallBatchable`](crate::types::multicall::MulticallBatchable)
//! and [`StateOverrideBatchable`](crate::types::state_override::StateOverrideBatchable)
//! have: say what one row's params look like on the wire and how to read one
//! result back, and [`rpc_batch_collect_by_block`] does the rest.

use crate::{CollectByBlock, CollectError, Datatype, Params, Partition, Query, Source};
use polars::prelude::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;

type R<T> = ::core::result::Result<T, CollectError>;

/// Rows per HTTP request before the runner starts a second one.
///
/// [`Source::send_batch`] negotiates downward on its own when a provider
/// refuses a batch this size, so this is a starting point rather than a limit.
/// It matches the batch size the receipt and block helpers already use, which
/// keeps one number to reason about across the tool.
pub const DEFAULT_RPC_BATCH_ROWS: usize = 100;

/// Trait for `CollectByBlock` datasets whose per-row RPC can be batched at the
/// transport level.
///
/// Every row must map to the *same* JSON-RPC method — that is what lets the
/// calls share one HTTP body — differing only in parameters.
pub trait RpcBatchable: CollectByBlock {
    /// The parameter tuple for one row, serialized as this method's `params`.
    ///
    /// Typically `(Address, BlockNumberOrTag)`. A tuple serializes to a JSON
    /// array, which is the shape JSON-RPC wants.
    type Param: alloy::rpc::json_rpc::RpcSend + Send + Sync + 'static;

    /// The result type for one row, deserialized from this method's `result`.
    type Item: alloy::rpc::json_rpc::RpcRecv + Send + 'static;

    /// The JSON-RPC method every row in a batch invokes.
    fn method() -> &'static str;

    /// Build one row's parameters.
    ///
    /// # Errors
    /// Returns `Err` when the row is missing a required parameter; the runner
    /// routes that row to the per-call path rather than failing the partition.
    fn param_for_row(params: &Params) -> R<Self::Param>;

    /// Turn one row's result into the dataset's response.
    ///
    /// # Errors
    /// Returns `Err` only for unrecoverable decoding bugs.
    fn decode_row(params: &Params, item: Self::Item) -> R<Self::Response>;

    /// Rows per HTTP request for this dataset. Override when responses are
    /// large enough that a hundred of them is an unwieldy body — `eth_getCode`
    /// against a partition of large contracts, say.
    fn default_rpc_batch_rows() -> usize {
        DEFAULT_RPC_BATCH_ROWS
    }
}

/// JSON-RPC-batched collection for `D: RpcBatchable`.
///
/// Chunks the partition's rows, sends each chunk as one HTTP request, and
/// spawns the chunks concurrently so the node sees the same parallelism it does
/// today with a fraction of the requests. A chunk that fails falls back to
/// [`CollectByBlock::extract`] per row, so a provider that rejects batching
/// outright still produces the same output the per-row path would.
///
/// Unlike the state-override runner there is no grouping by block or contract:
/// every row carries its own parameters, including its own block, so any rows
/// may share a request.
///
/// # Errors
/// Returns `Err` only for unrecoverable conditions — mpsc send failure, a
/// dataset `transform` failure, or a per-row fallback that itself failed.
pub async fn rpc_batch_collect_by_block<D>(
    partition: Partition,
    source: Arc<Source>,
    query: Arc<Query>,
    inner_request_size: Option<u64>,
) -> R<HashMap<Datatype, DataFrame>>
where
    D: RpcBatchable + Send + Sync + 'static,
    D::Response: Send + 'static,
{
    let (sender, receiver) = mpsc::channel(1);
    let chain_id = source.chain_id;
    let rows_per_request = D::default_rpc_batch_rows().max(1);

    let all_rows = partition.param_sets(inner_request_size)?;
    let mut handles = Vec::new();

    for chunk in all_rows.chunks(rows_per_request) {
        let chunk = chunk.to_vec();
        let sender = sender.clone();
        let source = source.clone();
        let query = query.clone();
        let handle = tokio::task::spawn(async move {
            let responses = batch_with_fallback::<D>(chunk, &source, query).await?;
            for resp in responses {
                sender
                    .send(Ok(resp))
                    .await
                    .map_err(|_| CollectError::CollectError("mpsc send failed".to_string()))?;
            }
            Ok::<(), CollectError>(())
        });
        handles.push(handle);
    }

    drop(sender);

    let columns = <D as CollectByBlock>::transform_channel(receiver, &query).await?;
    crate::collect_generic::join_partition_handles(handles).await?;
    columns.create_dfs(&query.schemas, chain_id)
}

/// Send one chunk as a batch, falling back to per-row extraction if it fails.
///
/// There is deliberately no halving here — [`Source::send_batch`] already does
/// that, and it does it better, because it learns the provider's ceiling once
/// and keeps it for the rest of the call. A failure that reaches this function
/// has already survived that negotiation, so it is not about size, and the only
/// useful response left is to stop batching these rows.
async fn batch_with_fallback<D>(
    rows: Vec<Params>,
    source: &Arc<Source>,
    query: Arc<Query>,
) -> R<Vec<D::Response>>
where
    D: RpcBatchable,
{
    // A row missing a parameter cannot be encoded, so the whole chunk goes to
    // the per-row path where the dataset's own accessor reports which field is
    // absent — a better error than "batch encoding failed".
    let params: Result<Vec<D::Param>, CollectError> = rows.iter().map(D::param_for_row).collect();

    if let Ok(params) = params {
        match source.send_batch::<D::Param, D::Item>(D::method(), &params).await {
            Ok(items) if items.len() == rows.len() => {
                return rows.iter().zip(items).map(|(p, i)| D::decode_row(p, i)).collect()
            }
            // A short or long result array means the responses cannot be
            // matched to their rows. Falling back re-reads them one at a time
            // rather than pairing them up wrongly.
            Ok(_) | Err(_) => {}
        }
    }

    // Every row goes in flight at once rather than being awaited in sequence,
    // matching `state_override::per_row`. `Source`'s semaphore is what caps the
    // concurrency either way, so demoting a batch costs requests, not
    // serialisation. A sequential loop here would make an endpoint that refuses
    // batching *slower than before batching existed*: `codes` batches 50 rows
    // and `nonces` 100, so the run's in-flight requests would drop from one per
    // row to one per chunk.
    futures::future::try_join_all(
        rows.into_iter().map(|p| D::extract(p, source.clone(), query.clone())),
    )
    .await
}
