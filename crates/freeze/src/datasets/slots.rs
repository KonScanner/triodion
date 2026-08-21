use crate::*;
use alloy::primitives::{Address, B256, U256};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for balances
#[triodion_macros::to_df(Datatype::Slots)]
#[derive(Default)]
pub struct Slots {
    n_rows: usize,
    block_number: Vec<u32>,
    address: Vec<Vec<u8>>,
    slot: Vec<Vec<u8>>,
    value: Vec<Vec<u8>>,
    chain_id: Vec<u64>,
}

impl Dataset for Slots {
    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "address", "slot"])
    }

    fn required_parameters() -> Vec<Dim> {
        vec![Dim::Address, Dim::Slot]
    }

    fn aliases() -> Vec<&'static str> {
        vec!["storages"]
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }

    fn default_blocks() -> Option<String> {
        Some("latest".to_string())
    }
}

type BlockTxAddressOutput = (u32, Option<Vec<u8>>, Vec<u8>, Vec<u8>, Vec<u8>);

impl CollectByBlock for Slots {
    type Response = BlockTxAddressOutput;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let address = request.address()?;
        let block_number = request.block_number()? as u32;
        let slot = request.slot()?;
        let output = source
            .get_storage_at(
                Address::from_slice(&address),
                U256::from_be_slice(&slot),
                block_number.into(),
            )
            .await?;
        Ok((block_number, None, address, slot, output.to_vec_u8()))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Slots)?;
        process_slot(columns, response, schema)
    }

    async fn collect_by_block(
        partition: Partition,
        source: Arc<Source>,
        query: Arc<Query>,
        inner_request_size: Option<u64>,
    ) -> R<HashMap<Datatype, DataFrame>> {
        if query.batch_state_reads {
            state_override_collect_by_block::<Self>(partition, source, query, inner_request_size)
                .await
        } else {
            default_collect_by_block::<Self>(partition, source, query, inner_request_size).await
        }
    }
}

impl CollectByTransaction for Slots {
    type Response = ();
}

/// Read whole contracts' worth of slots per request instead of one slot per
/// request.
///
/// Every row of this dataset is a `(block, contract, slot)` triple, and the
/// per-row path spends one `eth_getStorageAt` on each. Grouping by
/// `(block, contract)` and overriding that contract's code with the `SLOAD`
/// extractor turns a group into a single `eth_call` — which is the entire point
/// of the technique: there is no aggregator contract for storage, because a
/// deployed contract cannot read another's slots, but bytecode *injected into*
/// the target can read all of them.
impl StateOverrideBatchable for Slots {
    fn reader() -> StateReader {
        StateReader::Storage
    }

    fn target(params: &Params) -> R<Address> {
        // `Dim::Contract` is aliased to `Dim::Address` for this dataset, so the
        // contract under inspection arrives as `address`.
        //
        // `ethers_address` rather than `Address::from_slice`, which panics on a
        // width mismatch — and `--address` is hex-decoded without a length
        // check. This runs in the grouping loop, not in a worker task, so a
        // panic here would take the whole freeze down instead of one row. As an
        // error it makes the row ineligible, and the per-call path reports the
        // bad argument exactly as it did before batching existed.
        params.ethers_address()
    }

    fn input_word(params: &Params) -> R<U256> {
        // `U256::from_be_slice` PANICS above 32 bytes, and a slot arrives from
        // the command line, so an over-long `--slot` would abort a worker task
        // rather than report a bad argument. The checked form turns it into an
        // error, which the runner treats as "not batchable" and sends down the
        // per-call path, where the same value is rejected with a real message.
        U256::try_from_be_slice(&params.slot()?).ok_or_else(|| err("slot does not fit in 32 bytes"))
    }

    fn decode_row(params: &Params, value: B256) -> R<Self::Response> {
        // `B256::to_vec` is the same 32 big-endian bytes `U256::to_vec_u8`
        // produces on the per-row path, so both paths write identical cells.
        Ok((params.block_number()? as u32, None, params.address()?, params.slot()?, value.to_vec()))
    }
}

fn process_slot(columns: &mut Slots, data: BlockTxAddressOutput, schema: &Table) -> R<()> {
    let (block, _tx, address, slot, output) = data;
    columns.n_rows += 1;
    store!(schema, columns, block_number, block);
    store!(schema, columns, address, address);
    store!(schema, columns, slot, slot);
    store!(schema, columns, value, output);
    Ok(())
}
