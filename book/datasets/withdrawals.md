# withdrawals

One row per EIP-4895 validator withdrawal.

- can collect by block and not by transaction
- required parameters: [none]
- optional parameters: [none]
- dataset aliases: [none]

[blocks](./blocks.md) carries `withdrawals_count` and
`withdrawals_amount_gwei`. Those are an aggregate, and an aggregate cannot be
taken apart again: which validator was paid, and how much each was paid, is
gone by the time that row is written. This dataset keeps the records.

## Schema

```
schema for withdrawals
──────────────────────
- block_number: uint32
- timestamp: uint32
- withdrawal_index: uint64
- validator_index: uint64
- address: binary
- amount_gwei: uint64
- chain_id: uint64

sorting withdrawals by: block_number, withdrawal_index

other available columns: block_hash, amount_wei
```

Run `triodion help withdrawals` to print this from the binary.

## A withdrawal is not a transaction

The protocol credits a withdrawal directly. It has no sender, no gas cost, no
receipt, no transaction hash and no calldata, and it never executes.

That has one consequence worth stating plainly: a withdrawal appears in no
other dataset. [traces](./traces.md) and [native_transfers](./native_transfers.md) both
read execution, and a withdrawal does not execute, so an ETH-flow analysis
built from those two alone is missing every validator payout on the chain. This
dataset is the only place they appear.

## Units

`amount_gwei` is gwei, which is the unit the protocol itself uses here. The
name says so because the mistake is easy and silent: reading it as wei
understates every payout by a factor of a billion.

`amount_wei` is the same value in wei, for joining against columns that are in
wei — `transactions.value`, for instance. It is opt-in, and it is a `u256`
rather than a `u64` because 2048 ETH in wei does not fit in 64 bits.

```bash
triodion withdrawals -b 20M:+100
triodion withdrawals -b 20M:+100 -i amount_wei
```

## Indexing

`withdrawal_index` is issued by the consensus layer and is monotonic across the
whole chain, not restarted per block. It is the stable identifier for a
withdrawal.

`validator_index` identifies the validator that was paid. `address` is the
execution address in its withdrawal credentials, which is the same address that
`deposit_requests.withdrawal_address` reports at deposit time.

## Before Shanghai

Pre-Shanghai blocks carry no withdrawals list at all, and this dataset writes
no rows for them. That differs from [blocks](./blocks.md), where the aggregate
columns are not nullable and record `0`. Here, no row is the honest answer:
there is nothing to enumerate.
