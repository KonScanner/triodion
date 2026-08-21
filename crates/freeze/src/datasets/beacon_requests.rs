//! Shared fetch path for the three EIP-7685 request datasets.
//!
//! `deposit_requests`, `withdrawal_requests` and `consolidation_requests` are
//! three shapes of the same journey: resolve the execution block, derive its
//! slot, and ask the consensus layer for that slot's requests. Only the
//! exploding differs, so only the exploding lives in the dataset modules.

use crate::{beacon::ExecutionRequests, *};
use alloy::rpc::types::BlockTransactionsKind;
use std::sync::Arc;

/// One execution block, its consensus position, and the requests it published.
#[derive(Clone, Debug)]
pub struct BlockRequests {
    /// Execution block number.
    pub block_number: u32,
    /// Execution block timestamp.
    pub timestamp: u32,
    /// Consensus slot, derived from the timestamp and confirmed against the
    /// block number the beacon block itself reports.
    pub slot: u64,
    /// Consensus epoch. `None` only if the node reported no slots per epoch.
    pub epoch: Option<u64>,
    /// Validator index of the slot's proposer.
    pub proposer_index: u64,
    /// The requests. Empty lists when the slot carried none; the whole value
    /// is absent before Electra, which the caller turns into zero rows.
    pub requests: Option<ExecutionRequests>,
}

/// Resolve one execution block's EIP-7685 requests.
///
/// # Errors
/// No beacon source configured; the block or its slot not found; or a slot
/// that reports a different execution block than the one asked about.
pub async fn fetch(request: Params, source: Arc<Source>) -> R<BlockRequests> {
    let beacon = source.beacon.as_ref().ok_or_else(|| {
        err("execution requests need --beacon-rpc: the execution layer commits to them in requests_hash but does not serve them")
    })?;
    let config = beacon.config.as_ref().ok_or_else(|| {
        err("execution requests need --beacon-rpc for the slot clock; --blob-archive alone cannot answer")
    })?;

    let block = source
        .get_block(request.block_number()?, BlockTransactionsKind::Hashes)
        .await?
        .ok_or_else(|| err("block not found"))?;

    let timestamp = block.header.timestamp;
    let slot = config.slot_at_timestamp(timestamp).ok_or_else(|| {
        err("block predates beacon genesis, so it has no slot and no execution requests")
    })?;
    let found = beacon.execution_requests_for_block(block.header.number, slot).await?;

    Ok(BlockRequests {
        block_number: block.header.number as u32,
        timestamp: timestamp as u32,
        slot,
        epoch: Some(config.epoch_of_slot(slot)),
        // A slot with no requests still has a proposer; a pre-Electra slot is
        // reported by the caller as zero rows, so the fallback never surfaces.
        proposer_index: found.as_ref().map(|f| f.proposer_index).unwrap_or_default(),
        requests: found.and_then(|f| f.requests),
    })
}
