use crate::*;
use alloy::{
    primitives::{Address, TxHash},
    rpc::types::{
        eth::Log,
        trace::parity::{Action, LocalizedTransactionTrace, TraceOutput},
        BlockTransactionsKind, Filter, FilterBlockOption,
    },
    sol_types::SolEvent,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for transactions
#[triodion_macros::to_df(Datatype::AddressAppearances)]
#[derive(Default)]
pub struct AddressAppearances {
    n_rows: usize,
    block_number: Vec<u32>,
    block_hash: Vec<Vec<u8>>,
    transaction_hash: Vec<Vec<u8>>,
    address: Vec<Vec<u8>>,
    relationship: Vec<String>,
    chain_id: Vec<u64>,
}

impl Dataset for AddressAppearances {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            // "block_hash",
            "transaction_hash",
            "address",
            "relationship",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "transaction_hash", "address", "relationship"])
    }
}

type BlockLogsTraces = (RpcBlock, Vec<Log>, Vec<LocalizedTransactionTrace>);

impl CollectByBlock for AddressAppearances {
    type Response = BlockLogsTraces;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let block_number = request.ethers_block_number()?;
        let filter = Filter {
            block_option: FilterBlockOption::Range {
                from_block: Some(block_number.into()),
                to_block: Some(block_number.into()),
            },
            ..Default::default()
        };

        // Three independent reads of the same block: none of them needs another's
        // answer, so awaiting them in sequence spent three round trips on one
        // block's worth of latency. `try_join!` issues all three and waits once.
        //
        // The request count is unchanged — the node still sees three calls, and
        // still charges three permits from the concurrency semaphore. What
        // changes is that a partition of N blocks costs N round trips of
        // latency instead of 3N.
        let (block, logs, traces) = futures::try_join!(
            source.get_block(request.block_number()?, BlockTransactionsKind::Hashes),
            source.get_logs(&filter),
            source.trace_block(request.block_number()?),
        )?;
        let block = block.ok_or(CollectError::CollectError("block not found".to_string()))?;
        Ok((block, logs, traces))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::AddressAppearances)?;
        process_appearances(response, columns, schema)
    }
}

impl CollectByTransaction for AddressAppearances {
    type Response = BlockLogsTraces;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let tx_hash = request.ethers_transaction_hash()?;

        // The receipt and the traces are keyed by transaction hash, which we
        // already have, so neither waits on the transaction body. Only the block
        // does — it is fetched by number, and the number comes out of the body.
        // So the four reads collapse into two round trips rather than four.
        let (tx_data, receipt, traces) = futures::try_join!(
            source.get_transaction_by_hash(tx_hash),
            source.get_transaction_receipt(tx_hash),
            source.trace_transaction(tx_hash),
        )?;

        let tx_data = tx_data.ok_or_else(|| {
            CollectError::CollectError("could not find transaction data".to_string())
        })?;
        let logs = receipt
            .ok_or(CollectError::CollectError("could not get tx receipt".to_string()))?
            .inner
            .logs()
            .to_vec();

        let block_number = tx_data
            .block_number
            .ok_or_else(|| CollectError::CollectError("block not found".to_string()))?;
        let block = source
            .get_block(block_number, BlockTransactionsKind::Hashes)
            .await?
            .ok_or(CollectError::CollectError("could not get block".to_string()))?;

        Ok((block, logs, traces))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::AddressAppearances)?;
        process_appearances(response, columns, schema)
    }
}

fn name(log: &Log) -> Option<&'static str> {
    let event = log.topic0().unwrap();
    if event == *ERC20::Transfer::SIGNATURE_HASH {
        if !log.data().data.is_empty() {
            Some("erc20_transfer")
        } else if log.topics().len() == 4 {
            Some("erc721_transfer")
        } else {
            None
        }
    } else {
        None
    }
}

/// The two parties to an ERC-20 or ERC-721 `Transfer`, in `(from, to)` order.
///
/// Returned as a pair deliberately. Both addresses are the low 20 bytes of an
/// indexed topic and differ only in *which* topic, so reading one of them twice
/// reads correctly at the call site. That is exactly what happened here: both
/// reads took `topics[1]`, so every transfer recipient was dropped and every
/// sender was written twice.
///
/// # Panics
///
/// If `log` carries fewer than three topics. Callers filter on that first.
fn transfer_parties(log: &Log) -> (Address, Address) {
    (Address::from_slice(&log.topics()[1][12..32]), Address::from_slice(&log.topics()[2][12..32]))
}

impl AddressAppearances {
    fn process_first_transaction(
        &mut self,
        block_author: Address,
        trace: &LocalizedTransactionTrace,
        schema: &Table,
        tx_hash: TxHash,
        logs_by_tx: &HashMap<TxHash, Vec<Log>>,
    ) {
        let block_number = trace.block_number.unwrap() as u32;
        let block_hash = trace.block_hash.unwrap().to_vec();
        self.process_address(block_author, "miner_fee", block_number, &block_hash, tx_hash, schema);

        if let Some(logs) = logs_by_tx.get(&tx_hash) {
            for log in logs.iter() {
                if log.topics().len() >= 3 {
                    if let Some(event) = name(log) {
                        // Derive both labels from `event`, never from each
                        // other: the old code shadowed `name`, so the second
                        // label came out as "..._from_to".
                        let (from, to) = transfer_parties(log);
                        for (address, suffix) in [(from, "_from"), (to, "_to")] {
                            self.process_address(
                                address,
                                &(event.to_string() + suffix),
                                block_number,
                                &block_hash,
                                tx_hash,
                                schema,
                            );
                        }
                    }
                }
            }
        }

        match &trace.trace.action {
            Action::Call(action) => {
                self.process_address(
                    action.from,
                    "tx_from",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
                self.process_address(
                    action.to,
                    "tx_to",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
            }
            Action::Create(action) => {
                self.process_address(
                    action.from,
                    "tx_from",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
            }
            _ => {}
        }

        if let Some(TraceOutput::Create(result)) = &trace.trace.result {
            self.process_address(
                result.address,
                "tx_to",
                block_number,
                &block_hash,
                tx_hash,
                schema,
            );
        }
    }

    fn process_trace(
        &mut self,
        trace: &LocalizedTransactionTrace,
        schema: &Table,
        tx_hash: TxHash,
    ) {
        let block_number = trace.block_number.unwrap() as u32;
        let block_hash = trace.block_hash.unwrap().to_vec();
        match &trace.trace.action {
            Action::Call(action) => {
                self.process_address(
                    action.from,
                    "call_from",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
                self.process_address(
                    action.to,
                    "call_to",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
            }
            Action::Create(action) => {
                self.process_address(
                    action.from,
                    "factory",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
            }
            Action::Selfdestruct(action) => {
                self.process_address(
                    action.address,
                    "suicide",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
                self.process_address(
                    action.refund_address,
                    "suicide_refund",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
            }
            Action::Reward(action) => {
                self.process_address(
                    action.author,
                    "author",
                    block_number,
                    &block_hash,
                    tx_hash,
                    schema,
                );
            }
        }

        if let Some(TraceOutput::Create(result)) = &trace.trace.result {
            self.process_address(
                result.address,
                "create",
                block_number,
                &block_hash,
                tx_hash,
                schema,
            );
        };
    }

    fn process_address(
        &mut self,
        address: Address,
        relationship: &str,
        block_number: u32,
        block_hash: &[u8],
        transaction_hash: TxHash,
        schema: &Table,
    ) {
        self.n_rows += 1;
        store!(schema, self, address, address.to_vec());
        store!(schema, self, relationship, relationship.to_string());
        store!(schema, self, block_number, block_number);
        store!(schema, self, block_hash, block_hash.to_vec());
        store!(schema, self, transaction_hash, transaction_hash.to_vec());
    }
}

fn process_appearances(
    traces: BlockLogsTraces,
    columns: &mut AddressAppearances,
    schema: &Table,
) -> R<()> {
    let (block, logs, traces) = traces;
    let mut logs_by_tx: HashMap<TxHash, Vec<Log>> = HashMap::new();
    for log in logs.into_iter() {
        if let Some(tx_hash) = log.transaction_hash {
            logs_by_tx.entry(tx_hash).or_default().push(log);
        }
    }

    let (_block_number, block_author) = (block.header.number, block.header.beneficiary);

    let mut current_tx_hash = TxHash::ZERO;
    for trace in traces.iter() {
        if let (Some(tx_hash), Some(_tx_pos)) = (trace.transaction_hash, trace.transaction_position)
        {
            if tx_hash != current_tx_hash {
                columns.process_first_transaction(block_author, trace, schema, tx_hash, &logs_by_tx)
            }
            columns.process_trace(trace, schema, tx_hash);
            current_tx_hash = tx_hash;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Bytes, LogData};

    fn transfer_log(from: Address, to: Address) -> Log {
        let inner = alloy::primitives::Log {
            address: Address::repeat_byte(0xda),
            data: LogData::new_unchecked(
                vec![ERC20::Transfer::SIGNATURE_HASH, from.into_word(), to.into_word()],
                Bytes::from(vec![0u8; 32]),
            ),
        };
        Log { inner, ..Default::default() }
    }

    #[test]
    fn the_transfer_parties_come_from_different_topics() {
        let from = Address::repeat_byte(0x11);
        let to = Address::repeat_byte(0x22);
        // Before the fix this returned `(from, from)`: the recipient of every
        // ERC-20 and ERC-721 transfer was absent from the dataset.
        assert_eq!(transfer_parties(&transfer_log(from, to)), (from, to));
    }
}
