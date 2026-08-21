use crate::*;
use polars::prelude::*;

/// columns for deposit_requests
///
/// One row per EIP-6110 deposit request, from Prague onward.
///
/// [`Blocks`] carries `requests_hash`, which is a commitment over all of a
/// block's execution requests and cannot be turned back into any of them. The
/// execution layer never serves the bodies — `eth_getBlockByNumber` returns
/// the hash and nothing else — so this dataset reads the consensus block and
/// needs `--beacon-rpc`.
///
/// EIP-6110 changed how a deposit reaches the beacon chain, not what a deposit
/// is. Before Prague the consensus layer discovered deposits by voting on
/// execution state; from Prague the block carries them directly. Both forms
/// originate in the same deposit-contract call, so for history before Prague,
/// read the contract's `DepositEvent` through `logs` instead.
#[triodion_macros::to_df(Datatype::DepositRequests)]
#[derive(Default)]
pub struct DepositRequests {
    n_rows: u64,
    block_number: Vec<u32>,
    timestamp: Vec<u32>,
    slot: Vec<u64>,
    epoch: Vec<Option<u64>>,
    proposer_index: Vec<u64>,
    // Position within this block's deposit list.
    request_index: Vec<u32>,
    // BLS public key of the validator being deposited to, 48 bytes.
    pubkey: Vec<Vec<u8>>,
    // 32 bytes. The first byte states the kind, and the rest is only an
    // address for kinds 0x01 and 0x02.
    withdrawal_credentials: Vec<Vec<u8>>,
    // 0x00 BLS, 0x01 execution address, 0x02 compounding. Kept as its own
    // column because the difference decides whether `withdrawal_address` means
    // anything.
    withdrawal_credentials_type: Vec<u32>,
    // The last 20 bytes of the credentials, for kinds 0x01 and 0x02 only.
    // Null for 0x00, where those bytes are part of a BLS key and are not an
    // address at all. This is the join key to `withdrawals.address`.
    withdrawal_address: Vec<Option<Vec<u8>>>,
    // Gwei, as the protocol states it.
    amount_gwei: Vec<u64>,
    // BLS signature over the deposit message, 96 bytes. Opt-in: it is large
    // and only a validity proof.
    signature: Vec<Vec<u8>>,
    // Position in the deposit contract's own global sequence, which is what
    // makes a deposit identifiable across the whole chain.
    deposit_index: Vec<u64>,
    chain_id: Vec<u64>,
}

impl Dataset for DepositRequests {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "timestamp",
            "slot",
            "request_index",
            "pubkey",
            "withdrawal_credentials_type",
            "withdrawal_address",
            "amount_gwei",
            "deposit_index",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "request_index"])
    }
}

impl CollectByBlock for DepositRequests {
    type Response = beacon_requests::BlockRequests;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        beacon_requests::fetch(request, source).await
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::DepositRequests)?;
        let Some(requests) = response.requests.as_ref() else { return Ok(()) };
        for (index, deposit) in requests.deposits.iter().enumerate() {
            columns.n_rows += 1;
            store!(schema, columns, block_number, response.block_number);
            store!(schema, columns, timestamp, response.timestamp);
            store!(schema, columns, slot, response.slot);
            store!(schema, columns, epoch, response.epoch);
            store!(schema, columns, proposer_index, response.proposer_index);
            store!(schema, columns, request_index, index as u32);
            store!(schema, columns, pubkey, deposit.pubkey.clone());
            store!(schema, columns, withdrawal_credentials, deposit.withdrawal_credentials.clone());
            store!(
                schema,
                columns,
                withdrawal_credentials_type,
                deposit.withdrawal_credentials.first().copied().unwrap_or_default() as u32
            );
            store!(
                schema,
                columns,
                withdrawal_address,
                withdrawal_address(&deposit.withdrawal_credentials)
            );
            store!(schema, columns, amount_gwei, deposit.amount);
            store!(schema, columns, signature, deposit.signature.clone());
            store!(schema, columns, deposit_index, deposit.index);
        }
        Ok(())
    }
}

impl CollectByTransaction for DepositRequests {
    type Response = ();
}

/// The execution address inside a set of withdrawal credentials.
///
/// Only kinds `0x01` and `0x02` hold one. For `0x00` the remaining bytes are a
/// hashed BLS key, and reading 20 of them as an address would produce a
/// well-formed address that belongs to nobody.
fn withdrawal_address(credentials: &[u8]) -> Option<Vec<u8>> {
    if credentials.len() != 32 {
        return None
    }
    match credentials[0] {
        0x01 | 0x02 => Some(credentials[12..].to_vec()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_execution_credentials_yield_an_address() {
        let mut credentials = vec![0u8; 32];
        credentials[12..].copy_from_slice(&[0xab; 20]);

        credentials[0] = 0x00;
        assert_eq!(
            withdrawal_address(&credentials),
            None,
            "0x00 is a hashed BLS key, not an address"
        );
        credentials[0] = 0x01;
        assert_eq!(withdrawal_address(&credentials), Some(vec![0xab; 20]));
        credentials[0] = 0x02;
        assert_eq!(withdrawal_address(&credentials), Some(vec![0xab; 20]));
    }

    #[test]
    fn credentials_of_the_wrong_length_yield_no_address() {
        assert_eq!(withdrawal_address(&[0x01; 20]), None);
        assert_eq!(withdrawal_address(&[]), None);
    }
}
