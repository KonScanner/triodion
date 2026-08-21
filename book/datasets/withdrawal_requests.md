# withdrawal_requests

One row per EIP-7002 withdrawal request. Prague onward.

- can collect by block and not by transaction
- required parameters: [none]
- optional parameters: [none]
- dataset aliases: [none]

Needs `--beacon-rpc`. See
[deposit_requests](./deposit_requests.md#why-this-needs-a-beacon-node) for why.

## Schema

```
schema for withdrawal_requests
──────────────────────────────
- block_number: uint32
- timestamp: uint32
- slot: uint64
- request_index: uint32
- source_address: binary
- validator_pubkey: binary
- amount_gwei: uint64
- is_full_exit: bool
- chain_id: uint64

sorting withdrawal_requests by: block_number, request_index

other available columns: epoch, proposer_index
```

Run `triodion help withdrawal_requests` to print this from the binary.

## A request is not a withdrawal

This dataset and [withdrawals](./withdrawals.md) are different things, and the
names are close enough to be worth separating carefully.

| | `withdrawal_requests` | `withdrawals` |
| :- | :- | :- |
| EIP | 7002 | 4895 |
| What it is | a request to exit or withdraw | the payment itself |
| Who causes it | an execution address | the protocol |
| When | the block it was submitted in | a later block, after a queue |
| Source | consensus block, needs `--beacon-rpc` | execution block |

A request here causes a payment there, eventually. They are related by
validator, never by block, and there is no key that joins one row to one row —
a request may produce a payment many blocks later, and validators are also paid
without any request at all.

## Why the request exists

EIP-7002 lets the holder of a validator's *withdrawal credentials* exit it
without holding the validator key. That matters for staking pools and for any
setup where the two keys are held by different parties.

The requester is therefore an execution address, and that is what
`source_address` records. It must hold the validator's withdrawal credentials
for the request to be honoured.

## Zero does not mean nothing

`amount_gwei` of `0` is not an empty request. EIP-7002 encodes a **full exit**
as an amount of zero; a non-zero amount is a partial withdrawal.

This is the sharpest trap in the dataset. Summing `amount_gwei` reports zero
for the largest withdrawals on the chain — every full exit contributes nothing
to the total. `is_full_exit` is provided beside it precisely so the zero cannot
be read as an absence:

```bash
# full exits, which sum to zero gwei and are not nothing
triodion withdrawal_requests -b 22.5M:+1000
```

`is_full_exit` is derived, not reported by the node: it is exactly
`amount_gwei == 0`.
