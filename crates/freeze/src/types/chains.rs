//! Cross-chain-family plumbing.
//!
//! triodion targets three EVM chain families — Ethereum mainnet, the OP stack
//! (OP Mainnet, Base, …) and the Arbitrum stack (Arbitrum One, Nova, …). They
//! do not agree on the set of EIP-2718 transaction type bytes:
//!
//! | family   | extra type bytes                     |
//! |----------|--------------------------------------|
//! | Ethereum | none beyond `0x00`–`0x04`            |
//! | OP stack | `0x7e` deposit                       |
//! | Arbitrum | `0x64`–`0x6a`, `0x78`                |
//!
//! `alloy::consensus::TxEnvelope` models Ethereum's set exactly, and its
//! `serde` impl is an untagged enum. A single OP deposit transaction in a block
//! therefore fails the *whole* `eth_getBlockByNumber` response with
//! "data did not match any variant of untagged enum BlockTransactions" — not a
//! missing column, a missing block. Every OP-stack and Arbitrum-stack block
//! contains at least one such transaction (the L1-attributes deposit and the
//! `ArbitrumInternalTx` respectively), so before this module `transactions`
//! collected exactly zero rows on those chains.
//!
//! The fix is to speak [`alloy::network::AnyNetwork`] instead of `Ethereum`.
//! Unknown type bytes deserialize into [`alloy::network::UnknownTxEnvelope`],
//! which keeps every JSON field verbatim in an `OtherFields` map and still
//! implements `alloy::consensus::Transaction`, so the shared columns (nonce,
//! value, input, gas, to/from) are read the same way on every chain. The
//! family-specific fields are read out of that map by name — see
//! [`op`] and [`arbitrum`] for the keys, and the `other_*` readers below.
//!
//! # Encoding is one-way
//!
//! `AnyTxEnvelope::encode_2718` *panics* on an unknown type byte. triodion
//! never re-encodes a transaction, but any future code that does must check
//! [`is_reencodable`] first. `trie_hash` is safe — for unknown types it
//! returns the hash the RPC reported rather than recomputing it.

use alloy::{
    network::AnyTxEnvelope,
    primitives::{U256, U64},
    serde::OtherFields,
};

/// The network triodion speaks. See the module docs for why this is
/// `AnyNetwork` and not `Ethereum`.
pub type TriodionNetwork = alloy::network::AnyNetwork;

/// Provider handle for [`TriodionNetwork`].
pub type TriodionProvider = alloy::providers::RootProvider<TriodionNetwork>;

/// An `eth_getBlockBy*` response on any supported chain family.
pub type RpcBlock = alloy::network::AnyRpcBlock;

/// An `eth_getTransactionBy*` response on any supported chain family.
pub type RpcTransaction = alloy::network::AnyRpcTransaction;

/// An `eth_getTransactionReceipt` response on any supported chain family.
pub type RpcReceipt = alloy::network::AnyTransactionReceipt;

/// EIP-2718 transaction type bytes defined by the OP stack.
pub mod op {
    /// `OpTxType::Deposit` — an L1→L2 deposit, including the L1-attributes
    /// transaction the sequencer places first in every block.
    ///
    /// Deposits are not signed and have no meaningful `r`/`s`/`v`; the node
    /// reports zeros for all three.
    pub const DEPOSIT: u8 = 0x7e;

    /// Deposit-only field: the hash the deposit was derived from on L1. This
    /// is the join key back to the L1 `TransactionDeposited` event.
    pub const SOURCE_HASH: &str = "sourceHash";

    /// Deposit-only field: ETH minted on L2 by this deposit, in wei.
    pub const MINT: &str = "mint";

    /// Deposit-only field: set on transactions the protocol itself issues.
    /// Absent (rather than `false`) on user deposits from most nodes.
    pub const IS_SYSTEM_TX: &str = "isSystemTx";

    /// Deposit-only field: Canyon added version 1, which zeroes the deposit
    /// nonce in the receipt. Absent on pre-Canyon deposits.
    pub const DEPOSIT_RECEIPT_VERSION: &str = "depositReceiptVersion";

    /// Receipt field: total L1 data-availability fee charged, in wei. This is
    /// the dominant cost of an OP-stack transaction and is *not* included in
    /// `gasUsed * effectiveGasPrice`.
    pub const L1_FEE: &str = "l1Fee";

    /// Receipt field: L1 gas attributed to this transaction's calldata.
    pub const L1_GAS_USED: &str = "l1GasUsed";

    /// Receipt field: L1 base fee used in the L1-fee formula, in wei.
    pub const L1_GAS_PRICE: &str = "l1GasPrice";

    /// Receipt field: pre-Ecotone fee scalar, serialised as a decimal string.
    pub const L1_FEE_SCALAR: &str = "l1FeeScalar";

    /// Receipt field: Ecotone blob base fee, in wei.
    pub const L1_BLOB_BASE_FEE: &str = "l1BlobBaseFee";

    /// Receipt field: Ecotone base-fee scalar.
    pub const L1_BASE_FEE_SCALAR: &str = "l1BaseFeeScalar";

    /// Receipt field: Ecotone blob-base-fee scalar.
    pub const L1_BLOB_BASE_FEE_SCALAR: &str = "l1BlobBaseFeeScalar";

    /// Receipt field: Isthmus operator fee scalar.
    pub const OPERATOR_FEE_SCALAR: &str = "operatorFeeScalar";

    /// Receipt field: Isthmus operator fee constant.
    pub const OPERATOR_FEE_CONSTANT: &str = "operatorFeeConstant";
}

/// EIP-2718 transaction type bytes defined by the Arbitrum stack.
///
/// Values mirror `ArbitrumDepositTxType` … `ArbitrumLegacyTxType` in
/// go-ethereum's Arbitrum fork (`core/types/transaction.go`). `0x67` is
/// deliberately unassigned there and so is absent here.
pub mod arbitrum {
    /// `ArbitrumDepositTx` — an L1→L2 ETH deposit.
    pub const DEPOSIT: u8 = 0x64;
    /// `ArbitrumUnsignedTx` — an L1 contract calling L2 directly.
    pub const UNSIGNED: u8 = 0x65;
    /// `ArbitrumContractTx` — as above, but from an L1 contract's alias.
    pub const CONTRACT: u8 = 0x66;
    /// `ArbitrumRetryTx` — a retryable ticket being redeemed.
    pub const RETRY: u8 = 0x68;
    /// `ArbitrumSubmitRetryableTx` — a retryable ticket being created.
    pub const SUBMIT_RETRYABLE: u8 = 0x69;
    /// `ArbitrumInternalTx` — the ArbOS bookkeeping transaction that opens
    /// every Arbitrum block.
    pub const INTERNAL: u8 = 0x6a;
    /// `ArbitrumLegacyTx` — a pre-Nitro (classic) transaction, replayed into
    /// the Nitro chain with its original semantics.
    pub const LEGACY: u8 = 0x78;

    /// Header field: the L1 block this L2 block was sequenced against.
    pub const L1_BLOCK_NUMBER: &str = "l1BlockNumber";

    /// Header field: root of the outbox (L2→L1 message) accumulator.
    pub const SEND_ROOT: &str = "sendRoot";

    /// Header field: number of L2→L1 messages sent so far.
    pub const SEND_COUNT: &str = "sendCount";

    /// Receipt field: the portion of `gasUsed` that paid for L1 data
    /// availability rather than L2 execution.
    ///
    /// This is why an Arbitrum `gasUsed` cannot be compared to a mainnet one:
    /// it conflates the two. Execution gas is `gasUsed - gasUsedForL1`.
    pub const GAS_USED_FOR_L1: &str = "gasUsedForL1";

    /// Retryable/unsigned field: the L1 request that created this transaction.
    pub const REQUEST_ID: &str = "requestId";

    /// Retry field: the retryable ticket being redeemed.
    pub const TICKET_ID: &str = "ticketId";

    /// Submit-retryable field: where to refund unused submission fee.
    pub const REFUND_TO: &str = "refundTo";
}

/// Which chain family defined a given EIP-2718 transaction type byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainFamily {
    /// `0x00`–`0x04`: legacy, EIP-2930, EIP-1559, EIP-4844, EIP-7702. Shared
    /// by every EVM chain.
    Ethereum,
    /// OP-stack-specific. See [`op`].
    OpStack,
    /// Arbitrum-stack-specific. See [`arbitrum`].
    Arbitrum,
    /// A type byte no supported family defines. Shared columns are still read
    /// (they come from JSON, not from a decoder), family-specific ones are
    /// null. Reaching this is how a new chain announces itself.
    Unknown,
}

impl ChainFamily {
    /// Classify an EIP-2718 transaction type byte.
    ///
    /// Note this classifies the *transaction*, not the chain: an OP-stack
    /// chain carries mostly `0x02` transactions, which are `Ethereum` here.
    pub const fn of_tx_type(ty: u8) -> Self {
        match ty {
            0x00..=0x04 => Self::Ethereum,
            op::DEPOSIT => Self::OpStack,
            // Listed one by one rather than as `0x64..=0x6a`, because that
            // range would swallow `0x67`, which the Arbitrum fork leaves
            // unassigned. A byte nobody has defined belongs in `Unknown`.
            arbitrum::DEPOSIT |
            arbitrum::UNSIGNED |
            arbitrum::CONTRACT |
            arbitrum::RETRY |
            arbitrum::SUBMIT_RETRYABLE |
            arbitrum::INTERNAL |
            arbitrum::LEGACY => Self::Arbitrum,
            _ => Self::Unknown,
        }
    }

    /// Short lowercase name, suitable for a column value.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ethereum => "ethereum",
            Self::OpStack => "op_stack",
            Self::Arbitrum => "arbitrum",
            Self::Unknown => "unknown",
        }
    }
}

/// Whether this envelope can be re-encoded to EIP-2718 bytes.
///
/// `false` for every non-Ethereum type byte. Calling `encode_2718`,
/// `encode_2718_len` or `eip2718_encoded_length` on one of those **panics**
/// inside alloy, so any caller that needs the encoded form must branch on this
/// first and report a null rather than a number it cannot compute.
pub fn is_reencodable(envelope: &AnyTxEnvelope) -> bool {
    matches!(envelope, AnyTxEnvelope::Ethereum(_))
}

/// Read a quantity-encoded (`"0x1a"`) unsigned integer out of an extra-fields
/// map. `None` when the key is absent or does not parse.
pub fn other_u64(fields: &OtherFields, key: &str) -> Option<u64> {
    fields.get_deserialized::<U64>(key).and_then(Result::ok).map(|v| v.to())
}

/// Read a quantity-encoded 256-bit integer out of an extra-fields map.
///
/// Used for wei-denominated fields (`mint`, `l1Fee`) that have no reason to
/// fit in 64 bits on a chain nobody has audited for us.
pub fn other_u256(fields: &OtherFields, key: &str) -> Option<U256> {
    fields.get_deserialized::<U256>(key).and_then(Result::ok)
}

/// Read a hex byte string (`"0x89ab…"`) out of an extra-fields map.
///
/// Works for any width — `Address`, `B256` and variable-length `Bytes` are all
/// the same shape on the wire. An odd-length or non-hex value yields `None`
/// rather than a truncated prefix.
pub fn other_bytes(fields: &OtherFields, key: &str) -> Option<Vec<u8>> {
    let raw = fields.get(key)?.as_str()?;
    alloy::hex::decode(raw).ok()
}

/// Read a JSON boolean out of an extra-fields map.
///
/// Absent and `false` are *not* the same thing: most nodes omit `isSystemTx`
/// entirely on user deposits, so the caller decides what absence means.
pub fn other_bool(fields: &OtherFields, key: &str) -> Option<bool> {
    fields.get(key)?.as_bool()
}

/// Read a decimal-string-encoded number out of an extra-fields map.
///
/// OP's `l1FeeScalar` is serialised as a bare decimal string (`"0.684"` or
/// `"1000000"`), not as a hex quantity, so [`other_u64`] cannot read it.
pub fn other_decimal_f64(fields: &OtherFields, key: &str) -> Option<f64> {
    let value = fields.get(key)?;
    match value {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// The two places alloy can leave a chain-specific transaction field.
///
/// Which one is used depends on whether alloy recognised the type byte:
///
/// - **unknown type byte** (OP `0x7e`, Arbitrum `0x64`–`0x6a`): the envelope deserializes as
///   [`alloy::network::UnknownTxEnvelope`] and *every* field it does not model — including
///   `sourceHash`, `mint`, `requestId` — lands in the envelope's own map.
/// - **known type byte** (`0x00`–`0x04`): the envelope consumes the fields it knows and the outer
///   RPC wrapper catches the rest.
///
/// A caller that reads only one of the two silently returns `None` for half
/// the chains, so this reads both, envelope first.
#[derive(Debug, Clone, Copy)]
pub struct TxExtras<'a> {
    envelope: Option<&'a OtherFields>,
    outer: &'a OtherFields,
}

impl<'a> TxExtras<'a> {
    /// Build a view over a transaction's extra fields.
    pub fn new(envelope: Option<&'a OtherFields>, outer: &'a OtherFields) -> Self {
        Self { envelope, outer }
    }

    fn read<T>(&self, key: &str, reader: fn(&OtherFields, &str) -> Option<T>) -> Option<T> {
        self.envelope.and_then(|fields| reader(fields, key)).or_else(|| reader(self.outer, key))
    }

    /// Read a quantity-encoded unsigned integer. See [`other_u64`].
    pub fn u64(&self, key: &str) -> Option<u64> {
        self.read(key, other_u64)
    }

    /// Read a quantity-encoded 256-bit integer. See [`other_u256`].
    pub fn u256(&self, key: &str) -> Option<U256> {
        self.read(key, other_u256)
    }

    /// Read a hex byte string. See [`other_bytes`].
    pub fn bytes(&self, key: &str) -> Option<Vec<u8>> {
        self.read(key, other_bytes)
    }

    /// Read a JSON boolean. See [`other_bool`].
    pub fn bool(&self, key: &str) -> Option<bool> {
        self.read(key, other_bool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(json: serde_json::Value) -> OtherFields {
        serde_json::from_value(json).expect("test fixture is an object")
    }

    #[test]
    fn classifies_the_three_families_we_support() {
        assert_eq!(ChainFamily::of_tx_type(0x00), ChainFamily::Ethereum);
        assert_eq!(ChainFamily::of_tx_type(0x02), ChainFamily::Ethereum);
        assert_eq!(ChainFamily::of_tx_type(0x04), ChainFamily::Ethereum);
        assert_eq!(ChainFamily::of_tx_type(op::DEPOSIT), ChainFamily::OpStack);
        assert_eq!(ChainFamily::of_tx_type(arbitrum::INTERNAL), ChainFamily::Arbitrum);
        assert_eq!(ChainFamily::of_tx_type(arbitrum::LEGACY), ChainFamily::Arbitrum);
    }

    #[test]
    fn an_unassigned_arbitrum_byte_is_not_claimed_as_arbitrum() {
        // 0x67 sits inside 0x64..=0x6a numerically but go-ethereum's Arbitrum
        // fork never assigns it. Claiming it would invent a family for a
        // transaction we have no field names for.
        assert_eq!(ChainFamily::of_tx_type(0x67), ChainFamily::Unknown);
    }

    #[test]
    fn an_unheard_of_type_byte_is_unknown_not_a_parse_error() {
        assert_eq!(ChainFamily::of_tx_type(0x50), ChainFamily::Unknown);
        assert_eq!(ChainFamily::of_tx_type(0xff), ChainFamily::Unknown);
    }

    #[test]
    fn reads_the_op_deposit_fields_off_a_real_payload() {
        // Trimmed from OP Mainnet block 0x8000000, transaction index 0.
        let f = fields(serde_json::json!({
            "sourceHash": "0x883aa371d61056b40ec30d9b74257103351b24d70eb8063940f19f7510e39799",
            "mint": "0x0",
            "depositReceiptVersion": "0x1",
        }));
        assert_eq!(other_bytes(&f, op::SOURCE_HASH).map(|b| b.len()), Some(32));
        assert_eq!(other_u256(&f, op::MINT), Some(U256::ZERO));
        assert_eq!(other_u64(&f, op::DEPOSIT_RECEIPT_VERSION), Some(1));
        // Absent, and absence is not `false`.
        assert_eq!(other_bool(&f, op::IS_SYSTEM_TX), None);
    }

    #[test]
    fn reads_the_arbitrum_header_fields_off_a_real_payload() {
        // Trimmed from Arbitrum One block 0xc000001.
        let f = fields(serde_json::json!({
            "l1BlockNumber": "0x12c04c1",
            "sendRoot": "0xf0f401b0308982116f63f8af9eac3d2ddf7545cfab79a3e132538c36c1036557",
            "sendCount": "0x1c40c",
        }));
        assert_eq!(other_u64(&f, arbitrum::L1_BLOCK_NUMBER), Some(19_662_017));
        assert_eq!(other_bytes(&f, arbitrum::SEND_ROOT).map(|b| b.len()), Some(32));
        assert_eq!(other_u64(&f, arbitrum::SEND_COUNT), Some(115_724));
    }

    #[test]
    fn a_decimal_string_scalar_is_not_read_as_a_hex_quantity() {
        // OP serialises `l1FeeScalar` as a bare decimal string. Feeding it to
        // the quantity reader yields nothing, which is the honest answer.
        let f = fields(serde_json::json!({ "l1FeeScalar": "0.684" }));
        assert_eq!(other_u64(&f, op::L1_FEE_SCALAR), None);
        assert_eq!(other_decimal_f64(&f, op::L1_FEE_SCALAR), Some(0.684));
    }

    #[test]
    fn tx_extras_reads_the_envelope_map_before_the_outer_one() {
        // An OP deposit's `mint` arrives inside the unknown envelope. A reader
        // that only looked at the outer wrapper would report null for every
        // deposit on every OP-stack chain.
        let envelope = fields(serde_json::json!({ "mint": "0x2a" }));
        let outer = fields(serde_json::json!({}));
        let extras = TxExtras::new(Some(&envelope), &outer);
        assert_eq!(extras.u256(op::MINT), Some(U256::from(42)));
    }

    #[test]
    fn tx_extras_falls_back_to_the_outer_map_for_known_type_bytes() {
        // A `0x02` transaction on an L2 deserializes as a known Ethereum
        // envelope, so anything chain-specific lands on the outer wrapper.
        let outer = fields(serde_json::json!({ "l1BlockNumber": "0x10" }));
        let extras = TxExtras::new(None, &outer);
        assert_eq!(extras.u64(arbitrum::L1_BLOCK_NUMBER), Some(16));
    }

    #[test]
    fn a_missing_or_malformed_value_yields_none_not_a_prefix() {
        let f = fields(serde_json::json!({ "sendRoot": "0xzz", "sendCount": "not a number" }));
        assert_eq!(other_bytes(&f, arbitrum::SEND_ROOT), None);
        assert_eq!(other_u64(&f, arbitrum::SEND_COUNT), None);
        assert_eq!(other_bytes(&f, "absent"), None);
    }
}
