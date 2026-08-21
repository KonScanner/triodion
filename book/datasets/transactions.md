# transactions

One row per transaction.

- can collect by block or by transaction
- required parameters: [none]
- optional parameters: `from_address`, `to_address`
- dataset aliases: `txs`

Transactions are read through alloy's `AnyNetwork`. This is what makes the
dataset work on the OP stack and the Arbitrum stack at all: alloy's
`Ethereum` transaction envelope models only Ethereum's EIP-2718 type bytes and
deserializes as an untagged enum, so one OP deposit (type `0x7e`) or one
Arbitrum internal transaction (type `0x6a`) failed the entire
`eth_getBlockByNumber` response. Every OP-stack block opens with a deposit and
every Arbitrum block opens with an internal transaction, so on those chains
every block failed and the dataset collected no rows.

## Schema

```
schema for transactions
───────────────────────
- block_number: uint32
- transaction_index: uint64
- transaction_hash: binary
- nonce: uint64
- from_address: binary
- to_address: binary
- value_binary: binary
- value_string: string
- value_f64: float64
- input: binary
- gas_limit: uint64
- gas_used: uint64
- gas_price: uint64
- transaction_type: uint32
- max_priority_fee_per_gas: uint64
- max_fee_per_gas: uint64
- success: bool
- n_input_bytes: uint32
- n_input_zero_bytes: uint32
- n_input_nonzero_bytes: uint32
- chain_id: uint64

sorting transactions by: block_number, transaction_index

other available columns: n_rlp_bytes, block_hash, timestamp, r, s, v,
y_parity, tx_chain_id, chain_family, n_access_list_addresses,
n_access_list_storage_keys, max_fee_per_blob_gas, n_blob_versioned_hashes,
blob_versioned_hashes, blob_gas_used, blob_gas_price, n_authorizations,
source_hash, mint, is_system_tx, deposit_receipt_version, l1_fee, l1_gas_used,
l1_gas_price, l1_fee_scalar, l1_blob_base_fee, l1_base_fee_scalar,
l1_blob_base_fee_scalar, operator_fee_scalar, operator_fee_constant,
gas_used_for_l1, gas_used_for_l2, request_id, ticket_id, refund_to
```

Run `triodion help transactions` to print this from the binary.

The default column set is unchanged from before the multi-chain work. Every
column listed under "other available columns" is opt-in, so an existing
pipeline that does not name columns produces a byte-identical output schema
after upgrading.

`value` is a 256-bit integer, written once per format named by `--u256-types`,
which is why it appears as `value_binary`, `value_string` and `value_f64`. The
same expansion applies to the 256-bit optional columns:
`max_fee_per_blob_gas`, `blob_gas_price`, `mint`, `l1_fee` and
`l1_blob_base_fee`.

## Columns that require a receipt

A transaction body does not carry what it cost. These columns come from
`eth_getTransactionReceipt`, and asking for any of them makes the run fetch
receipts, which roughly doubles the request count:

`gas_used`, `success`, `gas_price`, `blob_gas_used`, `blob_gas_price`,
`l1_fee`, `l1_gas_used`, `l1_gas_price`, `l1_fee_scalar`, `l1_blob_base_fee`,
`l1_base_fee_scalar`, `l1_blob_base_fee_scalar`, `operator_fee_scalar`,
`operator_fee_constant`, `gas_used_for_l1`, `gas_used_for_l2`.

Three of those are default columns, so a default run already fetches receipts.
Drop `gas_used`, `success` and `gas_price` with `-e` if you only need bodies.

`gas_price` now prefers the receipt's `effectiveGasPrice`, then the
transaction's own `gasPrice`, and only computes the EIP-1559 formula when
neither is present.

## `v` and `y_parity`

**This is a breaking change to an existing column.**

`v` used to be a `bool`. It held alloy's `Signature::v()`, which is the
y-parity **bit**, not the EIP-155 `v` **scalar**. Those are different numbers.
Every legacy row was therefore labelled with a value that was not its `v`, and
because a replay-protected legacy transaction encodes its chain id inside `v`,
that chain id was unrecoverable from the output.

`v` is now `uint64` and holds the scalar as it appears on the wire:

| transaction | `v` |
| --- | --- |
| legacy, unprotected | `27` or `28` |
| legacy, EIP-155 replay-protected | `chain_id * 2 + 35 + parity` |
| typed (EIP-2930 and later) | `0` or `1` |

The parity bit is still available, as the new `y_parity` column. That column is
what the old `v` column actually contained, so a reader who wants the previous
values should read `y_parity`, not `v`.

`v` is not a default column. Only a caller who named it explicitly is
affected, and that caller was being handed the wrong number.

Unsigned transaction types have no signature. OP deposits and Arbitrum
internal transactions report `r`, `s`, `v` and `y_parity` as null, rather than
the zeros the node sends for them, because a zero there would assert that a
signature exists.

## Optional columns by origin

### Signature and identity

| column | meaning |
| --- | --- |
| `r`, `s` | signature components; null on unsigned transaction types |
| `v` | EIP-155 `v` scalar; see above |
| `y_parity` | the parity bit on its own |
| `tx_chain_id` | the chain id the transaction commits to |
| `chain_family` | which family defined this transaction's type byte |

`tx_chain_id` is not the same column as `chain_id`. `chain_id` is the network
the run pointed at. `tx_chain_id` is what the transaction itself committed to,
and it is null for an unprotected pre-EIP-155 legacy transaction, which
committed to nothing.

`chain_family` is one of `ethereum`, `op_stack`, `arbitrum` or `unknown`. It
classifies the transaction, not the chain: an OP-stack chain carries mostly
type `0x02` transactions, and those are `ethereum`. A type byte no supported
family defines is `unknown`; shared columns are still populated for it, since
they come from JSON rather than from a decoder, and family-specific ones are
null.

### Encoding and position

`n_rlp_bytes`, `block_hash`, `timestamp`.

`n_rlp_bytes` changed type from `uint32` to a nullable `uint32`. It is null for
every transaction type alloy cannot re-encode, which is every OP-stack and
Arbitrum-stack type byte. The call that computes the encoded length panics
inside alloy for exactly those bytes, so the length is not computed and the
column reports null instead.

### EIP-2930 (Berlin): access lists

`n_access_list_addresses`, `n_access_list_storage_keys`.

Both are null for a legacy transaction, which has no access list, and `0` for
a typed transaction that carries an empty one. The null carries information.

### EIP-4844 (Cancun): blobs

| column | meaning |
| --- | --- |
| `max_fee_per_blob_gas` | the blob gas price bid |
| `n_blob_versioned_hashes` | number of blobs committed to |
| `blob_versioned_hashes` | the hashes concatenated, 32 bytes each, in commitment order |
| `blob_gas_used` | blob gas consumed, from the receipt |
| `blob_gas_price` | blob gas price paid, from the receipt |

`blob_versioned_hashes` is kept verbatim rather than summarised because it is
the only link from an L1 transaction to the blob it paid for. That is the join
key the [blobs](./blobs.md) dataset uses.

### EIP-7702 (Prague): authorizations

`n_authorizations`, the number of authorization tuples in a set-code
transaction. Null for every other type.

### OP stack

Deposit-transaction fields, present only on type `0x7e`:

| column | meaning |
| --- | --- |
| `source_hash` | the L1 hash the deposit was derived from |
| `mint` | ETH minted on L2 by this deposit, in wei |
| `is_system_tx` | set on transactions the protocol itself issues |
| `deposit_receipt_version` | Canyon added version 1; absent on pre-Canyon deposits |

The L1-fee family, from the receipt:

`l1_fee`, `l1_gas_used`, `l1_gas_price`, `l1_fee_scalar`, `l1_blob_base_fee`,
`l1_base_fee_scalar`, `l1_blob_base_fee_scalar`, `operator_fee_scalar`,
`operator_fee_constant`.

`l1_fee` is the total L1 data-availability fee charged, in wei. It is **not**
included in `gas_used * gas_price`. On an OP-stack chain it is usually the
larger of the two, so a fee analysis that ignores it is wrong rather than
imprecise. `l1_fee_scalar` is the pre-Ecotone scalar; `l1_blob_base_fee`,
`l1_base_fee_scalar` and `l1_blob_base_fee_scalar` are the Ecotone
replacements; the two `operator_fee_*` columns arrived with Isthmus. Each is
null where the chain is not at that fork.

### Arbitrum stack

| column | meaning |
| --- | --- |
| `gas_used_for_l1` | the part of `gas_used` that paid for L1 data availability |
| `gas_used_for_l2` | the remainder, which is execution gas |
| `request_id` | the L1 request that created an unsigned or retryable transaction |
| `ticket_id` | the retryable ticket being redeemed |
| `refund_to` | where to refund an unused submission fee |

## `gas_used` on Arbitrum

Arbitrum folds the L1 data-availability charge into `gas_used`. An Arbitrum
`gas_used` has therefore never been comparable to a mainnet one: it is two
different quantities added together. On a sampled Arbitrum One transaction,
`gas_used_for_l1` was 217,092 of a `gas_used` of 309,001. About 70% of the
number was data availability, not execution.

`gas_used_for_l2` is the execution-only remainder, `gas_used - gas_used_for_l1`.
It is the column to compare against a mainnet `gas_used`.

The subtraction is checked. If a node reports an L1 share larger than the
total, `gas_used_for_l2` is null rather than a wrapped number, because that
node is describing something triodion does not model.

Both columns come from the receipt, and both are null on every non-Arbitrum
chain.
