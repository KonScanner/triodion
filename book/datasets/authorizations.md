# authorizations

One row per EIP-7702 authorization tuple.

- can collect by block or by transaction
- required parameters: [none]
- optional parameters: [none]
- dataset aliases: [none]

[transactions](./transactions.md) carries `n_authorizations`, which is a count.
It says a type-`0x04` transaction carried three authorizations; it does not say
which accounts delegated, or to what code. This dataset keeps the tuples.

## Schema

```
schema for authorizations
─────────────────────────
- block_number: uint32
- transaction_index: uint64
- transaction_hash: binary
- authorization_index: uint32
- authority: binary
- delegate_address: binary
- authorization_chain_id: uint64
- nonce: uint64
- chain_id: uint64

sorting authorizations by: block_number, transaction_index, authorization_index

other available columns: y_parity, r, s
```

Run `triodion help authorizations` to print this from the binary.

## What EIP-7702 does

An authorization is a signed statement by an externally owned account that
calls to it should execute another contract's code. Applying one writes a
23-byte *delegation designator* — `0xef0100` followed by the delegate address —
as that account's code.

So the two halves live in different datasets:

| Question | Dataset |
| :- | :- |
| Who signed an authorization, and for what code? | `authorizations` |
| What is this account delegating to right now? | [codes](./codes.md), columns `is_delegated` and `delegate_address` |

They join on `authorizations.authority` = `codes.address`, and
`authorizations.delegate_address` = `codes.delegate_address`.

## Submitted is not applied

A row here is an authorization that was *included in a block*. The protocol
applies it only if the authority's nonce and the chain id still match at
execution time. A stale one is skipped, and the transaction carrying it still
succeeds — nothing in the transaction's own data records which happened.

This dataset therefore does not claim it did. Compare the row's `nonce` against
the authority's account nonce to tell the two apart:

```bash
# an authorization applies only if `nonce` equals the authority's nonce
triodion authorizations -b 22.5M:+100
triodion nonces -b 22.5M:+100 --address <AUTHORITY>
```

A worked example. In mainnet block 25,800,355, authority
`0xc4cbdbc0988fd5f419d5ed787ec2743b84be0d0b` carries an authorization
requiring nonce 632, while the account sits at nonce 631 before and after the
block. The authorization was included and skipped, and that account kept the
delegation it already had. Twenty-one of that block's twenty-two
authorizations applied; this one did not.

## The columns that can be null

`authority` is recovered from the signature, not stated in the payload. A
signature no key could have produced yields no address rather than a wrong one,
so `authority` is null in that case.

`authorization_chain_id` is null only if the value does not fit in 64 bits,
which no real chain id does.

`y_parity` is null when the node reported a parity outside `{0, 1}`. Every
authorization carries one, so a null here means malformed, not absent.

## Two values that look like nulls and are not

**`delegate_address` of all zeros.** The zero address is the defined way to
*clear* a delegation, not a missing value. An account whose latest applied
authorization named the zero address has no delegation.

**`authorization_chain_id` of `0`.** Zero is specified and means the
authorization is valid on *every* chain, not that the chain is unknown. This is
worth filtering on: a chain-id-zero authorization signed for one chain can be
replayed on another.

## Ordering matters

The protocol applies a transaction's authorizations in list order, so a later
tuple can overwrite an earlier one for the same authority. `authorization_index`
preserves that order; sort by it, not by hash.
