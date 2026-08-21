use crate::*;
use alloy::{
    consensus::{Transaction as ConsensusTransaction, Typed2718},
    eips::eip2718::Encodable2718,
    network::{AnyTxEnvelope, ReceiptResponse},
    primitives::{Address, TxKind, U256},
    rpc::types::{BlockTransactions, BlockTransactionsKind},
    serde::OtherFields,
};
use polars::prelude::*;

/// columns for transactions
#[triodion_macros::to_df(Datatype::Transactions)]
#[derive(Default)]
pub struct Transactions {
    n_rows: u64,
    block_number: Vec<Option<u32>>,
    transaction_index: Vec<Option<u64>>,
    transaction_hash: Vec<Vec<u8>>,
    nonce: Vec<u64>,
    from_address: Vec<Vec<u8>>,
    to_address: Vec<Option<Vec<u8>>>,
    value: Vec<U256>,
    input: Vec<Vec<u8>>,
    gas_limit: Vec<u64>,
    gas_used: Vec<Option<u64>>,
    gas_price: Vec<Option<u64>>,
    transaction_type: Vec<u32>,
    max_priority_fee_per_gas: Vec<Option<u64>>,
    max_fee_per_gas: Vec<Option<u64>>,
    success: Vec<bool>,
    n_input_bytes: Vec<u32>,
    n_input_zero_bytes: Vec<u32>,
    n_input_nonzero_bytes: Vec<u32>,
    // `None` for any transaction type alloy cannot re-encode — every OP-stack
    // and Arbitrum-stack type byte. Computing it would require serialising the
    // envelope, which panics for those. See `chains::is_reencodable`.
    n_rlp_bytes: Vec<Option<u32>>,
    block_hash: Vec<Vec<u8>>,
    chain_id: Vec<u64>,
    timestamp: Vec<u32>,
    r: Vec<Option<Vec<u8>>>,
    s: Vec<Option<Vec<u8>>>,
    // The EIP-155 `v` scalar as it appears on the wire: `27`/`28` for an
    // unprotected legacy transaction, `chain_id * 2 + 35 + y_parity` for a
    // replay-protected one, and `0`/`1` for every typed (>= EIP-2930)
    // transaction.
    //
    // This column used to be `bool`, holding `Signature::v()` — which alloy
    // defines as the y-parity bit, not the `v` scalar. Every legacy row was
    // therefore mislabelled and the chain id embedded in `v` was unrecoverable.
    // Unsigned transaction types (OP deposits, Arbitrum internal transactions)
    // are now `None` rather than a fabricated `false`.
    v: Vec<Option<u64>>,
    // The y-parity bit on its own, which is what the old `v` column actually
    // contained. Kept as a distinct column so both readings are available.
    y_parity: Vec<Option<bool>>,
    // The chain id *the transaction commits to*, which is not the same thing as
    // the `chain_id` column: that one is the network the run pointed at. They
    // differ for unprotected pre-EIP-155 legacy transactions, where this is
    // `None`.
    tx_chain_id: Vec<Option<u64>>,
    // Which family defined this transaction's type byte: "ethereum",
    // "op_stack", "arbitrum" or "unknown". See `ChainFamily`.
    chain_family: Vec<String>,

    // --- EIP-2930 (Berlin) ------------------------------------------------
    // Access-list shape. `None` for legacy transactions, which have no list;
    // `Some(0)` for a typed transaction that carries an empty one.
    n_access_list_addresses: Vec<Option<u32>>,
    n_access_list_storage_keys: Vec<Option<u32>>,

    // --- EIP-4844 (Cancun) ------------------------------------------------
    max_fee_per_blob_gas: Vec<Option<U256>>,
    n_blob_versioned_hashes: Vec<Option<u32>>,
    // The versioned hashes concatenated, 32 bytes each, in commitment order.
    // This is the only join key from an L1 blob to the L2 batch it carries, so
    // it is kept verbatim rather than summarised.
    blob_versioned_hashes: Vec<Option<Vec<u8>>>,
    // From the receipt: blob gas this transaction consumed and the price it
    // paid per unit. `None` for non-blob transactions.
    blob_gas_used: Vec<Option<u64>>,
    blob_gas_price: Vec<Option<U256>>,

    // --- EIP-7702 (Prague) ------------------------------------------------
    n_authorizations: Vec<Option<u32>>,

    // --- OP stack ---------------------------------------------------------
    // Deposit-transaction fields (type 0x7e). See `chains::op`.
    source_hash: Vec<Option<Vec<u8>>>,
    mint: Vec<Option<U256>>,
    is_system_tx: Vec<Option<bool>>,
    deposit_receipt_version: Vec<Option<u64>>,
    // L1 data-availability cost, from the receipt. `l1_fee` is *not* included
    // in `gas_used * gas_price`; on an OP-stack chain it is usually the larger
    // of the two, so a fee analysis that ignores it is wrong, not imprecise.
    l1_fee: Vec<Option<U256>>,
    l1_gas_used: Vec<Option<u64>>,
    l1_gas_price: Vec<Option<u64>>,
    l1_fee_scalar: Vec<Option<f64>>,
    l1_blob_base_fee: Vec<Option<U256>>,
    l1_base_fee_scalar: Vec<Option<u64>>,
    l1_blob_base_fee_scalar: Vec<Option<u64>>,
    operator_fee_scalar: Vec<Option<u64>>,
    operator_fee_constant: Vec<Option<u64>>,

    // --- Arbitrum stack ---------------------------------------------------
    // Arbitrum folds the L1 data-availability charge *into* `gas_used`, so an
    // Arbitrum `gas_used` is not comparable to a mainnet one. These split it:
    // `gas_used_for_l2` is `gas_used - gas_used_for_l1`, i.e. execution gas.
    gas_used_for_l1: Vec<Option<u64>>,
    gas_used_for_l2: Vec<Option<u64>>,
    // Cross-domain join keys for retryables and L1-initiated transactions.
    request_id: Vec<Option<Vec<u8>>>,
    ticket_id: Vec<Option<Vec<u8>>>,
    refund_to: Vec<Option<Vec<u8>>>,
}

impl Dataset for Transactions {
    fn aliases() -> Vec<&'static str> {
        vec!["txs"]
    }

    fn default_columns() -> Option<Vec<&'static str>> {
        // Deliberately unchanged by the EIP / L2 work: every column added
        // above is opt-in via `--columns`, so an existing pipeline's output
        // schema is byte-identical after upgrading.
        Some(vec![
            "block_number",
            "transaction_index",
            "transaction_hash",
            "nonce",
            "from_address",
            "to_address",
            "value",
            "input",
            "gas_limit",
            "gas_used",
            "gas_price",
            "transaction_type",
            "max_priority_fee_per_gas",
            "max_fee_per_gas",
            "success",
            "n_input_bytes",
            "n_input_zero_bytes",
            "n_input_nonzero_bytes",
            "chain_id",
        ])
    }

    fn optional_parameters() -> Vec<Dim> {
        vec![Dim::FromAddress, Dim::ToAddress]
    }
}

/// tuple representing transaction and optional receipt
pub type TransactionAndReceipt = (RpcTransaction, Option<RpcReceipt>);

/// Columns that can only be filled from a receipt.
///
/// `success` and `gas_used` were the original two; the OP L1-fee family, the
/// Arbitrum L1-gas split and the EIP-4844 blob-gas pair are also receipt-only,
/// so asking for any of them has to trigger the receipt fetch. Missing an entry
/// here does not error — it silently yields a column of nulls.
const RECEIPT_BACKED_COLUMNS: &[&str] = &[
    "gas_used",
    "success",
    "gas_price",
    "blob_gas_used",
    "blob_gas_price",
    "l1_fee",
    "l1_gas_used",
    "l1_gas_price",
    "l1_fee_scalar",
    "l1_blob_base_fee",
    "l1_base_fee_scalar",
    "l1_blob_base_fee_scalar",
    "operator_fee_scalar",
    "operator_fee_constant",
    "gas_used_for_l1",
    "gas_used_for_l2",
];

fn needs_receipt(schema: &Table) -> bool {
    RECEIPT_BACKED_COLUMNS.iter().any(|column| schema.has_column(column))
}

impl CollectByBlock for Transactions {
    type Response = (RpcBlock, Vec<TransactionAndReceipt>, bool);

    async fn extract(request: Params, source: Arc<Source>, query: Arc<Query>) -> R<Self::Response> {
        let block = source
            .get_block(request.block_number()?, BlockTransactionsKind::Full)
            .await?
            .ok_or(CollectError::CollectError("block not found".to_string()))?;
        let schema = query.schemas.get_schema(&Datatype::Transactions)?;

        // 1. collect transactions and filter them if optional parameters are supplied
        // filter by from_address
        let from_filter: Box<dyn Fn(&RpcTransaction) -> bool + Send> =
            if let Some(from_address) = &request.from_address {
                Box::new(move |tx| tx.inner.inner.signer() == Address::from_slice(from_address))
            } else {
                Box::new(|_| true)
            };
        // filter by to_address
        let to_filter: Box<dyn Fn(&RpcTransaction) -> bool + Send> =
            if let Some(to_address) = &request.to_address {
                Box::new(move |tx| match tx.inner.inner.kind() {
                    TxKind::Create => false,
                    TxKind::Call(address) => address == Address::from_slice(to_address),
                })
            } else {
                Box::new(|_| true)
            };
        // A block fetched with `BlockTransactionsKind::Full` always carries
        // full bodies; a node that answers with hashes anyway is malformed, and
        // erroring beats unwrapping the whole worker task away.
        let transactions: Vec<RpcTransaction> = block
            .transactions
            .as_transactions()
            .ok_or_else(|| err("node returned transaction hashes for a full-block request"))?
            .iter()
            .filter(|&x| from_filter(x))
            .filter(|&x| to_filter(x))
            .cloned()
            .collect();

        // 2. collect receipts if necessary
        // if transactions are filtered fetch by set of transaction hashes, else fetch all receipts
        // in block
        let receipts: Vec<Option<_>> = if needs_receipt(schema) {
            // receipts required
            let receipts = if request.from_address.is_some() || request.to_address.is_some() {
                source.get_tx_receipts(BlockTransactions::Full(transactions.clone())).await?
            } else {
                source.get_tx_receipts_in_block(&block).await?
            };
            receipts.into_iter().map(Some).collect()
        } else {
            vec![None; transactions.len()]
        };

        let transactions_with_receips = transactions.into_iter().zip(receipts).collect();
        Ok((block, transactions_with_receips, query.exclude_failed))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Transactions)?;
        let (block, transactions_with_receipts, exclude_failed) = response;
        let timestamp = block.header.timestamp as u32;
        let base_fee_per_gas = block.header.base_fee_per_gas;
        for (tx, receipt) in transactions_with_receipts.into_iter() {
            process_transaction(
                tx,
                receipt,
                columns,
                schema,
                exclude_failed,
                timestamp,
                base_fee_per_gas,
            )?;
        }
        Ok(())
    }
}

impl CollectByTransaction for Transactions {
    type Response = (TransactionAndReceipt, RpcBlock, bool, u32);

    async fn extract(request: Params, source: Arc<Source>, query: Arc<Query>) -> R<Self::Response> {
        let tx_hash = request.ethers_transaction_hash()?;
        let schema = query.schemas.get_schema(&Datatype::Transactions)?;
        let transaction = source
            .get_transaction_by_hash(tx_hash)
            .await?
            .ok_or(CollectError::CollectError("transaction not found".to_string()))?;
        let receipt = if needs_receipt(schema) {
            source.get_transaction_receipt(tx_hash).await?
        } else {
            None
        };

        let block_number = transaction
            .block_number
            .ok_or(CollectError::CollectError("no block number for tx".to_string()))?;

        let block = source
            .get_block(block_number, BlockTransactionsKind::Hashes)
            .await?
            .ok_or(CollectError::CollectError("block not found".to_string()))?;

        let timestamp = block.header.timestamp as u32;

        Ok(((transaction, receipt), block, query.exclude_failed, timestamp))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Transactions)?;
        let ((transaction, receipt), block, exclude_failed, timestamp) = response;
        let base_fee_per_gas = block.header.base_fee_per_gas;
        process_transaction(
            transaction,
            receipt,
            columns,
            schema,
            exclude_failed,
            timestamp,
            base_fee_per_gas,
        )?;
        Ok(())
    }
}

pub(crate) fn process_transaction(
    tx: RpcTransaction,
    receipt: Option<RpcReceipt>,
    columns: &mut Transactions,
    schema: &Table,
    exclude_failed: bool,
    timestamp: u32,
    base_fee_per_gas: Option<u64>,
) -> R<()> {
    let success = if exclude_failed | schema.has_column("success") {
        let success = tx_success(&receipt)?;
        if exclude_failed & !success {
            return Ok(());
        }
        success
    } else {
        false
    };

    // Split the RPC wrapper from the transaction so the two extra-field maps
    // are addressable separately; see `TxExtras` for why both matter.
    let outer_fields = tx.0.other;
    let tx = tx.0.inner;
    let envelope = tx.inner.inner();
    let envelope_fields = match envelope {
        AnyTxEnvelope::Unknown(unknown) => Some(&unknown.inner.fields),
        AnyTxEnvelope::Ethereum(_) => None,
    };
    let extras = TxExtras::new(envelope_fields, &outer_fields);
    // A transaction with no receipt has no receipt-borne fields; an empty map
    // makes every reader below return `None` without a second code path.
    let no_receipt_fields = OtherFields::default();
    let receipt_fields: &OtherFields =
        receipt.as_ref().map(|r| &r.other).unwrap_or(&no_receipt_fields);

    let tx_type = envelope.ty();
    let family = ChainFamily::of_tx_type(tx_type);

    columns.n_rows += 1;
    store!(schema, columns, block_number, tx.block_number.map(|x| x as u32));
    store!(schema, columns, transaction_index, tx.transaction_index);
    // `trie_hash` is the one encoding-adjacent call that is safe on an unknown
    // type byte: it returns the hash the node reported instead of recomputing.
    store!(schema, columns, transaction_hash, envelope.trie_hash().to_vec());
    store!(schema, columns, from_address, tx.inner.signer().to_vec());
    store!(
        schema,
        columns,
        to_address,
        match envelope.kind() {
            TxKind::Create => None,
            TxKind::Call(address) => Some(address.to_vec()),
        }
    );
    store!(schema, columns, nonce, envelope.nonce());
    store!(schema, columns, value, envelope.value());
    store!(schema, columns, input, envelope.input().to_vec());
    store!(schema, columns, gas_limit, envelope.gas_limit());
    store!(schema, columns, success, success);
    if schema.has_column("n_input_bytes") |
        schema.has_column("n_input_zero_bytes") |
        schema.has_column("n_input_nonzero_bytes")
    {
        let n_input_bytes = envelope.input().len() as u32;
        let n_input_zero_bytes = envelope.input().iter().filter(|&&x| x == 0).count() as u32;
        store!(schema, columns, n_input_bytes, n_input_bytes);
        store!(schema, columns, n_input_zero_bytes, n_input_zero_bytes);
        store!(schema, columns, n_input_nonzero_bytes, n_input_bytes - n_input_zero_bytes);
    }
    // in alloy eip2718_encoded_length is rlp_encoded_length. It panics on any
    // type byte alloy cannot re-encode, so ask first and report null otherwise.
    store!(
        schema,
        columns,
        n_rlp_bytes,
        is_reencodable(envelope).then(|| envelope.encode_2718_len() as u32)
    );
    store!(schema, columns, gas_used, receipt.as_ref().map(|r| r.gas_used));
    store!(
        schema,
        columns,
        gas_price,
        effective_gas_price(&tx, receipt.as_ref(), base_fee_per_gas)
    );
    store!(schema, columns, transaction_type, tx_type as u32);
    store!(schema, columns, max_fee_per_gas, get_max_fee_per_gas(envelope));
    store!(
        schema,
        columns,
        max_priority_fee_per_gas,
        envelope.max_priority_fee_per_gas().and_then(|value| u64::try_from(value).ok())
    );
    store!(schema, columns, timestamp, timestamp);
    store!(schema, columns, block_hash, tx.block_hash.unwrap_or_default().to_vec());

    let signature = match envelope {
        AnyTxEnvelope::Ethereum(inner) => Some(*inner.signature()),
        // Unsigned types (OP deposits, Arbitrum internal transactions) report
        // r = s = v = 0. Storing those zeros would claim a signature exists.
        AnyTxEnvelope::Unknown(_) => None,
    };
    store!(schema, columns, r, signature.map(|sig| sig.r().to_vec_u8()));
    store!(schema, columns, s, signature.map(|sig| sig.s().to_vec_u8()));
    store!(schema, columns, y_parity, signature.map(|sig| sig.v()));
    store!(
        schema,
        columns,
        v,
        signature.map(|sig| v_scalar(tx_type, envelope.chain_id(), sig.v()))
    );
    store!(schema, columns, tx_chain_id, envelope.chain_id());
    store!(schema, columns, chain_family, family.as_str().to_string());

    // EIP-2930. `access_list()` is `None` for legacy transactions and `Some`
    // (possibly empty) for every typed one, so the null carries information.
    let access_list = envelope.access_list();
    store!(schema, columns, n_access_list_addresses, access_list.map(|list| list.len() as u32));
    store!(
        schema,
        columns,
        n_access_list_storage_keys,
        access_list
            .map(|list| list.iter().map(|item| item.storage_keys.len()).sum::<usize>() as u32)
    );

    // EIP-4844.
    store!(schema, columns, max_fee_per_blob_gas, envelope.max_fee_per_blob_gas().map(U256::from));
    let blob_hashes = envelope.blob_versioned_hashes();
    store!(schema, columns, n_blob_versioned_hashes, blob_hashes.map(|hashes| hashes.len() as u32));
    store!(
        schema,
        columns,
        blob_versioned_hashes,
        blob_hashes.map(|hashes| hashes.iter().flat_map(|hash| hash.0).collect::<Vec<u8>>())
    );
    store!(schema, columns, blob_gas_used, receipt.as_ref().and_then(|r| r.blob_gas_used));
    store!(
        schema,
        columns,
        blob_gas_price,
        receipt.as_ref().and_then(|r| r.blob_gas_price).map(U256::from)
    );

    // EIP-7702.
    store!(
        schema,
        columns,
        n_authorizations,
        envelope.authorization_list().map(|list| list.len() as u32)
    );

    // OP stack: deposit body, then the L1-fee family off the receipt.
    store!(schema, columns, source_hash, extras.bytes(op::SOURCE_HASH));
    store!(schema, columns, mint, extras.u256(op::MINT));
    store!(schema, columns, is_system_tx, extras.bool(op::IS_SYSTEM_TX));
    store!(schema, columns, deposit_receipt_version, extras.u64(op::DEPOSIT_RECEIPT_VERSION));
    store!(schema, columns, l1_fee, other_u256(receipt_fields, op::L1_FEE));
    store!(schema, columns, l1_gas_used, other_u64(receipt_fields, op::L1_GAS_USED));
    store!(schema, columns, l1_gas_price, other_u64(receipt_fields, op::L1_GAS_PRICE));
    store!(schema, columns, l1_fee_scalar, other_decimal_f64(receipt_fields, op::L1_FEE_SCALAR));
    store!(schema, columns, l1_blob_base_fee, other_u256(receipt_fields, op::L1_BLOB_BASE_FEE));
    store!(schema, columns, l1_base_fee_scalar, other_u64(receipt_fields, op::L1_BASE_FEE_SCALAR));
    store!(
        schema,
        columns,
        l1_blob_base_fee_scalar,
        other_u64(receipt_fields, op::L1_BLOB_BASE_FEE_SCALAR)
    );
    store!(
        schema,
        columns,
        operator_fee_scalar,
        other_u64(receipt_fields, op::OPERATOR_FEE_SCALAR)
    );
    store!(
        schema,
        columns,
        operator_fee_constant,
        other_u64(receipt_fields, op::OPERATOR_FEE_CONSTANT)
    );

    // Arbitrum: split `gas_used` into its DA and execution halves.
    let gas_used_for_l1 = other_u64(receipt_fields, arbitrum::GAS_USED_FOR_L1);
    store!(schema, columns, gas_used_for_l1, gas_used_for_l1);
    store!(
        schema,
        columns,
        gas_used_for_l2,
        // `checked_sub`, not `-`: a node that reports an L1 share larger than
        // the total is describing something we do not model, and a wrapped
        // 18-quintillion answer would be worse than an admitted null.
        gas_used_for_l1
            .zip(receipt.as_ref().map(|r| r.gas_used))
            .and_then(|(l1, total)| total.checked_sub(l1))
    );
    store!(schema, columns, request_id, extras.bytes(arbitrum::REQUEST_ID));
    store!(schema, columns, ticket_id, extras.bytes(arbitrum::TICKET_ID));
    store!(schema, columns, refund_to, extras.bytes(arbitrum::REFUND_TO));

    Ok(())
}

/// Reconstruct the EIP-155 `v` scalar from the parity bit alloy exposes.
///
/// Typed transactions (EIP-2930 onwards) put the parity bit itself on the wire,
/// so `v` is `0` or `1`. Legacy transactions encode it as `27 + parity` when
/// unprotected, and `chain_id * 2 + 35 + parity` once EIP-155 folded the chain
/// id in — which is why a legacy `v` is the only place the transaction's chain
/// id is recorded.
///
/// The multiply saturates rather than wrapping: a chain id above
/// `(u64::MAX - 36) / 2` cannot be represented here, and clamping is visibly
/// wrong where a wrap would look like a small, plausible chain.
fn v_scalar(tx_type: u8, chain_id: Option<u64>, y_parity: bool) -> u64 {
    if tx_type != 0 {
        return y_parity as u64
    }
    match chain_id {
        Some(id) => id.saturating_mul(2).saturating_add(35).saturating_add(y_parity as u64),
        None => 27 + y_parity as u64,
    }
}

/// The price actually paid per unit of gas, in wei.
///
/// Three sources, in descending order of authority:
///
/// 1. the receipt's `effectiveGasPrice`, which the node computed post-execution;
/// 2. the transaction's own `gasPrice`, which is that same number for legacy and EIP-2930
///    transactions and, on most nodes, for typed ones too;
/// 3. the EIP-1559 formula, as a last resort when neither is present.
///
/// The previous implementation went straight to (3) with three unguarded
/// operations — `base_fee_per_gas.unwrap()`, `max_priority_fee_per_gas().unwrap()`
/// and an unchecked `max_fee - base_fee` subtraction — so a typed transaction in
/// a block without a base fee (every OP deposit, every pre-London block) panicked
/// the worker task, and a max fee below the base fee underflowed.
fn effective_gas_price(
    tx: &alloy::rpc::types::Transaction<AnyTxEnvelope>,
    receipt: Option<&RpcReceipt>,
    base_fee_per_gas: Option<u64>,
) -> Option<u64> {
    let price = receipt
        .map(|r| r.effective_gas_price)
        .or(tx.effective_gas_price)
        .or_else(|| Some(tx.inner.inner().effective_gas_price(base_fee_per_gas)))?;
    // Saturating here would invent a ceiling price; a value that does not fit
    // is one we cannot report, so say so.
    u64::try_from(price).ok()
}

fn get_max_fee_per_gas(envelope: &AnyTxEnvelope) -> Option<u64> {
    // Legacy and EIP-2930 transactions have no max fee; alloy's trait method
    // returns the gas price for them, which would silently duplicate a
    // different concept into this column.
    if !envelope.is_dynamic_fee() {
        return None
    }
    u64::try_from(envelope.max_fee_per_gas()).ok()
}

/// Whether the transaction succeeded, per its receipt.
///
/// Pre-Byzantium receipts carry a state root instead of a status, and alloy
/// reports those as `status() == false`. triodion has never handled that case
/// correctly; it now says so instead of guessing, which is the same rule the
/// rest of the codebase follows — a value we cannot measure is not written.
fn tx_success(receipt: &Option<RpcReceipt>) -> R<bool> {
    match receipt {
        Some(r) => Ok(r.status()),
        None => Err(err("could not determine status of transaction: no receipt")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        chains::{arbitrum, op},
        wire_fixtures::{arbitrum_one_block, op_mainnet_block},
    };

    fn parse(json: serde_json::Value) -> RpcBlock {
        serde_json::from_value(json).expect("block fixture deserializes")
    }

    /// Every column except `success`, which cannot be answered without a
    /// receipt — see [`tx_success`]. These tests exercise the transaction body,
    /// so they deliberately do not supply one.
    fn schema() -> Table {
        let columns: Vec<String> = Datatype::Transactions
            .column_types()
            .keys()
            .filter(|name| **name != "success")
            .map(|name| name.to_string())
            .collect();
        Datatype::Transactions
            .table_schema(
                &[U256Type::String],
                &ColumnEncoding::Hex,
                &None,
                &None,
                &Some(columns),
                None,
                None,
            )
            .expect("every column is nameable")
    }

    fn collect(block: &RpcBlock) -> Transactions {
        let schema = schema();
        let mut columns = Transactions::default();
        let timestamp = block.header.timestamp as u32;
        let base_fee = block.header.base_fee_per_gas;
        for tx in block.transactions.as_transactions().expect("fixture has full bodies") {
            process_transaction(
                tx.clone(),
                None,
                &mut columns,
                &schema,
                false,
                timestamp,
                base_fee,
            )
            .expect("a transaction with no receipt still yields a row");
        }
        columns
    }

    #[test]
    fn an_op_stack_block_deserializes_instead_of_failing_whole() {
        // THE regression. Before switching to `AnyNetwork`, the `0x7e` deposit
        // made the *entire* response fail with "data did not match any variant
        // of untagged enum BlockTransactions" — so `transactions` collected
        // zero rows on OP Mainnet, Base and every other OP-stack chain, not
        // merely missing columns.
        let block = parse(op_mainnet_block());
        let txs = block.transactions.as_transactions().expect("full bodies");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].inner.inner.inner().ty(), op::DEPOSIT);
    }

    #[test]
    fn an_arbitrum_block_deserializes_instead_of_failing_whole() {
        // Same failure, different type byte: every Arbitrum block opens with an
        // `ArbitrumInternalTx` (0x6a).
        let block = parse(arbitrum_one_block());
        let txs = block.transactions.as_transactions().expect("full bodies");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].inner.inner.inner().ty(), arbitrum::INTERNAL);
    }

    #[test]
    fn an_op_deposit_keeps_its_deposit_only_fields() {
        let columns = collect(&parse(op_mainnet_block()));
        assert_eq!(columns.n_rows, 2);
        assert_eq!(columns.chain_family, vec!["op_stack", "ethereum"]);
        assert_eq!(
            columns.source_hash[0].as_ref().map(alloy::hex::encode),
            Some("883aa371d61056b40ec30d9b74257103351b24d70eb8063940f19f7510e39799".to_string())
        );
        assert_eq!(columns.mint[0], Some(U256::ZERO));
        assert_eq!(columns.deposit_receipt_version[0], Some(1));
        // The 1559 transaction beside it has none of these.
        assert_eq!(columns.source_hash[1], None);
        assert_eq!(columns.deposit_receipt_version[1], None);
    }

    #[test]
    fn an_unsigned_transaction_reports_no_signature_rather_than_zeros() {
        // OP deposits and Arbitrum internal transactions carry r = s = v = 0 on
        // the wire because they are not signed at all. Writing those zeros into
        // the signature columns would claim a signature that does not exist —
        // the same class of invented value as the `erc20_supplies` nulls.
        let op = collect(&parse(op_mainnet_block()));
        assert_eq!(op.r[0], None);
        assert_eq!(op.s[0], None);
        assert_eq!(op.v[0], None);
        assert_eq!(op.y_parity[0], None);
        // The signed transaction in the same block still reports one.
        assert!(op.r[1].is_some());
        assert_eq!(op.y_parity[1], Some(false));

        let arb = collect(&parse(arbitrum_one_block()));
        assert_eq!(arb.v[0], None);
    }

    #[test]
    fn a_non_reencodable_transaction_reports_no_rlp_length_instead_of_panicking() {
        // `encode_2718_len` panics inside alloy for any type byte it cannot
        // re-encode, which is every OP-stack and Arbitrum-stack type. Reaching
        // this assertion at all is the test: an unguarded call would abort the
        // worker task and take the whole chunk with it.
        let op = collect(&parse(op_mainnet_block()));
        assert_eq!(op.n_rlp_bytes[0], None, "a deposit has no re-encodable length");
        assert!(op.n_rlp_bytes[1].is_some(), "the 1559 transaction beside it does");

        let arb = collect(&parse(arbitrum_one_block()));
        assert_eq!(arb.n_rlp_bytes[0], None);
    }

    #[test]
    fn shared_columns_are_read_the_same_way_on_every_family() {
        // The point of `AnyNetwork`: nonce, value, input and gas come off an
        // unknown envelope exactly as they do off a known one.
        let arb = collect(&parse(arbitrum_one_block()));
        assert_eq!(arb.n_rows, 1);
        assert_eq!(arb.chain_family, vec!["arbitrum"]);
        assert_eq!(arb.nonce[0], 0);
        assert_eq!(arb.value[0], U256::ZERO);
        assert_eq!(arb.n_input_bytes[0], 132);
        assert_eq!(arb.tx_chain_id[0], Some(42161));
    }

    #[test]
    fn a_legacy_v_carries_the_chain_id_and_a_typed_v_does_not() {
        // EIP-155 folds the chain id into the legacy `v` scalar. Storing
        // alloy's `Signature::v()` (the y-parity bool) in a column named `v`
        // discarded it for every legacy row.
        assert_eq!(v_scalar(0, Some(1), true), 38);
        assert_eq!(v_scalar(0, Some(1), false), 37);
        assert_eq!(v_scalar(0, Some(42161), false), 84357);
        // Pre-EIP-155, unprotected.
        assert_eq!(v_scalar(0, None, false), 27);
        assert_eq!(v_scalar(0, None, true), 28);
        // Typed transactions put the parity bit itself on the wire.
        assert_eq!(v_scalar(2, Some(1), true), 1);
        assert_eq!(v_scalar(3, Some(1), false), 0);
    }

    #[test]
    fn an_absurd_chain_id_saturates_rather_than_wrapping_into_a_plausible_one() {
        // `chain_id * 2 + 35` overflows u64 above ~9.2e18. A wrap would produce
        // a small, believable `v`; saturation produces a visibly impossible one.
        assert_eq!(v_scalar(0, Some(u64::MAX), true), u64::MAX);
    }
}
