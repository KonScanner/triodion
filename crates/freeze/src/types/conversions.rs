use crate::CollectError;
/// conversion operations
use alloy::primitives::{Bytes, I256, U256};
use prefix_hex;

/// convert Bytes to u32
pub fn bytes_to_u32(value: Bytes) -> Result<u32, CollectError> {
    let v = value.to_vec();
    if v.len() == 32 && v[0..28].iter().all(|b| *b == 0) {
        Ok(u32::from_be_bytes([v[28], v[29], v[30], v[31]]))
    } else {
        Err(CollectError::CollectError("could not convert bytes to u32".to_string()))
    }
}

/// Decode an `eth_call` return that should be a single `uint256`.
///
/// A conformant `totalSupply()` / `balanceOf()` returns **exactly** one 32-byte
/// word. Anything else — empty, short, or long — is not a `uint256` answer and
/// decodes to `None`.
///
/// Both alternatives fabricate data, in opposite directions:
///
/// - `U256::try_from_be_slice` accepts any slice *up to* 32 bytes, so an empty return (what an
///   address with no code answers) becomes `Some(U256::ZERO)` — asserting a supply of exactly zero
///   for something that is not a token.
/// - Taking the leading word of an over-long return invents a number out of the first 32 bytes of a
///   payload that was never a `uint256`.
///
/// Requiring the exact width is the only reading that never invents a value,
/// and it makes the per-call and Multicall3 paths agree byte for byte.
pub fn decode_u256_word(data: &[u8]) -> Option<U256> {
    (data.len() == 32).then(|| U256::from_be_slice(data))
}

/// Converts data to Vec<u8>
pub trait ToVecU8 {
    /// Convert to Vec<u8>
    fn to_vec_u8(&self) -> Vec<u8>;
}

impl ToVecU8 for U256 {
    fn to_vec_u8(&self) -> Vec<u8> {
        self.to_be_bytes_vec()
    }
}

impl ToVecU8 for I256 {
    fn to_vec_u8(&self) -> Vec<u8> {
        self.into_raw().to_vec_u8()
    }
}

impl ToVecU8 for Vec<I256> {
    fn to_vec_u8(&self) -> Vec<u8> {
        self.iter().map(|x| x.into_raw()).collect::<Vec<_>>().to_vec_u8()
    }
}

impl ToVecU8 for Vec<U256> {
    fn to_vec_u8(&self) -> Vec<u8> {
        let mut vec = Vec::new();
        for value in self {
            vec.extend_from_slice(&value.to_be_bytes_vec())
        }
        vec
    }
}

// pub trait ToVecHex {
//     fn to_vec_hex(&self) -> Vec<String>;
// }

// impl ToVecHex for Vec<Vec<u8>> {
//     fn to_vec_hex(&self) -> Vec<String> {
//         self.iter().map(|v| prefix_hex::encode(v.clone())).collect()
//     }
// }

/// Encodes data as Vec of hex String
pub trait ToVecHex {
    /// Output type
    type Output;

    /// Convert to Vec of hex String
    fn to_vec_hex(&self) -> Self::Output;
}

impl ToVecHex for Vec<Vec<u8>> {
    type Output = Vec<String>;

    fn to_vec_hex(&self) -> Self::Output {
        self.iter().map(|v| prefix_hex::encode(v.clone())).collect()
    }
}

impl ToVecHex for Vec<Option<Vec<u8>>> {
    type Output = Vec<Option<String>>;

    fn to_vec_hex(&self) -> Self::Output {
        self.iter().map(|opt| opt.as_ref().map(|v| prefix_hex::encode(v.clone()))).collect()
    }
}

#[cfg(test)]
mod decode_u256_word_tests {
    use super::*;

    #[test]
    fn an_empty_return_is_absent_not_zero() {
        // An address with no code answers `eth_call` with `0x`. The old code
        // used `U256::try_from_be_slice`, which accepts any slice up to 32
        // bytes, so this decoded to `Some(0)` — writing "total supply is 0"
        // for an EOA. `--no-multicall erc20_supplies` on vitalik.eth produced
        // exactly that row.
        assert_eq!(decode_u256_word(&[]), None);
    }

    #[test]
    fn a_short_return_is_absent() {
        assert_eq!(decode_u256_word(&[0xff; 31]), None);
    }

    #[test]
    fn a_full_word_decodes() {
        let mut word = [0u8; 32];
        word[31] = 42;
        assert_eq!(decode_u256_word(&word), Some(U256::from(42)));
    }

    #[test]
    fn an_over_long_return_is_absent_not_a_guess() {
        // A 64-byte return is not a `uint256`. The Multicall3 path used to take
        // its leading word while the per-call path returned null, so the same
        // address produced different output depending on `--no-multicall`.
        let mut data = vec![0u8; 64];
        data[31] = 7;
        assert_eq!(decode_u256_word(&data), None);
    }

    #[test]
    fn max_supply_survives_the_round_trip() {
        assert_eq!(decode_u256_word(&[0xff; 32]), Some(U256::MAX));
    }
}
