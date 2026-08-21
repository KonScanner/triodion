# codes

The code stored at an address, at a given block.

- can collect by block and not by transaction
- required parameters: address
- optional parameters: [none]
- dataset aliases: [none]
- parameter aliases: contract → address

## Schema

```
schema for codes
────────────────
- block_number: uint32
- address: binary
- code: binary
- chain_id: uint64

sorting codes by: block_number, address

other available columns: is_delegated, delegate_address
```

Run `triodion help codes` to print this from the binary.

`--blocks` defaults to `latest` for this dataset, so a query without a block
range reads current code.

```bash
triodion codes --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2
triodion codes -b 20M:+100 --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2
```

## Code no longer means contract

This is the assumption EIP-7702 broke, and it is worth stating directly because
so much analysis rests on it.

Before Prague, "this address has code" and "this address is a contract" were
the same statement. Since EIP-7702 they are not. An authorization can write a
23-byte **delegation designator** — `0xef0100` followed by an address — to an
*externally owned account*. Calls to that account then execute the named
contract's code, while the account remains an EOA with a private key behind it.

So a classifier that reads "code is non-empty, therefore contract" now counts
every delegated EOA as a contract. Two opt-in columns make the case visible:

| column | meaning |
| :- | :- |
| `is_delegated` | the code is a delegation designator, so this is a delegated EOA |
| `delegate_address` | the contract whose code runs on calls here; null unless `is_delegated` |

```bash
triodion codes -b 25.8M --address <ADDRESS> -i is_delegated delegate_address
```

Both are opt-in, so the default output of this dataset is unchanged from before
they existed.

The length check behind them is not a formality. `0xef` has been an invalid
opcode since EIP-3541 reserved it, so no ordinary contract begins with the
prefix — but a longer body that happens to start with those three bytes is
still not a designator, and its trailing bytes are not an address. Only code of
exactly 23 bytes with exactly that prefix is read as a delegation.

## The other half of EIP-7702

This dataset shows the *state*: what an account delegates to right now. The
[authorizations](./authorizations.md) dataset shows the *events*: who signed
which authorization, in which transaction.

They join on `codes.address` = `authorizations.authority`. A delegation present
here with no matching authorization in the range you collected simply means the
authorization was signed earlier.
