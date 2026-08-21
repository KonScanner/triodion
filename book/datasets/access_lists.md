# access_lists

One row per EIP-2930 access-list entry, exploded to one row per storage key.

- can collect by block or by transaction
- required parameters: [none]
- optional parameters: [none]
- dataset aliases: [none]

[transactions](./transactions.md) carries `n_access_list_addresses` and
`n_access_list_storage_keys`. Those are counts: they say how large a list was,
never which accounts or slots it named. This dataset keeps the entries.

## Schema

```
schema for access_lists
───────────────────────
- block_number: uint32
- transaction_index: uint64
- transaction_hash: binary
- entry_index: uint32
- address: binary
- storage_key_index: uint32
- storage_key: binary
- chain_id: uint64

sorting access_lists by: block_number, transaction_index, entry_index, storage_key_index

other available columns: transaction_type
```

Run `triodion help access_lists` to print this from the binary.

## The row shape

An access list is a list of accounts, and each account carries its own list of
storage keys. That is two levels, and a table is one, so the rows are the
flattening: one row per storage key, repeating the address.

`entry_index` is the account's position in the transaction's list.
`storage_key_index` is the key's position within that one account's keys. The
pair is unique within a transaction.

## A null storage key is an entry, not a gap

EIP-2930 permits an entry that names an account and no storage keys at all. It
still warms the account, and it still costs gas, so it is a real entry and it
gets a row — with `storage_key` and `storage_key_index` null.

Null here means "this entry listed no slots". It never means "unknown". Filter
with `storage_key IS NULL` to find them.

## A declaration, not a record

An access list states what a transaction intends to touch, before it runs. It
is not a record of what running it actually touched.

- A listed slot may never be read.
- A slot the transaction does read may be absent from the list — access lists
  are optional, and omitting a slot costs more gas but is legal.

For what execution actually touched, use `storage_reads` and `balance_reads`.
Comparing the two is a reasonable way to measure how well a list was built.

## Which transactions have one

`access_list()` is absent for a legacy (type `0x00`) transaction, which has no
such field, and present-but-possibly-empty for every typed one. Both produce no
rows here. If you need to tell "legacy, so no list" from "typed, and declared an
empty list", read `n_access_list_addresses` on
[transactions](./transactions.md), which is null in the first case and `0` in
the second.

Add `transaction_type` to see which type each row came from:

```bash
triodion access_lists -b 20M:+100 -i transaction_type
```
