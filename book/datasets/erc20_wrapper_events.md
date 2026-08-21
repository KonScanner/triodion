# erc20_wrapper_events

One row per `Deposit` or `Withdrawal` event on a wrapped native token —
the WETH shape, and the contracts that copy it.

```
erc20_wrapper_events
────────────────────
- can collect by block or by transaction
- required parameters: [none]
- optional parameters: address, topic1
- dataset aliases: wrapper_events, weth_events
- parameter aliases: 
    - contracts -> addresses


schema for erc20_wrapper_events
───────────────────────────────
- block_number: uint32
- transaction_index: uint32
- log_index: uint32
- transaction_hash: binary
- erc20: binary
- event_type: string
- account: binary
- value_binary: binary
- value_string: string
- value_f64: float64
- chain_id: uint64

sorting erc20_wrapper_events by: block_number, log_index

other available columns: block_hash
```

Run `triodion help erc20_wrapper_events` to print this from the binary.

Aliases: `wrapper_events`, `weth_events`.

Wrapped native tokens do not emit `Transfer` when native currency enters or
leaves the contract. They emit `Deposit(address indexed dst, uint wad)` and
`Withdrawal(address indexed src, uint wad)` instead, so a pipeline that only
reads [erc20_transfers](./erc20_transfers.md) sees WETH supply change with no
event explaining it. This dataset covers that gap.

The same event shape is used by the wrapped native token on most chains, so
point `--address` at whichever contract the chain uses:

```bash
# WETH on Ethereum mainnet
triodion erc20_wrapper_events -b 18M:+1000 \
    --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2
```

This dataset is a member of the `log_events` group.
