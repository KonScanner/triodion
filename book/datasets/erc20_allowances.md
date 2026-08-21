# erc20_allowances

One row per (token, owner, spender) per block: how much of the owner's balance
the spender is currently permitted to move.

```
erc20_allowances
────────────────
- can collect by block and not by transaction
- required parameters: contract, from_address, to_address
- optional parameters: [none]
- dataset aliases: allowances
- parameter aliases: [none]


schema for erc20_allowances
───────────────────────────
- block_number: uint32
- erc20: binary
- from_address: binary
- to_address: binary
- allowance_binary: binary
- allowance_string: string
- allowance_f64: float64
- chain_id: uint64

sorting erc20_allowances by: block_number, erc20, from_address, to_address

other available columns: [none]
```

Run `triodion help erc20_allowances` to print this from the binary.

`--blocks` defaults to `latest`. All three parameters are required, and none of
them can be inferred — see [Why all three are required](#why-all-three-are-required).

```bash
triodion erc20_allowances \
    --contract 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 \
    --from-address 0xc4a622e178ec7dd5a839c64de1b474f5d299f64e \
    --to-address 0x111111125421ca6dc452d289314280a0f8842a65
```

`--from-address` is the owner, whose tokens may be spent. `--to-address` is the
spender, permitted to move them. They are named that way so a row here joins
directly against [erc20_approvals](./erc20_approvals.md), which stores the
`Approval` event's indexed owner and spender in columns of the same names.

## Which dataset do you want?

Two datasets answer questions about ERC-20 approvals, and they answer different
ones. Pick by the question, not by the name.

| | [erc20_approvals](./erc20_approvals.md) | erc20_allowances |
|---|---|---|
| Source | `Approval` event log | `allowance()` call |
| Gives | every approval *change* in a block range | the value in force at a block |
| Owner and spender | discovered | must be named |
| Also gives | transaction hash, log index | nothing but the value |
| Answers | "who approved what, and when" | "what can be spent right now" |

The event log alone cannot answer the second question, and the difference is
not a small one.

An allowance also falls when the spender spends. `transferFrom` decrements it
without emitting `Approval` on most implementations, so replaying approval
events overstates what is actually spendable. Only the call reads the truth.

Here is that gap in real data. Both rows are USDC approvals granted in block
25803911, read back at the same block:

| owner | spender | `Approval` event value | allowance at end of block |
|---|---|---|---|
| `0xa8c1…1185` | `0x1111…2a65` | 21838278808 | **0** |
| `0xc4a6…f64e` | `0x1111…2a65` | 12000000000 | 12000000000 |

The first owner approved a router and the router spent it in the same
transaction. The event says 21838278808. Nothing is spendable.

The two datasets compose well: run `erc20_approvals` over a block range to
*find* the pairs, then feed those pairs to `erc20_allowances` to read what
survives.

## Why all three are required

An `allowance()` call takes an owner and a spender. There is no state read that
enumerates the spenders a token has approved, because the keys of a Solidity
mapping are not recoverable from its slots — only `keccak256(key . slot)` is
stored, and that is one-way.

So this dataset cannot discover pairs. A user who does not know them yet wants
`erc20_approvals`, which finds them in the event log.

## Null and zero are different

A successful read of `0` means the spender may move nothing. That is a real
measurement, and it is the state after a revocation.

A null means the call did not return an allowance at all: the address has no
code at this block, or is not an ERC-20.

Filling the null with `0` would report "approved for nothing" where the right
answer is "no such token".

## On "unlimited" approvals

The common way to grant an unbounded allowance is to set it to
`type(uint256).max`. It is a convention, not a rule. Some tokens and front-ends
use `2^255 - 1`, and some decrement from whatever was set.

The `allowance` column is therefore the raw value, with no derived "unlimited"
flag. Testing `== 2^256 - 1` would miss the other conventions, and thresholding
at some round number would be this tool inventing a fact. Threshold it yourself,
in the direction your analysis needs.

## Batching

Every row is one `eth_call`, so `--multicall` aggregates the rows that share a
block into one `aggregate3`:

```bash
triodion erc20_allowances --multicall \
    --contract 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 \
    --from-address 0xc4a622e178ec7dd5a839c64de1b474f5d299f64e \
    --to-address 0x111111125421ca6dc452d289314280a0f8842a65 \
    -b 25803911
```

The `eth_call` state-override path used by [slots](./slots.md) does not apply
here. That trick reads *slots*, and finding the slot of
`allowances[owner][spender]` needs the mapping's base slot, which varies per
token and is not discoverable from the ABI.
