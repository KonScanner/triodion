//! Wire-format block fixtures for the chain families triodion supports.
//!
//! These are real `eth_getBlockByNumber` responses, reduced to the fields the
//! tests read and rebuilt with `serde_json::json!`. They are deliberately Rust
//! source rather than checked-in `.json`: the repository does not carry data
//! files, and a reviewer can see which field came from which chain without
//! opening a second file.
//!
//! Every value below is verbatim from the named block, except where a comment
//! says otherwise. Boilerplate the tests never assert on — the four roots, the
//! logs bloom — is filled with zeroes, because the deserializer requires the
//! keys to be present but nothing reads them.

use serde_json::{json, Value};

fn zeros(bytes: usize) -> String {
    format!("0x{}", "00".repeat(bytes))
}

/// Header keys every EVM chain sends and no test here asserts on.
fn boilerplate(header: &mut serde_json::Map<String, Value>) {
    for key in ["stateRoot", "transactionsRoot", "receiptsRoot"] {
        header.entry(key).or_insert_with(|| json!(zeros(32)));
    }
    header.entry("sha3Uncles").or_insert_with(|| json!(zeros(32)));
    header.entry("logsBloom").or_insert_with(|| json!(zeros(256)));
    header.entry("uncles").or_insert_with(|| json!([]));
}

fn block(mut header: Value, transactions: Vec<Value>) -> Value {
    let map = header.as_object_mut().expect("fixture header is an object");
    boilerplate(map);
    map.insert("transactions".to_string(), Value::Array(transactions));
    header
}

/// OP Mainnet block 134,217,728 (`0x8000000`).
///
/// Reduced to two transactions: the L1-attributes deposit the sequencer places
/// first in every OP-stack block (type `0x7e`, the byte that used to fail the
/// whole response), and one ordinary EIP-1559 transaction beside it.
pub(crate) fn op_mainnet_block() -> Value {
    block(
        json!({
            "hash": "0xecff7a0ff9580ef1d9ffe84673255ce09ae495b89a8bae5b1fb1e2747c40f773",
            "parentHash": "0x9832ecd6e1cb6fe40d18ec77c3e420e74bce7d7ffdb65ea6208294b03b8945b8",
            "number": "0x8000000",
            "timestamp": "0x67f3d9b9",
            "miner": "0x4200000000000000000000000000000000000011",
            "gasLimit": "0x3938700",
            "gasUsed": "0xf66210",
            "difficulty": "0x0",
            "size": "0x56da",
            "extraData": "0x00000000fa00000006",
            "baseFeePerGas": "0x719d12",
            "mixHash": "0x89dcf088f9955194c5349f914b6ddf58edd6dea242fa0cb11d3d6ab15e2cf06f",
            "nonce": "0x0000000000000000",
            // Cancun. OP-stack chains carry the fields but post no blobs of
            // their own, so both gas figures are a real zero here.
            "blobGasUsed": "0x0",
            "excessBlobGas": "0x0",
            "parentBeaconBlockRoot":
                "0xc780419b2497ab5b41ee93f5e43774d797f04bb702144c61d219a76f84fb163b",
            "withdrawals": [],
            "withdrawalsRoot":
                "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        }),
        vec![
            json!({
                "hash": "0xfef32a4fab45b73e89e64ba640ef261009a157c64617956ab64e831856f928f7",
                "blockHash": "0xecff7a0ff9580ef1d9ffe84673255ce09ae495b89a8bae5b1fb1e2747c40f773",
                "blockNumber": "0x8000000",
                "transactionIndex": "0x0",
                "type": "0x7e",
                "from": "0xdeaddeaddeaddeaddeaddeaddeaddeaddead0001",
                "to": "0x4200000000000000000000000000000000000015",
                "gas": "0xf4240",
                "gasPrice": "0x0",
                "nonce": "0x1ba3d8a",
                "value": "0x0",
                "input": "0x440a5e200000146b000f79c5000000000000000300000000\
                          67f3d96300000000015302f100000000000000000000000000\
                          000000000000000000000000000002438e769a000000000000\
                          000000000000000000000000000000000000000000019ac964\
                          ba5f3511f78ffce1555042a48961196583ef9de7ab6384226f\
                          da712273645e02610000000000000000000000006887246668\
                          a3b87f54deb3b94ba47a6f63f32985",
                // Deposit-only fields.
                "sourceHash":
                    "0x883aa371d61056b40ec30d9b74257103351b24d70eb8063940f19f7510e39799",
                "mint": "0x0",
                "depositReceiptVersion": "0x1",
                // Deposits are not signed. The node reports zeroes rather than
                // omitting the keys, which is exactly the trap being tested.
                "r": "0x0",
                "s": "0x0",
                "v": "0x0",
                "yParity": "0x0",
            }),
            json!({
                "hash": "0x4a122c4be49a3b41419c95db593e6e81c0b4acc70d09ff22e0816c1f2bab00ee",
                "blockHash": "0xecff7a0ff9580ef1d9ffe84673255ce09ae495b89a8bae5b1fb1e2747c40f773",
                "blockNumber": "0x8000000",
                "transactionIndex": "0x1",
                "type": "0x2",
                "chainId": "0xa",
                "from": "0x1442511b89b7d460de4e728f6425098e62d4be6a",
                "to": "0x62d2293e1db885e8ba8eeac7829a7eae26cee1a5",
                "gas": "0x5e394",
                "gasPrice": "0x2eccc64d",
                "maxFeePerGas": "0x2eccc64d",
                "maxPriorityFeePerGas": "0x2eccc64d",
                "nonce": "0xfc2d",
                "value": "0x0",
                "input": "0xb2460c48000d0000000000576ead010000000000006323d3573c5240b93462254ec2fc01",
                "accessList": [],
                "r": "0x1f973a37b2f99e7920628e74425bea066235dd413d658ca9f9c2f66d3b24fe8f",
                "s": "0xd9d2b4e7488d55a4c6625ec9b92057f4c627e28ceac416696384f32c7973bd4",
                "v": "0x0",
                "yParity": "0x0",
            }),
        ],
    )
}

/// Arbitrum One block 201,326,592 (`0xc000000`).
///
/// Reduced to its `ArbitrumInternalTx` (type `0x6a`), the ArbOS bookkeeping
/// transaction that opens every Arbitrum block. Note what the header does
/// *not* have: no `blobGasUsed`, no `parentBeaconBlockRoot`, no
/// `withdrawalsRoot` — and three fields no other family sends.
pub(crate) fn arbitrum_one_block() -> Value {
    block(
        json!({
            "hash": "0x33fb349283ffd2e2c5302492d11fcb96e606cb1899e9bdcef102ab89edf33c70",
            "parentHash": "0x08b38a627f91fcffc6581241e9b272ab34438975773e76845703a82e4e3fa5ee",
            "number": "0xc000000",
            "timestamp": "0x661d51b9",
            "miner": "0xa4b000000000000000000073657175656e636572",
            "gasLimit": "0x4000000000000",
            "gasUsed": "0x3c1d21",
            "difficulty": "0x1",
            "size": "0x28f2",
            "baseFeePerGas": "0x989680",
            // Arbitrum reuses `extraData` for the send root and packs the send
            // count and L1 block into `mixHash` / `nonce`. Both are also sent
            // under their own names, which is what triodion reads.
            "extraData": "0xf0f401b0308982116f63f8af9eac3d2ddf7545cfab79a3e132538c36c1036557",
            "mixHash": "0x000000000001c40c00000000012c04c100000000000000140000000000000000",
            "nonce": "0x000000000016a24e",
            "l1BlockNumber": "0x12c04c1",
            "sendRoot": "0xf0f401b0308982116f63f8af9eac3d2ddf7545cfab79a3e132538c36c1036557",
            "sendCount": "0x1c40c",
        }),
        vec![json!({
            "hash": "0x431a1c991fe68597f472ddd6e675570b5e1e0825ea90da5195709658e77bfede",
            "blockHash": "0x33fb349283ffd2e2c5302492d11fcb96e606cb1899e9bdcef102ab89edf33c70",
            "blockNumber": "0xc000000",
            "transactionIndex": "0x0",
            "type": "0x6a",
            "chainId": "0xa4b1",
            "from": "0x00000000000000000000000000000000000a4b05",
            "to": "0x00000000000000000000000000000000000a4b05",
            "gas": "0x0",
            "gasPrice": "0x0",
            "nonce": "0x0",
            "value": "0x0",
            "input": "0x6bf6a42d0000000000000000000000000000000000000000\
                      00000000000000000000000000000000000000000000000000\
                      000000000000000000000000000000012c04c1000000000000\
                      000000000000000000000000000000000000000000000c0000\
                      00000000000000000000000000000000000000000000000000\
                      0000000000000000",
            // Unsigned, like the OP deposit above.
            "r": "0x0",
            "s": "0x0",
            "v": "0x0",
        })],
    )
}

/// Ethereum mainnet block 20,000,000 (`0x1312d00`), reduced to its one
/// EIP-4844 blob transaction.
///
/// The calldata is truncated to its selector — this fixture exists to exercise
/// the blob-hash-to-transaction join, and nothing reads the input. Every other
/// value, including the versioned hash, is verbatim.
pub(crate) fn mainnet_blob_block() -> Value {
    block(
        json!({
            "hash": "0xd24fd73f794058a3807db926d8898c6481e902b7edb91ce0d479d6760f276183",
            "parentHash": "0x9f6d1a1a5e9d0b1c14a6a1cb2ad0f0f3a3ef7c34cb03bd0f18df2a3ba46d3d78",
            "number": "0x1312d00",
            // 2024-06-01T22:36:47Z -> beacon slot 9,204,782.
            "timestamp": "0x665ba27f",
            "miner": "0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5",
            "gasLimit": "0x1c9c380",
            "gasUsed": "0xb5e6b5",
            "difficulty": "0x0",
            "size": "0x1f3a4",
            "extraData": "0x6265617665726275696c642e6f7267",
            "baseFeePerGas": "0x1263d3d54",
            "mixHash": "0x2d6bd1cf0f8ac4a1e0d0d0a9a5b0e2c1f7b6a3d4c5e6f70819a2b3c4d5e6f708",
            "nonce": "0x0000000000000000",
            "blobGasUsed": "0x20000",
            "excessBlobGas": "0x0",
            "parentBeaconBlockRoot":
                "0x2e5d1f04e0bd1a1b6c1a6a04c9a2d9d3e56f4a1b2c3d4e5f60718293a4b5c6d7",
            "withdrawals": [],
            "withdrawalsRoot":
                "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        }),
        vec![json!({
            "hash": "0x0ff07f37baa7fa26bb7de3d3fc63002bf0acf3295bdab7f67c108c0d1a3bff15",
            "blockHash": "0xd24fd73f794058a3807db926d8898c6481e902b7edb91ce0d479d6760f276183",
            "blockNumber": "0x1312d00",
            "transactionIndex": "0x15",
            "type": "0x3",
            "chainId": "0x1",
            "from": "0x000000633b68f5d8d3a86593ebb815b4663bcbe0",
            "to": "0x06a9ab27c7e2255df1815e6cc0168d7755feb19a",
            "gas": "0x2dc6c0",
            "gasPrice": "0x25049f114",
            "maxFeePerGas": "0x826299e00",
            "maxPriorityFeePerGas": "0x12a05f200",
            "maxFeePerBlobGas": "0x3b9aca00",
            "nonce": "0x50d6",
            "value": "0x3b9aca00",
            // Truncated to the selector; see the note above.
            "input": "0xef16e845",
            "accessList": [],
            "blobVersionedHashes": [
                "0x017ba4bd9c166498865a3d08618e333ee84812941b5c3a356971b4a6ffffa574"
            ],
            "r": "0x79d49cd5724eb7194af4202b59a25e9782d3bd6cb8f20e7049dd0204c8ff58e8",
            "s": "0x662fb12590d7121243aaddf9d39ab8231758abbfe84df53805750fd40db6c1ce",
            "v": "0x1",
            "yParity": "0x1",
        })],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keys whose value is a *byte string* rather than a quantity.
    ///
    /// The distinction is the point: JSON-RPC quantities are minimally encoded,
    /// so `"0x0"` is a correct `blobGasUsed` and an odd length proves nothing.
    /// Byte strings are fixed- or whole-width, so an odd length there is always
    /// a transcription slip. `nonce` is deliberately absent — it is a byte
    /// string on a header and a quantity on a transaction.
    const BYTE_STRING_KEYS: &[&str] = &[
        "input",
        "hash",
        "blockHash",
        "parentHash",
        "stateRoot",
        "transactionsRoot",
        "receiptsRoot",
        "sha3Uncles",
        "logsBloom",
        "mixHash",
        "extraData",
        "sourceHash",
        "parentBeaconBlockRoot",
        "withdrawalsRoot",
        "sendRoot",
        "from",
        "to",
        "miner",
    ];

    /// Every byte string in a fixture is even-length and parses.
    ///
    /// Hand-transcribed calldata is exactly the thing that goes wrong quietly:
    /// an odd-length `input` deserializes to an *empty* `Bytes` rather than
    /// failing, so the block still parses and a length assertion elsewhere
    /// reports a confusing zero. This walks the fixtures and names the key.
    fn assert_byte_strings_are_well_formed(value: &Value, path: &str, key: Option<&str>) {
        match value {
            Value::String(s) if key.is_some_and(|k| BYTE_STRING_KEYS.contains(&k)) => {
                assert!(
                    alloy::hex::decode(s).is_ok(),
                    "{path} is not valid hex ({} chars after 0x)",
                    s.len().saturating_sub(2)
                );
            }
            Value::Object(map) => {
                for (child_key, child) in map {
                    assert_byte_strings_are_well_formed(
                        child,
                        &format!("{path}.{child_key}"),
                        Some(child_key),
                    );
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    assert_byte_strings_are_well_formed(child, &format!("{path}[{index}]"), None);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn the_fixtures_are_well_formed_hex_throughout() {
        assert_byte_strings_are_well_formed(&op_mainnet_block(), "op_mainnet_block", None);
        assert_byte_strings_are_well_formed(&arbitrum_one_block(), "arbitrum_one_block", None);
        assert_byte_strings_are_well_formed(&mainnet_blob_block(), "mainnet_blob_block", None);
    }

    #[test]
    fn the_transcribed_calldata_is_the_length_the_chain_reported() {
        let lengths = |block: Value| -> Vec<usize> {
            block["transactions"]
                .as_array()
                .expect("fixtures carry full bodies")
                .iter()
                .map(|tx| alloy::hex::decode(tx["input"].as_str().unwrap()).unwrap().len())
                .collect()
        };
        assert_eq!(lengths(op_mainnet_block()), vec![164, 36]);
        assert_eq!(lengths(arbitrum_one_block()), vec![132]);
        assert_eq!(lengths(mainnet_blob_block()), vec![4]);
    }
}
