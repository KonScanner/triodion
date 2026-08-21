use crate::*;
use alloy::{
    consensus::Transaction as ConsensusTransaction, eips::eip2718::Encodable2718,
    rpc::types::BlockTransactionsKind,
};
use polars::prelude::*;

/// columns for authorizations
///
/// One row per EIP-7702 authorization tuple.
///
/// [`Transactions`] carries `n_authorizations`, which is a count. It says a
/// type-`0x04` transaction carried three authorizations; it does not say which
/// accounts delegated, or to what code. This dataset keeps the tuples.
///
/// Two cautions, both of which the columns are shaped around:
///
/// - An authorization in a block is a *submitted* authorization. The protocol applies it only if
///   the nonce and chain id still match at execution time, and a stale one is skipped while the
///   transaction still succeeds. Nothing in the transaction's own data records which of the two
///   happened, so this dataset does not claim to. Compare `nonce` against the authority's nonce in
///   `nonce_diffs` if you need applied-versus-submitted.
/// - `authority` is recovered from the signature, not stated in the payload. A malformed signature
///   yields no address rather than a wrong one.
#[triodion_macros::to_df(Datatype::Authorizations)]
#[derive(Default)]
pub struct Authorizations {
    n_rows: u64,
    block_number: Vec<u32>,
    transaction_index: Vec<Option<u64>>,
    transaction_hash: Vec<Vec<u8>>,
    // Position within this transaction's authorization list. Order matters:
    // the protocol applies them in order, so a later tuple can overwrite an
    // earlier one for the same authority.
    authorization_index: Vec<u32>,
    // The account being delegated, recovered from the signature. Null when
    // recovery fails — a signature that no key could have produced is a fact
    // about the row, not a reason to invent an address.
    authority: Vec<Option<Vec<u8>>>,
    // The code the authority delegates to. The zero address is not a null: it
    // is the defined way to clear an existing delegation.
    delegate_address: Vec<Vec<u8>>,
    // The authorization's own chain id, which is not the transaction's. Zero
    // is meaningful and specified: it authorizes on every chain. Null only
    // when the value does not fit in 64 bits, which no real chain id does.
    authorization_chain_id: Vec<Option<u64>>,
    // The authority's expected nonce, not the transaction sender's.
    nonce: Vec<u64>,
    // Null when the node reported a parity outside {0, 1}. Every authorization
    // carries one, so null here means malformed rather than absent.
    y_parity: Vec<Option<bool>>,
    r: Vec<Vec<u8>>,
    s: Vec<Vec<u8>>,
    chain_id: Vec<u64>,
}

impl Dataset for Authorizations {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "transaction_index",
            "transaction_hash",
            "authorization_index",
            "authority",
            "delegate_address",
            "authorization_chain_id",
            "nonce",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "transaction_index", "authorization_index"])
    }
}

impl CollectByBlock for Authorizations {
    type Response = RpcBlock;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        source
            .get_block(request.block_number()?, BlockTransactionsKind::Full)
            .await?
            .ok_or_else(|| err("block not found"))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Authorizations)?;
        let transactions = response
            .transactions
            .as_transactions()
            .ok_or_else(|| err("node returned transaction hashes for a full-block request"))?;
        let block_number = response.header.number as u32;
        for tx in transactions {
            process_authorizations(tx, block_number, columns, schema);
        }
        Ok(())
    }
}

impl CollectByTransaction for Authorizations {
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
        let schema = query.schemas.get_schema(&Datatype::Authorizations)?;
        let (transaction, block_number) = response;
        process_authorizations(&transaction, block_number, columns, schema);
        Ok(())
    }
}

/// Explode one transaction's authorization list into rows.
fn process_authorizations(
    tx: &RpcTransaction,
    block_number: u32,
    columns: &mut Authorizations,
    schema: &Table,
) {
    let envelope = tx.inner.inner.inner();
    // `None` for every transaction type other than 0x04.
    let Some(authorizations) = envelope.authorization_list() else { return };
    let transaction_hash = envelope.trie_hash().to_vec();
    let transaction_index = tx.inner.transaction_index;

    for (index, authorization) in authorizations.iter().enumerate() {
        columns.n_rows += 1;
        store!(schema, columns, block_number, block_number);
        store!(schema, columns, transaction_index, transaction_index);
        store!(schema, columns, transaction_hash, transaction_hash.clone());
        store!(schema, columns, authorization_index, index as u32);
        store!(
            schema,
            columns,
            authority,
            authorization.recover_authority().ok().map(|address| address.to_vec())
        );
        store!(schema, columns, delegate_address, authorization.address.to_vec());
        store!(schema, columns, authorization_chain_id, u64::try_from(authorization.chain_id).ok());
        store!(schema, columns, nonce, authorization.nonce);
        store!(schema, columns, y_parity, parity_bit(authorization.y_parity()));
        store!(schema, columns, r, authorization.r().to_vec_u8());
        store!(schema, columns, s, authorization.s().to_vec_u8());
    }
}

/// Signature parity as a bit, or nothing if it is neither 0 nor 1.
///
/// The wire type is a `u8` and a node can put anything in it. Coercing an out
/// of range value to `true` would report a signature the sender never made.
const fn parity_bit(y_parity: u8) -> Option<bool> {
    match y_parity {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parity_outside_zero_and_one_is_null_rather_than_true() {
        assert_eq!(parity_bit(0), Some(false));
        assert_eq!(parity_bit(1), Some(true));
        assert_eq!(parity_bit(2), None);
        assert_eq!(parity_bit(27), None);
    }
}
