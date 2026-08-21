# erc1155_metadata

One row per ERC-1155 `URI(string value, uint256 indexed id)` log.

```
erc1155_metadata
────────────────
- can collect by block or by transaction
- required parameters: [none]
- optional parameters: address, topic1
- dataset aliases: erc1155_uris
- parameter aliases: 
    - contracts -> addresses


schema for erc1155_metadata
───────────────────────────
- block_number: uint32
- transaction_index: uint32
- log_index: uint32
- transaction_hash: binary
- erc1155: binary
- token_id_binary: binary
- token_id_string: string
- token_id_f64: float64
- uri: string
- is_uri_template: bool
- is_uri_lossy: bool
- chain_id: uint64

sorting erc1155_metadata by: block_number, log_index

other available columns: block_hash
```

Run `triodion help erc1155_metadata` to print this from the binary.

Alias: `erc1155_uris`.

## An empty result is the normal case

The `URI` event is optional in the standard, and most contracts never emit it.
They serve one static `{id}` template from `uri(id)` and never announce a
change. A full-range scan of a busy chain can legitimately return zero rows.

This is not a failure, and it is not a sign the filter is wrong.

## Why this reads logs and not uri(id)

A call needs a token id to call with. triodion partitions by block, address and
topic — there is no token-id dimension to enumerate ids from, so the call form
has no input. The log carries its own id, so the log form does.

`token_id` is indexed, so it arrives in `topics[1]`. `uri` is not indexed, so it
sits in the log data as an ABI-encoded dynamic string and is decoded, not
sliced.

## is_uri_template

True when `uri` contains the literal `{id}`.

Such a string is a template, not a URL, and fetching it verbatim fails. Expand
it with the row's `token_id` first — as 64 lowercase hex digits, no `0x`
prefix, which is what the standard requires of a conforming client.

It also means the string is shared by every token of the contract, so it does
not identify this token.

The match is literal and case-sensitive. Matching `{ID}` too would flag strings
that no conforming client expands.

## is_uri_lossy

True when the on-chain bytes were not valid UTF-8, so `uri` holds U+FFFD
replacement characters that were never on chain.

Without the column, a mangled string is indistinguishable from a faithful one.
With it, a pipeline can drop or re-read those rows rather than storing bytes the
contract never wrote.

```bash
triodion erc1155_metadata -b 18M:+10000 \
    --address 0x76be3b62873462d2142405439777e971754e8e77
```

This dataset is not a member of the `log_events` group. Requesting it
alongside `logs` or `erc20_transfers` therefore costs a separate
`eth_getLogs` per block range rather than sharing one with them.
