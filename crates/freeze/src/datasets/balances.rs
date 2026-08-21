use crate::*;
use alloy::primitives::{Address, B256, U256};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for balances
#[triodion_macros::to_df(Datatype::Balances)]
#[derive(Default)]
pub struct Balances {
    n_rows: usize,
    block_number: Vec<u32>,
    address: Vec<Vec<u8>>,
    balance: Vec<U256>,
    chain_id: Vec<u64>,
}

impl Dataset for Balances {
    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "address"])
    }

    fn required_parameters() -> Vec<Dim> {
        vec![Dim::Address]
    }

    fn default_blocks() -> Option<String> {
        Some("latest".to_string())
    }
}

type BlockTxAddressOutput = (u32, Option<Vec<u8>>, Vec<u8>, U256);

impl CollectByBlock for Balances {
    type Response = BlockTxAddressOutput;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let address = request.address()?;
        let block_number = request.block_number()? as u32;
        let balance =
            source.get_balance(Address::from_slice(&address), block_number.into()).await?;
        Ok((block_number, None, address, balance))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get(&Datatype::Balances).ok_or(err("schema not provided"))?;
        process_balance(columns, response, schema)
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

impl CollectByTransaction for Balances {
    type Response = ();
}

/// Read a whole block's worth of balances per request.
///
/// `BALANCE` takes its account from the stack rather than from the execution
/// context, so unlike the storage extractor this one needs no cooperation from
/// the addresses being read: the loop runs at
/// [`SCRATCH_ADDRESS`](crate::SCRATCH_ADDRESS) and the calldata names the
/// accounts. Every row at a block therefore batches together, whether or not
/// the addresses are related, and no real account's code is overridden at all.
///
/// # Why balances may be read this way and `codes` may not
///
/// The batch path is a routing decision, so it has to be invisible in the
/// output: the opcode and the JSON-RPC method it stands in for must agree on
/// *every* account, not only on the ones that exist. `BALANCE` and
/// `eth_getBalance` both report zero for an account that has never been
/// touched, so a row answered by the extractor and the same row answered by
/// [`extract`](CollectByBlock::extract) after a demotion write the same cell.
///
/// `EXTCODEHASH` has no such agreement — it returns zero for an account that
/// does not exist and `keccak256("")` for one that exists with no code, where
/// `eth_getCode` answers `0x` to both — so [`Codes`](crate::Codes) batches at
/// the transport instead. That asymmetry, not the shape of the loop, is what
/// decides which datasets belong here.
impl StateOverrideBatchable for Balances {
    fn reader() -> StateReader {
        StateReader::Balance
    }

    fn target(_params: &Params) -> R<Address> {
        // The account readers ignore the target — the calldata names the
        // accounts and the code is injected at `SCRATCH_ADDRESS` — so a
        // constant is what puts every row at a block into one group.
        //
        // The trap: `Address::ZERO` is inside the reserved precompile range
        // that `StateReader::refuses_target` rejects, and it survives only
        // because that check is applied to `StateReader::Storage` alone.
        // Widening it to every reader would corrupt no row, but it would make
        // every balances row ineligible and silently return the dataset to one
        // request per row.
        Ok(Address::ZERO)
    }

    fn input_word(params: &Params) -> R<U256> {
        // `BALANCE` masks its operand to the low 160 bits, so the calldata
        // word is the address right-aligned — which is exactly what widening
        // 20 big-endian bytes to `U256` gives.
        //
        // `ethers_address` is the width check, not decoration: `--address` is
        // hex-decoded without a length check, and `U256::from_be_slice` PANICS
        // above 32 bytes. This runs in the grouping loop, not in a worker task,
        // so a panic here would take the whole freeze down instead of one row.
        // As an error it makes the row ineligible, and the per-call path
        // reports the bad argument exactly as it did before batching existed.
        Ok(U256::from_be_slice(params.ethers_address()?.as_slice()))
    }

    fn decode_row(params: &Params, value: B256) -> R<Self::Response> {
        // `BALANCE` pushes the balance as one full word, so re-widening those
        // 32 big-endian bytes is the same `U256` `eth_getBalance` hands back on
        // the per-row path. Both paths write identical cells, which is the
        // property that lets a batch demote mid-partition.
        Ok((params.block_number()? as u32, None, params.address()?, U256::from_be_bytes(value.0)))
    }
}

fn process_balance(columns: &mut Balances, data: BlockTxAddressOutput, schema: &Table) -> R<()> {
    let (block, _tx, address, balance) = data;
    columns.n_rows += 1;
    store!(schema, columns, block_number, block);
    store!(schema, columns, address, address);
    store!(schema, columns, balance, balance);
    Ok(())
}
