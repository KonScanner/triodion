# erc20_approvals

One row per ERC-20 `Approval` event: the owner granting the allowance, the
spender receiving it, and the amount.

```
erc20_approvals
───────────────
- can collect by block or by transaction
- required parameters: [none]
- optional parameters: address, topic0, topic1, topic2, from_address, to_address
- dataset aliases: [none]
- parameter aliases: 
    - contracts -> addresses


schema for erc20_approvals
──────────────────────────
- block_number: uint32
- transaction_index: uint32
- log_index: uint32
- transaction_hash: binary
- erc20: binary
- from_address: binary
- to_address: binary
- value_binary: binary
- value_string: string
- value_f64: float64
- chain_id: uint64

sorting erc20_approvals by: block_number, log_index

other available columns: block_hash
```

Run `triodion help erc20_approvals` to print this from the binary.

`Approval` shares its shape with `Transfer` — two indexed addresses and a
value — so the columns line up with [erc20_transfers](./erc20_transfers.md).
The names differ in meaning: `from_address` is the token owner and
`to_address` is the approved spender, not a recipient.

An approval is a grant, not a movement. A large `value` here says a spender
*may* move that much, and says nothing about whether it ever did. The common
`2^256 - 1` value is an unlimited approval.

Filter to one token with `--contract`, which is an alias for `--address`:

```bash
triodion erc20_approvals -b 18M:+1000 \
    --contract 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
```

This dataset is a member of the `log_events` group.
