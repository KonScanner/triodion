use crate::*;
use alloy::{
    consensus::{Transaction as ConsensusTransaction, Typed2718},
    eips::eip2718::Encodable2718,
    rpc::types::BlockTransactionsKind,
};
use polars::prelude::*;

/// columns for access_lists
///
/// One row per EIP-2930 access-list entry, exploded to one row per storage key.
///
/// [`Transactions`] carries `n_access_list_addresses` and
/// `n_access_list_storage_keys`. Those are counts: they say how large a list
/// was, never which accounts or slots it named. This dataset keeps the entries.
///
/// An access list pre-warms accounts and storage slots so that touching them
/// costs less gas. It is a declaration made before execution, not a record of
/// what execution did — a listed slot may never be read, and a slot the
/// transaction does read may be absent from the list. For what was actually
/// touched, use `storage_reads` and `balance_reads`.
#[triodion_macros::to_df(Datatype::AccessLists)]
#[derive(Default)]
pub struct AccessLists {
    n_rows: u64,
    block_number: Vec<u32>,
    transaction_index: Vec<Option<u64>>,
    transaction_hash: Vec<Vec<u8>>,
    transaction_type: Vec<u32>,
    // Position of the account entry within the transaction's access list.
    entry_index: Vec<u32>,
    address: Vec<Vec<u8>>,
    // Position of the key within this entry's own key list. Null when the
    // entry names no keys at all — see `storage_key`.
    storage_key_index: Vec<Option<u32>>,
    // Null means the entry warmed the account and listed no slots, which
    // EIP-2930 permits. It does not mean "unknown".
    storage_key: Vec<Option<Vec<u8>>>,
    chain_id: Vec<u64>,
}

impl Dataset for AccessLists {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "transaction_index",
            "transaction_hash",
            "entry_index",
            "address",
            "storage_key_index",
            "storage_key",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "transaction_index", "entry_index", "storage_key_index"])
    }
}

impl CollectByBlock for AccessLists {
    type Response = RpcBlock;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        source
            .get_block(request.block_number()?, BlockTransactionsKind::Full)
            .await?
            .ok_or_else(|| err("block not found"))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::AccessLists)?;
        let transactions = response
            .transactions
            .as_transactions()
            .ok_or_else(|| err("node returned transaction hashes for a full-block request"))?;
        let block_number = response.header.number as u32;
        for tx in transactions {
            process_access_list(tx, block_number, columns, schema);
        }
        Ok(())
    }
}

impl CollectByTransaction for AccessLists {
    type Response = (RpcTransaction, u32);

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let tx_hash = request.ethers_transaction_hash()?;
        let transaction = source
            .get_transaction_by_hash(tx_hash)
            .await?
            .ok_or_else(|| err("transaction not found"))?;
        let block_number =
            transaction.block_number.ok_or_else(|| err("no block number for tx"))? as u32;
        Ok((transaction, block_number))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::AccessLists)?;
        let (transaction, block_number) = response;
        process_access_list(&transaction, block_number, columns, schema);
        Ok(())
    }
}

/// Explode one transaction's access list into rows.
fn process_access_list(
    tx: &RpcTransaction,
    block_number: u32,
    columns: &mut AccessLists,
    schema: &Table,
) {
    let envelope = tx.inner.inner.inner();
    // `None` for a legacy transaction, which has no access list at all.
    // `Some(empty)` for a typed transaction that declared none. Both produce no
    // rows here, and `transactions.n_access_list_addresses` keeps the
    // distinction for anyone who needs it.
    let Some(access_list) = envelope.access_list() else { return };
    let transaction_hash = envelope.trie_hash().to_vec();
    let transaction_index = tx.inner.transaction_index;
    let transaction_type = envelope.ty() as u32;

    for (entry_index, entry) in access_list.iter().enumerate() {
        // An entry with no storage keys still warms the account, so it is a
        // real entry and gets a row. Dropping it would lose the gas it paid for.
        if entry.storage_keys.is_empty() {
            push_row(
                columns,
                schema,
                block_number,
                &transaction_hash,
                transaction_index,
                transaction_type,
                entry_index as u32,
                &entry.address,
                None,
                None,
            );
            continue
        }
        for (key_index, key) in entry.storage_keys.iter().enumerate() {
            push_row(
                columns,
                schema,
                block_number,
                &transaction_hash,
                transaction_index,
                transaction_type,
                entry_index as u32,
                &entry.address,
                Some(key_index as u32),
                Some(key.to_vec()),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_row(
    columns: &mut AccessLists,
    schema: &Table,
    block_number: u32,
    transaction_hash: &[u8],
    transaction_index: Option<u64>,
    transaction_type: u32,
    entry_index: u32,
    address: &alloy::primitives::Address,
    storage_key_index: Option<u32>,
    storage_key: Option<Vec<u8>>,
) {
    columns.n_rows += 1;
    store!(schema, columns, block_number, block_number);
    store!(schema, columns, transaction_index, transaction_index);
    store!(schema, columns, transaction_hash, transaction_hash.to_vec());
    store!(schema, columns, transaction_type, transaction_type);
    store!(schema, columns, entry_index, entry_index);
    store!(schema, columns, address, address.to_vec());
    store!(schema, columns, storage_key_index, storage_key_index);
    store!(schema, columns, storage_key, storage_key);
}
