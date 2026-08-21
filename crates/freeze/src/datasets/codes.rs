use crate::*;
use alloy::primitives::Address;
use polars::prelude::*;

/// columns for codes
///
/// Since EIP-7702, code at an address no longer means the address is a
/// contract. A delegation designator is 23 bytes — `0xef0100` followed by an
/// address — written to an *externally owned account* by an authorization, and
/// it makes calls to that account execute the named contract's code instead.
/// Anything that classified accounts as "contract if code is non-empty" now
/// counts delegated EOAs as contracts. `is_delegated` and `delegate_address`
/// make that case visible; see the `authorizations` dataset for the
/// authorization tuples that cause it.
#[triodion_macros::to_df(Datatype::Codes)]
#[derive(Default)]
pub struct Codes {
    n_rows: usize,
    block_number: Vec<u32>,
    address: Vec<Vec<u8>>,
    code: Vec<Vec<u8>>,
    // EIP-7702: the code is a 23-byte delegation designator, so this address
    // is a delegated EOA rather than a contract.
    is_delegated: Vec<bool>,
    // The contract whose code runs on calls to this address. Null whenever
    // `is_delegated` is false — an ordinary contract delegates to nothing.
    delegate_address: Vec<Option<Vec<u8>>>,
    chain_id: Vec<u64>,
}

impl Dataset for Codes {
    // Stated explicitly rather than left to "every column", so that the two
    // EIP-7702 columns added above do not change the output of a command
    // anybody is already running.
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "address", "code", "chain_id"])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "address"])
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

type BlockTxAddressOutput = (u32, Option<Vec<u8>>, Vec<u8>, Vec<u8>);

impl CollectByBlock for Codes {
    type Response = BlockTxAddressOutput;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let address = request.address()?;
        let block_number = request.block_number()? as u32;
        let output = source.get_code(Address::from_slice(&address), block_number.into()).await?;
        Ok((block_number, None, address, output.to_vec()))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Codes)?;
        process_code(columns, response, schema)
    }
}

impl CollectByTransaction for Codes {
    type Response = ();
}

fn process_code(columns: &mut Codes, data: BlockTxAddressOutput, schema: &Table) -> R<()> {
    let (block, _tx, address, output) = data;
    let delegate = delegate_address(&output);
    columns.n_rows += 1;
    store!(schema, columns, block_number, block);
    store!(schema, columns, address, address);
    store!(schema, columns, is_delegated, delegate.is_some());
    store!(schema, columns, delegate_address, delegate);
    store!(schema, columns, code, output);
    Ok(())
}

/// The address inside an EIP-7702 delegation designator, if this is one.
///
/// The designator is exactly `0xef0100` followed by 20 bytes. The length check
/// is not decoration: `0xef` alone has been an invalid opcode since EIP-3541
/// reserved it, so a longer body starting with the same prefix is not a
/// designator and its trailing bytes are not an address.
fn delegate_address(code: &[u8]) -> Option<Vec<u8>> {
    const DESIGNATOR: [u8; 3] = [0xef, 0x01, 0x00];
    (code.len() == 23 && code[..3] == DESIGNATOR).then(|| code[3..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_designator_yields_the_delegate_address() {
        let mut code = vec![0xef, 0x01, 0x00];
        code.extend_from_slice(&[0xcd; 20]);
        assert_eq!(delegate_address(&code), Some(vec![0xcd; 20]));
    }

    #[test]
    fn ordinary_code_and_near_misses_yield_nothing() {
        assert_eq!(delegate_address(&[]), None, "an EOA has no code");
        assert_eq!(delegate_address(&[0x60, 0x80, 0x60, 0x40]), None, "ordinary contract code");
        // Right prefix, wrong length: not a designator, and its tail is not an
        // address.
        let mut too_long = vec![0xef, 0x01, 0x00];
        too_long.extend_from_slice(&[0xcd; 21]);
        assert_eq!(delegate_address(&too_long), None);
        // Right length, wrong prefix.
        let mut wrong_prefix = vec![0xef, 0x02, 0x00];
        wrong_prefix.extend_from_slice(&[0xcd; 20]);
        assert_eq!(delegate_address(&wrong_prefix), None);
    }
}
