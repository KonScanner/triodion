use crate::{
    types::rpc_params::{fixed_from_slice, log_address_matches, topic_matches},
    *,
};
use alloy::{
    primitives::{B256, U256},
    rpc::types::{Filter, Log, Topic},
    sol_types::SolEvent,
};
use polars::prelude::*;

/// columns for erc777_transfers
///
/// One row per ERC-777 `Sent`, `Minted` or `Burned` event.
///
/// # THIS DATASET DOUBLE-COUNTS AGAINST `erc20_transfers`. DO NOT UNION THEM.
///
/// ERC-777 is a superset of ERC-20, and a compliant ERC-777 token emits an
/// ERC-20 `Transfer` **for the same movement** alongside every `Sent`,
/// `Minted` and `Burned`. Both logs are real, both are in the receipt, and
/// both are collected — `Transfer` by [`Erc20Transfers`], the richer one here.
/// They are two views of one movement, not two movements.
///
/// So `SELECT ... FROM erc20_transfers UNION ALL SELECT ... FROM
/// erc777_transfers` reports every ERC-777 movement twice, and every volume,
/// count and balance derived from it is wrong by exactly the ERC-777 share of
/// the chain. Pick one table per token: this one when the operator or the
/// data payloads matter, `erc20_transfers` when only the movement does.
/// If you must have one table, anti-join on
/// `(transaction_hash, block_number, erc777)` — never on `log_index`, because
/// the mirrored pair are two different logs with two different indices.
///
/// The mirroring is a convention, not a consensus rule. A token can emit one
/// without the other, so neither table is a strict subset of the other.
#[triodion_macros::to_df(Datatype::Erc777Transfers)]
#[derive(Default)]
pub struct Erc777Transfers {
    n_rows: u64,
    block_number: Vec<u32>,
    block_hash: Vec<Option<Vec<u8>>>,
    transaction_index: Vec<u32>,
    log_index: Vec<u32>,
    transaction_hash: Vec<Vec<u8>>,
    erc777: Vec<Vec<u8>>,
    // The Solidity identifier as written in the standard — "Sent", "Minted",
    // "Burned" — so the value joins straight to an ABI without a translation
    // table. Which of the three it is decides which of the address columns
    // below is null.
    event_name: Vec<String>,
    // The party that executed the move. This is what ERC-777 adds over
    // ERC-20: an authorised operator can move a holder's tokens with no
    // allowance and no call from the holder. Always present — on an ordinary
    // self-initiated send the holder is its own operator, so a naive
    // COUNT(DISTINCT operator) counts every ordinary sender as an operator.
    // `is_operator_send` beside it separates the two.
    operator: Vec<Vec<u8>>,
    // Null on `Minted`. A mint has no payer, and ERC-777 does not use the
    // ERC-20 zero-address convention for mints, so writing 0x0 here would
    // invent a party that the event never named.
    from_address: Vec<Option<Vec<u8>>>,
    // Null on `Burned`, for the same reason: a burn names no recipient.
    to_address: Vec<Option<Vec<u8>>>,
    amount: Vec<U256>,
    // Non-indexed dynamic `bytes` from the log body, ABI-decoded. Empty is a
    // real, common value here — most sends carry no payload — and it is
    // stored as zero-length bytes, not as a null. Null would mean the field
    // does not exist, and it always exists on all three events.
    data: Vec<Vec<u8>>,
    // The operator's own payload, distinct from the holder's `data`. Same
    // rule: empty bytes, never null.
    operator_data: Vec<Vec<u8>>,
    // Derived, not reported: `operator != from_address`. True when a third
    // party moved someone else's tokens, which is the case that has no ERC-20
    // equivalent. Null on `Minted`, where there is no `from` to compare
    // against — false would claim a self-send that the event never described.
    is_operator_send: Vec<Option<bool>>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc777Transfers {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            // "block_hash",
            "transaction_index",
            "log_index",
            "transaction_hash",
            "erc777",
            "event_name",
            "operator",
            "from_address",
            "to_address",
            "amount",
            "data",
            "operator_data",
            "is_operator_send",
            "chain_id",
        ])
    }

    fn optional_parameters() -> Vec<Dim> {
        // Topic0 is fixed to the three ERC-777 signatures, so it is not
        // offered. The remaining topics are exposed by position, not by name,
        // because their meaning is not the same across the three events:
        //   topic1 = operator, on all three;
        //   topic2 = `from` on Sent and Burned, but `to` on Minted;
        //   topic3 = `to`, on Sent only.
        // A `--from-address` dim would therefore have to mean topic2, and
        // would silently match Minted recipients too. Filtering by topic3 also
        // drops every Minted and Burned row, because those logs have no
        // topic3 for the node to match against.
        vec![Dim::Address, Dim::Topic1, Dim::Topic2, Dim::Topic3]
    }

    fn use_block_ranges() -> bool {
        true
    }

    fn default_inner_request_size() -> u64 {
        // The same 50 as every other log dataset, and for the reason
        // `proxy_upgrades` gives: the dispatcher applies
        // `max(user, dataset_default)`, so this value is a floor and not a
        // default. A wider one would fetch these rare logs in fewer requests,
        // but `--inner-request-size` could then only raise it, never lower it,
        // and an endpoint that caps the block span of `eth_getLogs` would
        // reject every request with no flag left to fix it.
        50
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for Erc777Transfers {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let mut topics: [Topic; 4] = Default::default();
        // Union filter — eth_getLogs treats a list at one topic position as OR.
        topics[0] = vec![
            ERC777::Sent::SIGNATURE_HASH,
            ERC777::Minted::SIGNATURE_HASH,
            ERC777::Burned::SIGNATURE_HASH,
        ]
        .into();
        // Topic dims are the full 32-byte word, as everywhere else in the repo.
        // Left-padding a 20-byte address here would be dead code: the struct
        // update below evaluates `ethers_log_filter()`, which runs
        // `fixed_from_slice::<B256>` over the same raw dims and errors on
        // anything that is not exactly 32 bytes, so a short value never
        // survives to be padded.
        for (position, dim) in [(1, &request.topic1), (2, &request.topic2), (3, &request.topic3)] {
            if let Some(bytes) = dim {
                topics[position] =
                    fixed_from_slice::<B256>(bytes, &format!("topic{position}"))?.into();
            }
        }
        let filter = Filter { topics, ..request.ethers_log_filter()? };
        let logs = source.get_logs(&filter).await?;
        Ok(logs.into_iter().filter(is_erc777_transfer_event).collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc777Transfers)?;
        process_erc777_transfers(response, columns, schema)
    }
}

impl CollectByTransaction for Erc777Transfers {
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
                is_erc777_transfer_event(log) &&
                    log_address_matches(log, &request.address) &&
                    topic_matches(log, 1, &request.topic1) &&
                    topic_matches(log, 2, &request.topic2) &&
                    topic_matches(log, 3, &request.topic3)
            })
            .collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc777Transfers)?;
        process_erc777_transfers(response, columns, schema)
    }
}

/// True iff `log` carries one of the three ERC-777 movement signatures *and*
/// the topic count that signature implies: Sent indexes three addresses,
/// Minted and Burned index two.
///
/// The arity check is what stops a contract that reuses one of these topic0
/// values with a different argument list from being decoded as ERC-777. The
/// by-transaction path also needs the signature check, because it receives
/// every log in the transaction with no node-side filtering.
///
/// Written to stay reusable by a coalesced [`crate::LogEvents`] fan-out, which this datatype is NOT
/// a member of today (see `MultiDatatype::LogEvents`).
pub(crate) fn is_erc777_transfer_event(log: &Log) -> bool {
    match log.topics().first() {
        Some(topic0) if *topic0 == ERC777::Sent::SIGNATURE_HASH => log.topics().len() == 4,
        Some(topic0) if *topic0 == ERC777::Minted::SIGNATURE_HASH => log.topics().len() == 3,
        Some(topic0) if *topic0 == ERC777::Burned::SIGNATURE_HASH => log.topics().len() == 3,
        _ => false,
    }
}

/// One decoded ERC-777 movement, with the two absences made explicit.
struct DecodedErc777 {
    event_name: &'static str,
    operator: Vec<u8>,
    from_address: Option<Vec<u8>>,
    to_address: Option<Vec<u8>>,
    amount: U256,
    data: Vec<u8>,
    operator_data: Vec<u8>,
}

/// Decode one log, or `None` if it is not a well-formed ERC-777 movement.
///
/// `data` and `operatorData` are non-indexed dynamic `bytes`: the log body
/// holds offsets, then lengths, then padded content, and the two payloads can
/// be any length in any order of appearance. Slicing fixed windows out of the
/// body silently mangles them, so the generated ABI decoder does the work and
/// a body it rejects becomes a skipped row rather than a wrong one.
fn decode_erc777_event(log: &Log) -> Option<DecodedErc777> {
    let topic0 = *log.topics().first()?;
    let body = &log.inner.data;

    if topic0 == ERC777::Sent::SIGNATURE_HASH {
        let event = ERC777::Sent::decode_log_data(body).ok()?;
        Some(DecodedErc777 {
            event_name: "Sent",
            operator: event.operator.to_vec(),
            from_address: Some(event.from.to_vec()),
            to_address: Some(event.to.to_vec()),
            amount: event.amount,
            data: event.data.to_vec(),
            operator_data: event.operatorData.to_vec(),
        })
    } else if topic0 == ERC777::Minted::SIGNATURE_HASH {
        let event = ERC777::Minted::decode_log_data(body).ok()?;
        Some(DecodedErc777 {
            event_name: "Minted",
            operator: event.operator.to_vec(),
            // No sender exists. See the column comment.
            from_address: None,
            to_address: Some(event.to.to_vec()),
            amount: event.amount,
            data: event.data.to_vec(),
            operator_data: event.operatorData.to_vec(),
        })
    } else if topic0 == ERC777::Burned::SIGNATURE_HASH {
        let event = ERC777::Burned::decode_log_data(body).ok()?;
        Some(DecodedErc777 {
            event_name: "Burned",
            operator: event.operator.to_vec(),
            from_address: Some(event.from.to_vec()),
            // No recipient exists. See the column comment.
            to_address: None,
            amount: event.amount,
            data: event.data.to_vec(),
            operator_data: event.operatorData.to_vec(),
        })
    } else {
        None
    }
}

/// process logs into columns
fn process_erc777_transfers(
    logs: Vec<Log>,
    columns: &mut Erc777Transfers,
    schema: &Table,
) -> R<()> {
    for log in logs.iter() {
        // Both callers pre-filter, but the coalesced LogEvents extractor may
        // hand this loop mixed logs. Re-checking here keeps a foreign log a
        // skipped row instead of a mis-tagged one.
        if !is_erc777_transfer_event(log) {
            continue;
        }
        let (Some(bn), Some(tx), Some(ti), Some(li)) =
            (log.block_number, log.transaction_hash, log.transaction_index, log.log_index)
        else {
            continue;
        };
        let Some(event) = decode_erc777_event(log) else { continue };

        // Undefined rather than false when there is no counterparty to compare.
        let is_operator_send = event.from_address.as_ref().map(|from| *from != event.operator);

        columns.n_rows += 1;
        store!(schema, columns, block_number, bn as u32);
        store!(schema, columns, block_hash, log.block_hash.map(|bh| bh.to_vec()));
        store!(schema, columns, transaction_index, ti as u32);
        store!(schema, columns, log_index, li as u32);
        store!(schema, columns, transaction_hash, tx.to_vec());
        store!(schema, columns, erc777, log.address().to_vec());
        store!(schema, columns, event_name, event.event_name.to_string());
        store!(schema, columns, operator, event.operator);
        store!(schema, columns, from_address, event.from_address);
        store!(schema, columns, to_address, event.to_address);
        store!(schema, columns, amount, event.amount);
        store!(schema, columns, data, event.data);
        store!(schema, columns, operator_data, event.operator_data);
        store!(schema, columns, is_operator_send, is_operator_send);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Address, Bytes, B256};

    /// real `Erc777Transfers` schema, so `store!` actually evaluates each column
    fn schema() -> Table {
        Datatype::Erc777Transfers
            .table_schema(
                &[U256Type::String],
                &ColumnEncoding::Hex,
                &None,
                &None,
                &None,
                None,
                None,
            )
            .expect("Erc777Transfers has a valid default schema")
    }

    fn addr(byte: u8) -> Address {
        Address::repeat_byte(byte)
    }

    /// rpc `Log` around an encoded event, carrying the identity fields the
    /// processor requires
    fn rpc_log(data: alloy::primitives::LogData) -> Log {
        Log {
            inner: alloy::primitives::Log { address: addr(0x11), data },
            block_number: Some(1),
            transaction_hash: Some(B256::ZERO),
            transaction_index: Some(0),
            log_index: Some(0),
            ..Default::default()
        }
    }

    fn sent(operator: Address, from: Address, to: Address, data: Vec<u8>) -> Log {
        rpc_log(
            ERC777::Sent {
                operator,
                from,
                to,
                amount: U256::from(7u64),
                data: Bytes::from(data),
                operatorData: Bytes::new(),
            }
            .encode_log_data(),
        )
    }

    #[test]
    fn mint_has_no_sender_and_burn_has_no_recipient() {
        // The whole point of the two Option columns: an absent party must be
        // null, never the zero address.
        let minted = rpc_log(
            ERC777::Minted {
                operator: addr(0xaa),
                to: addr(0xbb),
                amount: U256::from(1u64),
                data: Bytes::new(),
                operatorData: Bytes::new(),
            }
            .encode_log_data(),
        );
        let burned = rpc_log(
            ERC777::Burned {
                operator: addr(0xaa),
                from: addr(0xcc),
                amount: U256::from(2u64),
                data: Bytes::new(),
                operatorData: Bytes::new(),
            }
            .encode_log_data(),
        );

        let mut columns = Erc777Transfers::default();
        process_erc777_transfers(vec![minted, burned], &mut columns, &schema()).unwrap();

        assert_eq!(columns.n_rows, 2);
        assert_eq!(columns.event_name, vec!["Minted".to_string(), "Burned".to_string()]);
        assert_eq!(columns.from_address, vec![None, Some(vec![0xccu8; 20])]);
        assert_eq!(columns.to_address, vec![Some(vec![0xbbu8; 20]), None]);
        // Minted has no `from`, so the comparison is undefined, not false.
        assert_eq!(columns.is_operator_send, vec![None, Some(true)]);
    }

    #[test]
    fn self_initiated_send_is_not_an_operator_send() {
        let logs = vec![
            sent(addr(0xaa), addr(0xaa), addr(0xbb), vec![]),
            sent(addr(0x99), addr(0xaa), addr(0xbb), vec![]),
        ];
        let mut columns = Erc777Transfers::default();
        process_erc777_transfers(logs, &mut columns, &schema()).unwrap();
        assert_eq!(columns.is_operator_send, vec![Some(false), Some(true)]);
    }

    #[test]
    fn dynamic_payloads_round_trip_and_empty_is_not_null() {
        // `data` is dynamic bytes in the body; a fixed-window slice would not
        // survive a payload that is not a multiple of 32 bytes.
        let payload = vec![0xde, 0xad, 0xbe, 0xef, 0x01];
        let logs = vec![
            sent(addr(0xaa), addr(0xaa), addr(0xbb), payload.clone()),
            sent(addr(0xaa), addr(0xaa), addr(0xbb), vec![]),
        ];
        let mut columns = Erc777Transfers::default();
        process_erc777_transfers(logs, &mut columns, &schema()).unwrap();
        assert_eq!(columns.data, vec![payload, Vec::<u8>::new()]);
        assert_eq!(columns.operator_data, vec![Vec::<u8>::new(), Vec::<u8>::new()]);
    }

    #[test]
    fn predicate_requires_the_arity_the_signature_implies() {
        let ok = sent(addr(0xaa), addr(0xaa), addr(0xbb), vec![]);
        assert!(is_erc777_transfer_event(&ok));

        // Sent topic0 with only three topics is not an ERC-777 Sent.
        let truncated = rpc_log(alloy::primitives::LogData::new_unchecked(
            vec![ERC777::Sent::SIGNATURE_HASH, B256::ZERO, B256::ZERO],
            ok.inner.data.data.clone(),
        ));
        assert!(!is_erc777_transfer_event(&truncated));

        let foreign = rpc_log(alloy::primitives::LogData::new_unchecked(
            vec![B256::repeat_byte(0xfe), B256::ZERO, B256::ZERO, B256::ZERO],
            ok.inner.data.data.clone(),
        ));
        assert!(!is_erc777_transfer_event(&foreign));
    }

    #[test]
    fn undecodable_body_is_skipped_not_panicking() {
        // Right signature and arity, truncated body: skip the row.
        let malformed = rpc_log(alloy::primitives::LogData::new_unchecked(
            vec![ERC777::Sent::SIGNATURE_HASH, B256::ZERO, B256::ZERO, B256::ZERO],
            Bytes::from(vec![0u8; 8]),
        ));
        assert!(is_erc777_transfer_event(&malformed));

        let mut columns = Erc777Transfers::default();
        process_erc777_transfers(vec![malformed], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn skips_logs_missing_identity_fields() {
        let mut log = sent(addr(0xaa), addr(0xaa), addr(0xbb), vec![]);
        log.log_index = None;
        let mut columns = Erc777Transfers::default();
        process_erc777_transfers(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn the_three_signatures_are_distinct() {
        let sig_sent = ERC777::Sent::SIGNATURE_HASH;
        let sig_minted = ERC777::Minted::SIGNATURE_HASH;
        let sig_burned = ERC777::Burned::SIGNATURE_HASH;
        assert_ne!(sig_sent, sig_minted);
        assert_ne!(sig_sent, sig_burned);
        assert_ne!(sig_minted, sig_burned);
        // The mirrored ERC-20 Transfer is a different log entirely, which is
        // why unioning the two datasets double-counts.
        assert_ne!(sig_sent, ERC20::Transfer::SIGNATURE_HASH);
    }
}
