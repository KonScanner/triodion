use crate::*;
use polars::prelude::*;

/// columns for consolidation_requests
///
/// One row per EIP-7251 consolidation request, from Prague onward.
///
/// A consolidation moves one validator's stake into another, so that a single
/// validator can hold more than the 32 ETH that used to be the ceiling. The
/// request is submitted from the execution layer by the address holding the
/// source validator's withdrawal credentials.
///
/// One shape of this request is not a consolidation at all. When
/// `source_pubkey` equals `target_pubkey`, the request upgrades that
/// validator's withdrawal credentials from `0x01` to `0x02` — the compounding
/// kind — and moves no stake between validators. These were the majority of
/// consolidation requests in the weeks after Prague, so a count that treats
/// every row as a merge is wrong by a wide margin. `is_credential_upgrade`
/// separates them.
///
/// Like the other request datasets this reads the consensus block, because the
/// execution layer commits to requests in `requests_hash` and does not serve
/// them.
#[triodion_macros::to_df(Datatype::ConsolidationRequests)]
#[derive(Default)]
pub struct ConsolidationRequests {
    n_rows: u64,
    block_number: Vec<u32>,
    timestamp: Vec<u32>,
    slot: Vec<u64>,
    epoch: Vec<Option<u64>>,
    proposer_index: Vec<u64>,
    // Position within this block's consolidation list.
    request_index: Vec<u32>,
    // The execution address that submitted the request, 20 bytes.
    source_address: Vec<Vec<u8>>,
    // BLS public key of the validator being consolidated away, 48 bytes.
    source_pubkey: Vec<Vec<u8>>,
    // BLS public key of the validator being consolidated into, 48 bytes.
    target_pubkey: Vec<Vec<u8>>,
    // Derived, not reported: `source_pubkey == target_pubkey`. See the note on
    // this dataset — a true here is a credential upgrade, not a merge.
    is_credential_upgrade: Vec<bool>,
    chain_id: Vec<u64>,
}

impl Dataset for ConsolidationRequests {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "timestamp",
            "slot",
            "request_index",
            "source_address",
            "source_pubkey",
            "target_pubkey",
            "is_credential_upgrade",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "request_index"])
    }
}

impl CollectByBlock for ConsolidationRequests {
    type Response = beacon_requests::BlockRequests;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        beacon_requests::fetch(request, source).await
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::ConsolidationRequests)?;
        let Some(requests) = response.requests.as_ref() else { return Ok(()) };
        for (index, consolidation) in requests.consolidations.iter().enumerate() {
            columns.n_rows += 1;
            store!(schema, columns, block_number, response.block_number);
            store!(schema, columns, timestamp, response.timestamp);
            store!(schema, columns, slot, response.slot);
            store!(schema, columns, epoch, response.epoch);
            store!(schema, columns, proposer_index, response.proposer_index);
            store!(schema, columns, request_index, index as u32);
            store!(schema, columns, source_address, consolidation.source_address.clone());
            store!(schema, columns, source_pubkey, consolidation.source_pubkey.clone());
            store!(schema, columns, target_pubkey, consolidation.target_pubkey.clone());
            store!(
                schema,
                columns,
                is_credential_upgrade,
                consolidation.source_pubkey == consolidation.target_pubkey
            );
        }
        Ok(())
    }
}

impl CollectByTransaction for ConsolidationRequests {
    type Response = ();
}
