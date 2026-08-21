# Request Batching

Most datasets read one thing per row: one storage slot, one balance, one header.
The obvious way to collect a partition is one RPC per row, and that is what
triodion did. It is also what makes a large query slow, and what gets a public
endpoint to answer `429`.

Triodion now takes the best route each dataset and endpoint allow. Every route
is **on by default**, and every route falls back to the per-row path when it
fails — so batching changes how many requests your results cost, never the
results themselves.

## The switches

| Flag | Turns off | Applies to |
|---|---|---|
| `--no-batch-rpc-calls` | JSON-RPC request batching | `blocks`, `nonces`, `codes` |
| `--no-batch-state-reads` | `eth_call` state overrides | `slots`, `balances`, `proxy_slots` |
| `--state-override-batch-size N` | (sets size, 0 = default 1000) | `slots`, `balances`, `proxy_slots` |
| `--no-multicall` | Multicall3 aggregation | `eth_calls`, the ERC-20 and vault read datasets |

Multicall3 is on by default like the other two, so the flag that changes
anything is `--no-multicall`. A bare `--multicall` is still accepted, as a
no-op, so older scripts keep working.

They are separate switches because they ask different things of the node. A
JSON-RPC batch needs nothing beyond JSON-RPC itself. A state override needs an
endpoint that honours the third `eth_call` parameter, which not every endpoint
does. An operator who distrusts an endpoint's override support has no reason to
give up plain batching as well.

## JSON-RPC batching

JSON-RPC has always allowed an array of requests in one HTTP body. `blocks`,
`nonces` and `codes` use it, because none of them has a better route available.

Measured on `blocks -b 21000000:+500` through a counting proxy in front of a
public mainnet endpoint:

| | HTTP requests | embedded calls | wall clock |
|---|---|---|---|
| `--no-batch-rpc-calls` | 501 | 501 | 0.96s |
| default | **6** | 501 | **0.46s** |

The two parquet files are byte-identical.

The request count is the reliable number: 83× fewer, and it follows from the
batch size rather than from the endpoint. The wall-clock figure does not
generalise — this endpoint was fast and did not throttle, so batching only won
2×. The further an endpoint is, and the harder it rate-limits, the more of the
501 round trips batching removes and the wider the gap gets. That is also the
case where the per-request path stops finishing at all.

The batch size negotiates itself downward when a provider refuses one — OP
Mainnet answers `413` above ten calls, Base returns
`-32014 "maximum 10 calls in 1 batch"` — and the discovered size is kept for the
rest of the call rather than rediscovered per batch.

`codes` uses a smaller batch (50) because whole contract bytecode is far larger
than a 32-byte word, and a hundred EIP-170-sized contracts is a 2.4MB response.

`nonces` is the one account field that can *only* batch this way. See
[nonces](../datasets/nonces.md).

## State overrides

`eth_call` takes a third parameter, a state-override set, that applies changes to
a scratch copy of state before execution. Overriding a contract's **code** with a
small extractor loop turns "read one slot" into "read every slot I name".

There is no Multicall3 equivalent for storage, because a deployed contract
cannot read another contract's slots. Bytecode *injected into* the target can
read all of them.

The same 25-byte loop with one opcode swapped serves four readers: `SLOAD` for
storage, and `BALANCE` / `EXTCODEHASH` / `EXTCODESIZE` for accounts. The account
readers take their address from the stack, so they run at a scratch address and
batch across unrelated addresses without overriding any real contract.

### Historical blocks

`eth_call` at a historical block executes under *that block's* fork rules.
Triodion's extractor is deliberately `PUSH0`-free, because `PUSH0` only became
valid at Shanghai (mainnet block 17034870, April 2023). Below that a `PUSH0`
extractor is rejected outright — geth reports `invalid opcode: PUSH0`, reth
reports `EVM error: NotActivated`.

The published 23-byte extractor from Dedaub's write-up of this technique is the
`PUSH0` form. Triodion emits a 25-byte variant instead, verified byte-identical
to `eth_getStorageAt` at latest, block 15000000 and block 12000000. There is no
fork detection to get wrong.

## The ladder

For `slots`, triodion tries three things in order. Measured on 4 contracts ×
6 slots at one block, through a proxy that denies methods on demand:

| rung | condition | requests |
|---|---|---|
| 1 | `eth_getStorageValues` available | **1** |
| 2 | native denied, overrides honoured | 4 `eth_call` + 1 probe |
| 3 | both denied | 24 `eth_getStorageAt` |

All three produce identical output.

**Rung 1** is `eth_getStorageValues`, added in geth v1.17.1. It does natively
what the extractor does by injection — many slots, across many contracts, one
round trip — with no override and none of the override failure modes. Because it
takes a `{contract: [slots]}` map, slots for *different* contracts at the same
block travel in one request. That is the one thing the extractor cannot do at
any batch size, since injected code reads only its own address's storage.

Support for it cannot be cached. A load-balanced pool holds a mix of node
versions: the same URL served this method on one request and returned
`Method not found` on the next. So triodion counts *consecutive* misses rather
than latching a verdict, and a mixed pool keeps using it whenever it works.

**Rung 2** is the extractor, one `eth_call` per contract.

**Rung 3** is the per-row path, exactly as it was before any of this existed.

## Verification

An endpoint that *accepts* the override parameter and silently ignores it is the
dangerous case. Our slot-key calldata would reach the real contract, and its
fallback output would be decoded as storage — plausible values, right shape,
entirely fabricated, written to a Parquet file that looks correct forever.

Two things prevent that:

1. The return must be **exactly** `32 × n` bytes. The extractor returns one word per calldata word by construction, so the length is positive proof that our code produced the answer.
2. The storage reader also sets a sentinel slot via `stateDiff` and appends that slot as the final calldata word. Getting the sentinel value back in the final returned word proves our bytecode ran, that the override was applied, and that words came back in order.

Anything else is an error, never a partial result — and an error only routes
those rows to the per-row path.

Note that `stateDiff` is used, never `state`. `state` replaces an account's
entire storage, which would return a perfectly well-formed file of zeros.

The sentinel slot is itself the one slot this path cannot read, because it is
the one slot this path writes. A row that asks for it would come back holding
the sentinel value, with the length check and the sentinel check both passing.
So a row naming that slot skips the batch and is read by `eth_getStorageAt`,
with no override in sight. The account readers have the same blind spot at the
scratch address they run at, and it is guarded the same way.
