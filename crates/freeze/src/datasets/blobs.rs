use crate::*;
use alloy::{
    consensus::Transaction as ConsensusTransaction, eips::eip2718::Encodable2718,
    rpc::types::BlockTransactionsKind,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for blobs
///
/// One row per EIP-4844 blob, joined to the L1 transaction that paid for it.
///
/// The execution layer never sees a blob — only the versioned hash committed to
/// in the transaction. The blob itself lives on the consensus layer, so this
/// dataset needs `--beacon-rpc`, and for anything older than about eighteen
/// days a `--blob-archive` too. See [`crate::types::beacon`].
#[triodion_macros::to_df(Datatype::Blobs)]
#[derive(Default)]
pub struct Blobs {
    n_rows: u64,
    block_number: Vec<u32>,
    timestamp: Vec<u32>,
    // Consensus-layer position. Derived from the block timestamp, which is
    // exact — slots are a fixed clock from genesis.
    slot: Vec<Option<u64>>,
    epoch: Vec<Option<u64>>,
    proposer_index: Vec<Option<u64>>,
    blob_index: Vec<u32>,
    // The join key in both directions: it is what the transaction commits to
    // and what the commitment hashes to.
    versioned_hash: Vec<Option<Vec<u8>>>,
    kzg_commitment: Vec<Option<Vec<u8>>>,
    kzg_proof: Vec<Option<Vec<u8>>>,
    // Resolved from the L1 block by matching `versioned_hash` against each
    // transaction's `blob_versioned_hashes`. `None` means the blob's hash did
    // not appear in any transaction in this block, which should not happen and
    // is worth seeing rather than papering over.
    transaction_hash: Vec<Option<Vec<u8>>>,
    transaction_index: Vec<Option<u64>>,
    from_address: Vec<Option<Vec<u8>>>,
    to_address: Vec<Option<Vec<u8>>>,
    // 131,072 bytes for a well-formed blob.
    blob_size: Vec<Option<u64>>,
    // Bytes used before zero padding; the gap to `blob_size` is padding the
    // poster paid for. Archive-sourced rows only.
    blob_used_size: Vec<Option<u64>>,
    // Which rollup posted the blob, when the archive could attribute it.
    rollup: Vec<Option<String>>,
    // The blob itself. Opt-in and never a default column: 128 KiB per row.
    // Only a beacon node serves the bytes; archive rows leave this null.
    blob: Vec<Option<Vec<u8>>>,
    // "beacon_node" or "archive". A blob-free block and a block nobody would
    // answer for both produce no rows, so this says who was asked when rows
    // *do* appear.
    blob_source: Vec<String>,
    chain_id: Vec<u64>,
}

impl Dataset for Blobs {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "timestamp",
            "slot",
            "blob_index",
            "versioned_hash",
            "kzg_commitment",
            "transaction_hash",
            "from_address",
            "to_address",
            "blob_size",
            "rollup",
            "blob_source",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "blob_index"])
    }
}

/// A block, its derived slot and epoch, and the blobs published in it.
///
/// Slot and epoch are computed in `extract`, where the chain's own slot clock
/// is in hand. `None` for an archive-only run, which has no beacon node to read
/// a clock from — the archive reports each blob's slot itself.
pub type BlockAndBlobs = (RpcBlock, Option<u64>, Option<u64>, Vec<BlobRecord>);

impl CollectByBlock for Blobs {
    type Response = BlockAndBlobs;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let beacon = source.beacon.as_ref().ok_or_else(|| {
            err("the blobs dataset needs --beacon-rpc (and --blob-archive for older blocks)")
        })?;

        // Full bodies, not hashes: the blob->transaction join is done here,
        // against this block's `blob_versioned_hashes`.
        let block = source
            .get_block(request.block_number()?, BlockTransactionsKind::Full)
            .await?
            .ok_or_else(|| err("block not found"))?;

        let slot = beacon.config.as_ref().and_then(|c| c.slot_at_timestamp(block.header.timestamp));
        let epoch = slot.zip(beacon.config.as_ref()).map(|(slot, c)| c.epoch_of_slot(slot));
        let blobs = beacon.blobs_for_block(block.header.number, slot).await?;
        Ok((block, slot, epoch, blobs))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Blobs)?;
        let (block, slot, epoch, blobs) = response;
        let carriers = blob_carriers(&block);
        let timestamp = block.header.timestamp as u32;
        let block_number = block.header.number as u32;

        for blob in blobs {
            let carrier = blob
                .versioned_hash
                .as_ref()
                .and_then(|hash| carriers.get(hash.as_slice()))
                .cloned();
            columns.n_rows += 1;
            store!(schema, columns, block_number, block_number);
            store!(schema, columns, timestamp, timestamp);
            store!(schema, columns, slot, blob.slot.or(slot));
            store!(schema, columns, epoch, epoch);
            store!(schema, columns, proposer_index, blob.proposer_index);
            store!(schema, columns, blob_index, blob.index as u32);
            store!(schema, columns, versioned_hash, blob.versioned_hash);
            store!(schema, columns, kzg_commitment, blob.kzg_commitment);
            store!(schema, columns, kzg_proof, blob.kzg_proof);
            store!(schema, columns, transaction_hash, carrier.as_ref().map(|c| c.hash.clone()));
            store!(schema, columns, transaction_index, carrier.as_ref().and_then(|c| c.index));
            store!(schema, columns, from_address, carrier.as_ref().map(|c| c.from.clone()));
            store!(schema, columns, to_address, carrier.as_ref().and_then(|c| c.to.clone()));
            store!(schema, columns, blob_size, blob.size);
            store!(schema, columns, blob_used_size, blob.used_size);
            store!(schema, columns, rollup, blob.rollup);
            store!(schema, columns, blob, blob.blob);
            store!(schema, columns, blob_source, blob.provenance.as_str().to_string());
        }
        Ok(())
    }
}

impl CollectByTransaction for Blobs {
    type Response = ();
}

/// The transaction that committed to a blob.
#[derive(Clone, Debug)]
struct BlobCarrier {
    hash: Vec<u8>,
    index: Option<u64>,
    from: Vec<u8>,
    to: Option<Vec<u8>>,
}

/// Map every versioned hash in a block to the transaction that carries it.
///
/// A blob sidecar does not name its transaction; the link only exists in the
/// execution-layer transaction's `blob_versioned_hashes`. Building the map here
/// is what makes the dataset joinable to `transactions` without a second pass.
fn blob_carriers(block: &RpcBlock) -> HashMap<Vec<u8>, BlobCarrier> {
    let mut carriers = HashMap::new();
    let Some(transactions) = block.transactions.as_transactions() else { return carriers };
    for tx in transactions {
        let envelope = tx.inner.inner.inner();
        let Some(hashes) = envelope.blob_versioned_hashes() else { continue };
        let carrier = BlobCarrier {
            hash: envelope.trie_hash().to_vec(),
            index: tx.inner.transaction_index,
            from: tx.inner.inner.signer().to_vec(),
            to: match envelope.kind() {
                alloy::primitives::TxKind::Create => None,
                alloy::primitives::TxKind::Call(address) => Some(address.to_vec()),
            },
        };
        for hash in hashes {
            carriers.insert(hash.to_vec(), carrier.clone());
        }
    }
    carriers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::wire_fixtures::mainnet_blob_block;

    #[test]
    fn every_versioned_hash_in_a_block_maps_back_to_its_transaction() {
        // Mainnet block 20,000,000 carries one type-0x03 transaction with one
        // blob. Its versioned hash is what the beacon sidecar's commitment
        // hashes to, so this map is the whole join.
        let block: RpcBlock =
            serde_json::from_value(mainnet_blob_block()).expect("fixture deserializes");
        let carriers = blob_carriers(&block);
        assert_eq!(carriers.len(), 1);
        let hash =
            alloy::hex::decode("017ba4bd9c166498865a3d08618e333ee84812941b5c3a356971b4a6ffffa574")
                .unwrap();
        let carrier = carriers.get(&hash).expect("the blob's hash is claimed by a transaction");
        assert_eq!(
            alloy::hex::encode(&carrier.hash),
            "0ff07f37baa7fa26bb7de3d3fc63002bf0acf3295bdab7f67c108c0d1a3bff15"
        );
        assert_eq!(carrier.index, Some(21));
    }

    #[test]
    fn a_block_with_no_blob_transactions_produces_an_empty_map_not_a_panic() {
        let block: RpcBlock =
            serde_json::from_value(crate::types::wire_fixtures::op_mainnet_block())
                .expect("fixture deserializes");
        assert!(blob_carriers(&block).is_empty());
    }
}
