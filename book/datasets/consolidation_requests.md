# consolidation_requests

One row per EIP-7251 consolidation request. Prague onward.

- can collect by block and not by transaction
- required parameters: [none]
- optional parameters: [none]
- dataset aliases: [none]

Needs `--beacon-rpc`. See
[deposit_requests](./deposit_requests.md#why-this-needs-a-beacon-node) for why.

## Schema

```
schema for consolidation_requests
─────────────────────────────────
- block_number: uint32
- timestamp: uint32
- slot: uint64
- request_index: uint32
- source_address: binary
- source_pubkey: binary
- target_pubkey: binary
- is_credential_upgrade: bool
- chain_id: uint64

sorting consolidation_requests by: block_number, request_index

other available columns: epoch, proposer_index
```

Run `triodion help consolidation_requests` to print this from the binary.

## What a consolidation does

Before Prague a validator held 32 ETH and no more. EIP-7251 raised that
ceiling, and a consolidation is how stake moves into a single larger validator:
the source validator is exited and its balance is added to the target.

The request is submitted from the execution layer by the address holding the
source validator's withdrawal credentials, which is `source_address`.

## Most of them are not consolidations

When `source_pubkey` equals `target_pubkey`, the request does not move stake
between validators at all. It upgrades that one validator's withdrawal
credentials from `0x01` to `0x02` — the compounding kind — so that it can hold
more than 32 ETH and compound its rewards.

These were the majority of consolidation requests in the weeks after Prague. A
count that treats every row as a merge is wrong by a wide margin, and it is
wrong in the direction that looks plausible.

`is_credential_upgrade` separates them. It is derived, not reported by the
node: it is exactly `source_pubkey == target_pubkey`.

```bash
# genuine merges only
triodion consolidation_requests -b 22.5M:+1000
# then filter is_credential_upgrade = false
```

The self-referential form is also how a validator opts in to compounding
without changing anything else about it, which is why it is a request rather
than a separate mechanism.
