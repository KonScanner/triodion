use crate::{
    types::rpc_params::{
        address_topic_matches, fixed_from_slice, log_address_matches, topic_matches,
    },
    *,
};
use alloy::{
    primitives::{Address, B256, U256},
    rpc::types::{Filter, Log, Topic},
    sol_types::SolEvent,
};
use polars::prelude::*;

/// columns for erc1155_transfers
///
/// One row per token id moved. Covers `TransferSingle` and `TransferBatch` in
/// one table, because both describe the same event: an amount of one token id
/// moving from one party to another.
///
/// `TransferBatch` carries two parallel arrays, `ids` and `values`. Storing
/// them whole, or storing their length, would put an aggregate in a cell that
/// cannot be taken apart again — which token moved, and how much of it, is
/// gone. A batch of five ids therefore becomes five rows, and a
/// `TransferSingle` becomes one row of exactly that shape.
/// `(log_index, token_id_index)` is the unique key within a block.
///
/// Both events carry a signature plus three indexed arguments, so the topic
/// count cannot tell them apart. topic0 is the only discriminator.
///
/// ERC-1155 indexes `operator` first, so `from` and `to` sit one topic slot
/// later than in ERC-20 and ERC-721 — see the filter in `extract`.
#[triodion_macros::to_df(Datatype::Erc1155Transfers)]
#[derive(Default)]
pub struct Erc1155Transfers {
    n_rows: u64,
    block_number: Vec<u32>,
    block_hash: Vec<Option<Vec<u8>>>,
    transaction_index: Vec<u32>,
    log_index: Vec<u32>,
    // Position of this token id within its own log's `ids` list, and 0 for a
    // `TransferSingle`. Batch order is emitted order and carries meaning, so
    // it is kept rather than reconstructed. Not `transfer_index`: that name
    // already counts within a block in `native_transfers`, and one word must
    // keep one meaning across datasets.
    token_id_index: Vec<u32>,
    transaction_hash: Vec<Vec<u8>>,
    erc1155: Vec<Vec<u8>>,
    // The `msg.sender` that performed the transfer, which need not be `from`:
    // an approved operator moves someone else's tokens under their own name.
    operator: Vec<Vec<u8>>,
    from_address: Vec<Vec<u8>>,
    to_address: Vec<Vec<u8>>,
    token_id: Vec<U256>,
    value: Vec<U256>,
    // True when the row came from a `TransferBatch`. ERC-1155 is a final
    // standard with exactly these two transfer events, so a flag states the
    // whole fact; every other column on the row reads the same either way.
    is_batch: Vec<bool>,
    // Derived: `from_address` is the zero address. ERC-1155 encodes a mint
    // that way, so the zero address is not an account that sent anything.
    // Without this column `GROUP BY from_address` reports it as the busiest
    // sender on the chain, and a supply figure counts it as a holder.
    is_mint: Vec<bool>,
    // Derived: `to_address` is the zero address, which encodes a burn. Kept
    // apart from `is_mint` because the two are independent facts about a row
    // and a single enum column would have to discard one of them.
    is_burn: Vec<bool>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc1155Transfers {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            // "block_hash",
            "transaction_index",
            "log_index",
            "token_id_index",
            "transaction_hash",
            "erc1155",
            "operator",
            "from_address",
            "to_address",
            "token_id",
            "value",
            "is_batch",
            "is_mint",
            "is_burn",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        // The inherited default stops at `log_index`, which leaves every row of
        // a batch tied. Polars does not promise to keep tied rows in input
        // order, so emitted batch order would be lost on write. Sorting by the
        // ordinal too makes the order total and reproducible.
        Some(vec!["block_number", "log_index", "token_id_index"])
    }

    fn optional_parameters() -> Vec<Dim> {
        // Topic0 is fixed to the two transfer signatures. Topic1 is the
        // indexed `operator`; `--from-address` and `--to-address` reach
        // topic2 and topic3, so they are named rather than exposed as raw
        // topic dimensions.
        vec![Dim::Address, Dim::Topic1, Dim::FromAddress, Dim::ToAddress]
    }

    fn use_block_ranges() -> bool {
        true
    }

    fn default_inner_request_size() -> u64 {
        // Same as the other log-shaped datasets: one `eth_getLogs` per 50
        // blocks rather than one per block. Override with --inner-request-size.
        50
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for Erc1155Transfers {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let mut topics: [Topic; 4] = Default::default();
        // A `Topic` is a set of values and `eth_getLogs` reads a set at one
        // position as OR, so both signatures come back from one request.
        topics[0] =
            vec![ERC1155::TransferSingle::SIGNATURE_HASH, ERC1155::TransferBatch::SIGNATURE_HASH]
                .into();
        // ERC-1155 indexes operator, from, to — in that order. `from` is
        // topic2 and `to` is topic3, not topic1 and topic2 as in ERC-20 and
        // ERC-721. Copying that layout filters on the operator instead and
        // returns an empty file with no error at all.
        if let Some(operator) = &request.topic1 {
            topics[1] = fixed_from_slice::<B256>(operator, "topic1")?.into();
        }
        if let Some(from_address) = &request.from_address {
            // `--from-address` is documented as a 20-byte address; left-pad it
            // into the 32-byte topic slot rather than panicking on a short one.
            let v = address_dim_as_topic(from_address).ok_or_else(|| {
                CollectError::CollectError("from_address must be at most 32 bytes".to_string())
            })?;
            topics[2] = v.into();
        }
        if let Some(to_address) = &request.to_address {
            let v = address_dim_as_topic(to_address).ok_or_else(|| {
                CollectError::CollectError("to_address must be at most 32 bytes".to_string())
            })?;
            topics[3] = v.into();
        }
        let filter = Filter { topics, ..request.ethers_log_filter()? };
        let logs = source.get_logs(&filter).await?;
        Ok(logs.into_iter().filter(is_erc1155_transfer).collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc1155Transfers)?;
        process_erc1155_transfers(response, columns, schema)
    }
}

impl CollectByTransaction for Erc1155Transfers {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let logs = source.get_transaction_logs(request.transaction_hash()?).await?;
        // The dims never reach the node on this path: `--txs` asks for one
        // transaction's whole receipt, so the narrowing the by-block path pins
        // into the `eth_getLogs` filter has to be re-applied here. Without it a
        // dim is accepted, counted into the partition set, printed in the run
        // summary, and then ignored — the run returns rows it was asked to
        // exclude and says nothing.
        Ok(logs
            .into_iter()
            .filter(|log| {
                is_erc1155_transfer(log) &&
                    log_address_matches(log, &request.address) &&
                    topic_matches(log, 1, &request.topic1) &&
                    address_topic_matches(log, 2, &request.from_address) &&
                    address_topic_matches(log, 3, &request.to_address)
            })
            .collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc1155Transfers)?;
        process_erc1155_transfers(response, columns, schema)
    }
}

/// True iff `log` carries one of the two ERC-1155 transfer signatures and the
/// topic count both events share: signature plus indexed operator, from, to.
///
/// Data width is deliberately not checked. A `TransferSingle` body is two
/// static words, but a `TransferBatch` body is two dynamic arrays behind
/// offsets and has no fixed length, so the ABI decoder in
/// [`process_erc1155_transfers`] is what finally validates a body. Written to
/// stay reusable by a coalesced [`crate::LogEvents`] fan-out, which this
/// datatype is NOT a member of today (see `MultiDatatype::LogEvents`).
pub(crate) fn is_erc1155_transfer(log: &Log) -> bool {
    log.topics().len() == 4 &&
        log.topics().first().is_some_and(|t| {
            *t == ERC1155::TransferSingle::SIGNATURE_HASH ||
                *t == ERC1155::TransferBatch::SIGNATURE_HASH
        })
}

/// One decoded transfer log, flattened to the parties plus the (id, value)
/// pairs it moved. A `TransferSingle` yields one pair, a `TransferBatch` as
/// many as it listed.
struct DecodedTransfer {
    operator: Address,
    from: Address,
    to: Address,
    is_batch: bool,
    /// `(token_id, value)`, in the order the contract emitted them.
    items: Vec<(U256, U256)>,
}

/// Decode either transfer event, or `None` for anything this dataset cannot
/// represent. Every rejection here becomes a skipped log, never a panic.
fn decode_transfer(log: &Log) -> Option<DecodedTransfer> {
    // Both events have 4 topics, so only topic0 separates them.
    let topic0 = *log.topics().first()?;

    if topic0 == ERC1155::TransferSingle::SIGNATURE_HASH {
        // `decode_log_data` checks the topic count and the signature before it
        // touches the body. The single body is two static words and could be
        // sliced by hand, but it goes through the same path as the batch so
        // the two cannot drift apart.
        let event = ERC1155::TransferSingle::decode_log_data(log.data()).ok()?;
        Some(DecodedTransfer {
            operator: event.operator,
            from: event.from,
            to: event.to,
            is_batch: false,
            items: vec![(event.id, event.value)],
        })
    } else if topic0 == ERC1155::TransferBatch::SIGNATURE_HASH {
        // The body is two dynamically sized arrays reached through offsets.
        // Let alloy walk it: a hand-written slice of an offset-encoded body is
        // where the bugs live.
        let event = ERC1155::TransferBatch::decode_log_data(log.data()).ok()?;
        // The two arrays are parallel by definition. Unequal lengths mean the
        // emitter is not compliant and no pairing can be recovered: padding
        // either side would invent a transfer that never happened, and
        // truncating would drop one that did. Skip the whole log.
        if event.ids.len() != event.values.len() {
            return None
        }
        Some(DecodedTransfer {
            operator: event.operator,
            from: event.from,
            to: event.to,
            is_batch: true,
            // An empty batch is well formed and moved nothing, so it yields no
            // rows. That is an absence, not a zero-value transfer.
            items: event.ids.into_iter().zip(event.values).collect(),
        })
    } else {
        None
    }
}

/// Explode each transfer log into one row per token id.
fn process_erc1155_transfers(
    logs: Vec<Log>,
    columns: &mut Erc1155Transfers,
    schema: &Table,
) -> R<()> {
    for log in logs.iter() {
        let (Some(bn), Some(tx), Some(ti), Some(li)) =
            (log.block_number, log.transaction_hash, log.transaction_index, log.log_index)
        else {
            continue
        };
        // A foreign topic0, a truncated body, or a batch whose two arrays
        // disagree all land here and produce no rows. Callers that reach this
        // function with mixed logs — a future coalesced [`crate::LogEvents`]
        // extractor, or a future one — therefore stay safe without a
        // pre-filter of their own.
        let Some(transfer) = decode_transfer(log) else { continue };
        let DecodedTransfer { operator, from, to, is_batch, items } = transfer;

        let block_hash = log.block_hash.map(|bh| bh.to_vec());
        let erc1155 = log.address().to_vec();
        for (index, (token_id, value)) in items.into_iter().enumerate() {
            columns.n_rows += 1;
            store!(schema, columns, block_number, bn as u32);
            store!(schema, columns, block_hash, block_hash.clone());
            store!(schema, columns, transaction_index, ti as u32);
            store!(schema, columns, log_index, li as u32);
            store!(schema, columns, token_id_index, index as u32);
            store!(schema, columns, transaction_hash, tx.to_vec());
            store!(schema, columns, erc1155, erc1155.clone());
            store!(schema, columns, operator, operator.to_vec());
            store!(schema, columns, from_address, from.to_vec());
            store!(schema, columns, to_address, to.to_vec());
            store!(schema, columns, token_id, token_id);
            store!(schema, columns, value, value);
            store!(schema, columns, is_batch, is_batch);
            store!(schema, columns, is_mint, from == Address::ZERO);
            store!(schema, columns, is_burn, to == Address::ZERO);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::LogData;

    /// real `Erc1155Transfers` schema, so `store!` actually evaluates each column
    fn schema() -> Table {
        Datatype::Erc1155Transfers
            .table_schema(
                &[U256Type::String],
                &ColumnEncoding::Hex,
                &None,
                &None,
                &None,
                None,
                None,
            )
            .expect("Erc1155Transfers has a valid default schema")
    }

    /// rpc `Log` carrying the block/tx/index fields `process_erc1155_transfers` requires
    fn rpc_log(data: LogData) -> Log {
        Log {
            inner: alloy::primitives::Log { address: Address::repeat_byte(0x11), data },
            block_number: Some(1),
            transaction_hash: Some(B256::ZERO),
            transaction_index: Some(0),
            log_index: Some(7),
            ..Default::default()
        }
    }

    fn addr(byte: u8) -> Address {
        Address::repeat_byte(byte)
    }

    #[test]
    fn batch_explodes_into_one_row_per_token_id() {
        let event = ERC1155::TransferBatch {
            operator: addr(0xaa),
            from: addr(0xbb),
            to: addr(0xcc),
            ids: vec![U256::from(1), U256::from(2), U256::from(3)],
            values: vec![U256::from(10), U256::from(20), U256::from(30)],
        };
        let mut columns = Erc1155Transfers::default();
        process_erc1155_transfers(vec![rpc_log(event.encode_log_data())], &mut columns, &schema())
            .unwrap();

        assert_eq!(columns.n_rows, 3);
        assert_eq!(columns.token_id_index, vec![0, 1, 2]);
        assert_eq!(columns.token_id, vec![U256::from(1), U256::from(2), U256::from(3)]);
        assert_eq!(columns.value, vec![U256::from(10), U256::from(20), U256::from(30)]);
        assert_eq!(columns.is_batch, vec![true, true, true]);
        // every row of a batch repeats the one log_index it came from
        assert_eq!(columns.log_index, vec![7, 7, 7]);
    }

    #[test]
    fn single_is_one_row_of_the_same_shape() {
        let event = ERC1155::TransferSingle {
            operator: addr(0xaa),
            from: addr(0xbb),
            to: addr(0xcc),
            id: U256::from(42),
            value: U256::from(5),
        };
        let mut columns = Erc1155Transfers::default();
        process_erc1155_transfers(vec![rpc_log(event.encode_log_data())], &mut columns, &schema())
            .unwrap();

        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.token_id_index, vec![0]);
        assert_eq!(columns.is_batch, vec![false]);
        assert_eq!(columns.operator, vec![vec![0xaau8; 20]]);
        assert_eq!(columns.from_address, vec![vec![0xbbu8; 20]]);
        assert_eq!(columns.to_address, vec![vec![0xccu8; 20]]);
    }

    #[test]
    fn batch_with_mismatched_array_lengths_is_skipped_whole() {
        // Two ids and one value: no pairing is recoverable. The log must
        // produce zero rows — never a padded or truncated one, never a panic.
        let event = ERC1155::TransferBatch {
            operator: addr(0xaa),
            from: addr(0xbb),
            to: addr(0xcc),
            ids: vec![U256::from(1), U256::from(2)],
            values: vec![U256::from(10)],
        };
        let mut columns = Erc1155Transfers::default();
        process_erc1155_transfers(vec![rpc_log(event.encode_log_data())], &mut columns, &schema())
            .unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn empty_batch_yields_no_rows() {
        let event = ERC1155::TransferBatch {
            operator: addr(0xaa),
            from: addr(0xbb),
            to: addr(0xcc),
            ids: vec![],
            values: vec![],
        };
        let mut columns = Erc1155Transfers::default();
        process_erc1155_transfers(vec![rpc_log(event.encode_log_data())], &mut columns, &schema())
            .unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn zero_address_sides_are_flagged_as_mint_and_burn() {
        let mint = ERC1155::TransferSingle {
            operator: addr(0xaa),
            from: Address::ZERO,
            to: addr(0xcc),
            id: U256::from(1),
            value: U256::from(1),
        };
        let burn = ERC1155::TransferSingle {
            operator: addr(0xaa),
            from: addr(0xbb),
            to: Address::ZERO,
            id: U256::from(1),
            value: U256::from(1),
        };
        let logs = vec![rpc_log(mint.encode_log_data()), rpc_log(burn.encode_log_data())];
        let mut columns = Erc1155Transfers::default();
        process_erc1155_transfers(logs, &mut columns, &schema()).unwrap();

        assert_eq!(columns.is_mint, vec![true, false]);
        assert_eq!(columns.is_burn, vec![false, true]);
    }

    #[test]
    fn topic_count_alone_does_not_separate_the_two_events() {
        // Regression guard for the dispatch: both signatures carry 4 topics, so
        // a shape-only check would decode a batch body as a single.
        assert_ne!(ERC1155::TransferSingle::SIGNATURE_HASH, ERC1155::TransferBatch::SIGNATURE_HASH);
        let batch = ERC1155::TransferBatch {
            operator: addr(0xaa),
            from: addr(0xbb),
            to: addr(0xcc),
            ids: vec![U256::from(9)],
            values: vec![U256::from(9)],
        };
        let log = rpc_log(batch.encode_log_data());
        assert_eq!(log.topics().len(), 4);
        assert!(is_erc1155_transfer(&log));
        assert!(ERC1155::TransferSingle::decode_log_data(log.data()).is_err());
    }

    #[test]
    fn foreign_topic0_is_rejected_by_the_predicate_and_the_decoder() {
        let log = rpc_log(LogData::new_unchecked(
            vec![B256::repeat_byte(0xfe), B256::ZERO, B256::ZERO, B256::ZERO],
            Default::default(),
        ));
        assert!(!is_erc1155_transfer(&log));
        let mut columns = Erc1155Transfers::default();
        process_erc1155_transfers(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn log_missing_identity_fields_is_skipped() {
        let event = ERC1155::TransferSingle {
            operator: addr(0xaa),
            from: addr(0xbb),
            to: addr(0xcc),
            id: U256::from(1),
            value: U256::from(1),
        };
        let mut log = rpc_log(event.encode_log_data());
        log.log_index = None;
        let mut columns = Erc1155Transfers::default();
        process_erc1155_transfers(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }
}
