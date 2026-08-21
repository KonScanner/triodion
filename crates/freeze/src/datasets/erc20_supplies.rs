use crate::*;
use alloy::{
    primitives::{Address, Bytes, U256},
    sol_types::SolCall,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for transactions
#[triodion_macros::to_df(Datatype::Erc20Supplies)]
#[derive(Default)]
pub struct Erc20Supplies {
    n_rows: u64,
    block_number: Vec<u32>,
    erc20: Vec<Vec<u8>>,
    total_supply: Vec<Option<U256>>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc20Supplies {
    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["erc20", "block_number"])
    }

    fn required_parameters() -> Vec<Dim> {
        vec![Dim::Address]
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }

    fn default_blocks() -> Option<String> {
        Some("latest".to_string())
    }
}

impl CollectByBlock for Erc20Supplies {
    type Response = (u32, Vec<u8>, Option<U256>);

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        // `totalSupply()` takes no arguments, so the selector is the whole
        // calldata. This used to append the contract address; Solidity and
        // Vyper both ignore trailing calldata so it did not corrupt results,
        // but it meant this path and the Multicall3 path sent different bytes
        // for the same row.
        let call_data = ERC20::totalSupplyCall {}.abi_encode();
        let block_number = request.ethers_block_number()?;
        let contract = request.ethers_address()?;

        // `contract_read` keeps the two failure modes apart. A revert (or an
        // address with no code) is a real answer about the chain and becomes a
        // null cell; a node that could not serve the state — pruned history on
        // a non-archive endpoint, a rate limit, a timeout — propagates, so the
        // chunk is counted as errored instead of written out as nulls.
        let output = contract_read(source.call2(contract, call_data, block_number).await)?;
        let total_supply = output.and_then(|bytes| decode_u256_word(&bytes));
        Ok((request.block_number()? as u32, request.address()?, total_supply))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc20Supplies)?;
        let (block, erc20, total_supply) = response;
        columns.n_rows += 1;
        store!(schema, columns, block_number, block);
        store!(schema, columns, erc20, erc20);
        store!(schema, columns, total_supply, total_supply);
        Ok(())
    }

    async fn collect_by_block(
        partition: Partition,
        source: Arc<Source>,
        query: Arc<Query>,
        inner_request_size: Option<u64>,
    ) -> R<HashMap<Datatype, DataFrame>> {
        if query.multicall {
            multicall_collect_by_block::<Self>(partition, source, query, inner_request_size).await
        } else {
            default_collect_by_block::<Self>(partition, source, query, inner_request_size).await
        }
    }
}

impl CollectByTransaction for Erc20Supplies {
    type Response = ();
}

impl MulticallBatchable for Erc20Supplies {
    fn calls_for_row(params: &Params, require_success: bool) -> R<Vec<Multicall3::Call3>> {
        let target = Address::from_slice(&params.address()?);
        // totalSupply() takes no args; emit just the selector. (The legacy
        // per-call path above concatenates the address to the calldata —
        // harmless because extra calldata is ignored, but the batched path
        // does it correctly.)
        let call_data = ERC20::totalSupplyCall {}.abi_encode();
        Ok(vec![Multicall3::Call3 {
            target,
            allowFailure: !require_success,
            callData: Bytes::from(call_data),
        }])
    }

    fn decode_row(params: &Params, results: &[Multicall3::Result]) -> R<Self::Response> {
        // Indexing here would panic the worker task on a short aggregate3
        // return, taking the whole chunk with it.
        let r = results.first().ok_or_else(|| err("multicall returned no result for row"))?;
        let total_supply = if r.success { decode_u256_word(&r.returnData) } else { None };
        Ok((params.block_number()? as u32, params.address()?, total_supply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        providers::ProviderBuilder, rpc::json_rpc::ErrorPayload, transports::mock::Asserter,
    };
    use std::borrow::Cow;

    /// A `Source` whose provider answers from a canned FIFO queue.
    ///
    /// This is the harness the project did not have: before it, no test
    /// anywhere constructed a `Source` or called a `Dataset::extract`, so
    /// nothing could observe what an extractor does when the node says no.
    fn mocked_source(asserter: Asserter) -> Arc<Source> {
        Arc::new(Source {
            provider: ProviderBuilder::default().connect_mocked_client(asserter),
            chain_id: 1,
            inner_request_size: 1,
            max_concurrent_chunks: None,
            rpc_url: String::new(),
            semaphore: Arc::new(None),
            rate_limiter: Arc::new(None),
            labels: SourceLabels::default(),
            l1_provider: None,
            l1_chain_id: None,
            l1_rpc_url: None,
        })
    }

    fn usdc_at(block: u64) -> Params {
        Params {
            block_number: Some(block),
            address: Some(alloy::hex::decode("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48").unwrap()),
            ..Default::default()
        }
    }

    /// `extract` ignores its `Query`; a stub keeps the signature satisfiable.
    fn stub_query() -> Arc<Query> {
        Arc::new(Query {
            datatypes: Vec::new(),
            schemas: HashMap::new(),
            time_dimension: TimeDimension::Blocks,
            partitions: Vec::new(),
            partitioned_by: Vec::new(),
            exclude_failed: false,
            js_tracer: None,
            labels: QueryLabels { align: false, reorg_buffer: 0 },
            multicall: false,
            multicall_batch_size: 0,
            multicall_require_success: false,
        })
    }

    #[tokio::test]
    async fn a_pruned_state_error_surfaces_instead_of_becoming_a_null() {
        // THE regression. A non-archive endpoint answers `eth_call` with
        // -32602. Before the fix, `.await.ok()` turned this into `None` and the
        // run wrote a row with a null supply while reporting
        // "chunks errored: 0 / 1 (0.0%)" and "rows written: 15".
        let asserter = Asserter::new();
        asserter.push_failure(ErrorPayload {
            code: -32602,
            message: Cow::Borrowed("Archive requests require a personal token."),
            data: None,
        });

        let result = <Erc20Supplies as CollectByBlock>::extract(
            usdc_at(25_800_000),
            mocked_source(asserter),
            stub_query(),
        )
        .await;

        assert!(
            result.is_err(),
            "a node that could not serve the state must error, not report a null supply"
        );
    }

    #[tokio::test]
    async fn a_revert_becomes_a_null_supply_and_the_row_survives() {
        // The other half of the contract: a contract-level refusal is real
        // information about the chain, so the row is kept with a null value.
        let asserter = Asserter::new();
        asserter.push_failure(ErrorPayload {
            code: 3,
            message: Cow::Borrowed("execution reverted"),
            data: None,
        });

        let (_block, _erc20, total_supply) = <Erc20Supplies as CollectByBlock>::extract(
            usdc_at(25_800_000),
            mocked_source(asserter),
            stub_query(),
        )
        .await
        .expect("a revert is a valid answer, not a collection failure");

        assert_eq!(total_supply, None);
    }

    #[tokio::test]
    async fn an_address_with_no_code_yields_a_null_not_a_zero_supply() {
        // `eth_call` to an EOA succeeds and returns `0x`. The old decoder
        // (`U256::try_from_be_slice`) turned that into `Some(0)` — a supply of
        // exactly zero, indistinguishable from a real zero-supply token.
        let asserter = Asserter::new();
        asserter.push_success(&Bytes::new());

        let (_block, _erc20, total_supply) = <Erc20Supplies as CollectByBlock>::extract(
            usdc_at(25_800_000),
            mocked_source(asserter),
            stub_query(),
        )
        .await
        .expect("an empty return is not a collection failure");

        assert_eq!(total_supply, None, "an empty return must not decode to a supply of 0");
    }

    #[tokio::test]
    async fn a_well_formed_supply_is_decoded() {
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&49_641_587_955_613_032u64.to_be_bytes());
        let asserter = Asserter::new();
        asserter.push_success(&Bytes::from(word.to_vec()));

        let (block, _erc20, total_supply) = <Erc20Supplies as CollectByBlock>::extract(
            usdc_at(25_800_000),
            mocked_source(asserter),
            stub_query(),
        )
        .await
        .expect("a 32-byte return decodes");

        assert_eq!(block, 25_800_000);
        assert_eq!(total_supply, Some(U256::from(49_641_587_955_613_032u64)));
    }
}
