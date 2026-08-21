# blocks

One row per block, read from the block header.

- can collect by block or by transaction
- required parameters: [none]
- optional parameters: [none]
- dataset aliases: [none]

Headers are read through alloy's `AnyNetwork` rather than its `Ethereum`
network, so the same code path serves mainnet, the OP stack and the Arbitrum
stack. Fields a chain does not define are read as null. Fields with no
standard header name, such as the three Arbitrum ones, arrive in the block's
extra-fields map and are decoded from there.

## Schema

```
schema for blocks
─────────────────
- block_number: uint32
- block_hash: binary
- timestamp: uint32
- author: binary
- gas_used: uint64
- extra_data: binary
- base_fee_per_gas: uint64
- withdrawals_count: uint32
- withdrawals_amount_gwei: uint64
- chain_id: uint64

sorting blocks by: block_number

other available columns: parent_hash, uncles_hash, state_root,
transactions_root, receipts_root, gas_limit, logs_bloom, difficulty,
total_difficulty, size, mix_hash, nonce, withdrawals_root, blob_gas_used,
excess_blob_gas, parent_beacon_block_root, requests_hash, l1_block_number,
send_root, send_count
```

Run `triodion help blocks` to print this from the binary.

The ten columns above the sort line are the default columns. The rest are
opt-in. Add them with `-i`, or replace the default set with `--columns`:

```bash
triodion blocks -b 20M:+100 -i blob_gas_used excess_blob_gas
triodion blocks -b 20M:+100 --columns block_number timestamp requests_hash
```

`total_difficulty` is a 256-bit integer. It is written in each of the formats
that `--u256-types` names, which defaults to `binary`, `string` and `f64`, so
one requested column can produce three output columns.

## Newer header columns

### EIP-4844: `blob_gas_used`, `excess_blob_gas`

Cancun added a second gas market for blob data. `blob_gas_used` is the blob gas
the block consumed, and `excess_blob_gas` is the running excess that sets the
next block's blob gas price. Both are null before Cancun and on chains that
never enabled blobs.

These columns describe the blob market only. The blobs themselves are not on
the execution layer at all. See [blobs](./blobs.md).

### EIP-4788: `parent_beacon_block_root`

The beacon block root of the parent slot, which Cancun exposes to the execution
layer. It is the value that lets an execution-layer query reach consensus-layer
state without trusting an oracle. Null before Cancun and on chains that do not
implement EIP-4788.

### EIP-7685: `requests_hash`

A single commitment over the block's execution requests: EIP-6110 deposits,
EIP-7002 withdrawals and EIP-7251 consolidations. Added by Prague. Null before
Prague and on chains without it.

It is a commitment and nothing more — the requests themselves are not in the
execution block, and a hash cannot be turned back into what it commits to. For
the requests, read [deposit_requests](./deposit_requests.md),
[withdrawal_requests](./withdrawal_requests.md) and
[consolidation_requests](./consolidation_requests.md), which take them from the
consensus block and need `--beacon-rpc`.

### Arbitrum header fields: `l1_block_number`, `send_root`, `send_count`

Arbitrum adds three fields to the header. `l1_block_number` is the L1 block the
L2 block was sequenced against. `send_root` and `send_count` track the outbox
accumulator for L2-to-L1 messages: the root of the accumulator and the number
of messages sent so far. All three are null on every other chain.

`mix_hash` and `nonce` are optional on a cross-chain header. Arbitrum populates
both with its own meanings, so they are kept rather than dropped.

## Why these columns are null and not zero

A null in these columns means the concept did not exist at this height on this
chain. A zero would be a measurement.

The difference matters as soon as you aggregate. On Arbitrum One,
`blob_gas_used` is null, because Arbitrum never enabled blobs. Writing zero
there would state that the block spent zero blob gas, which is a claim about a
market that does not exist on that chain, and it would sum into a total that
looks like real data. The same rule applies to `parent_beacon_block_root` and
`requests_hash` on a chain that does not implement those EIPs.

On an OP-stack header the Cancun fields are present and `blob_gas_used` is
genuinely `0`: the concept exists and the block used none of it. So the two
cases are distinguishable in the output, which is the point.

Two columns deliberately break this rule. `withdrawals_count` and
`withdrawals_amount_gwei` are `0` for pre-Shanghai blocks, where the block
carries no withdrawals list at all. They are not nullable. Pair them with
`timestamp` if you need to separate "no withdrawals in this block" from
"before withdrawals existed".

## The withdrawals themselves

Those two columns are an aggregate, and an aggregate cannot be taken apart
again: which validator was paid, and how much each was paid, is not recoverable
from a count and a sum. [withdrawals](./withdrawals.md) emits one row per
withdrawal from the same block body, with no extra RPC url required.
