# Dataset Reference

Every dataset triodion can collect. Run `triodion help datasets` to print this
list from the binary, and `triodion help <DATASET>` for one dataset's schema,
parameters and sort order.

Most dataset pages are still stubs — they carry the schema and little else.
`triodion help <DATASET>` prints the same schema from the binary, which is the
authority when the two disagree.

## Datasets

| dataset | aliases | description |
| --- | --- | --- |
| [access_lists](./access_lists.md) | | EIP-2930 access-list entries, one row per storage key |
| [address_appearances](./address_appearances.md) | | every address that appears in a block, with the relationship that made it appear |
| [authorizations](./authorizations.md) | | EIP-7702 authorization tuples, with the recovered authority |
| [balance_diffs](./balance_diffs.md) | | ETH balance changes, as before and after values |
| [balance_reads](./balance_reads.md) | | ETH balances read during execution |
| [balances](./balances.md) | | ETH balance of an address at a block |
| [blobs](./blobs.md) | | one row per EIP-4844 blob, read from the consensus layer |
| [blocks](./blocks.md) | | one row per block header |
| [code_diffs](./code_diffs.md) | | contract code changes, as before and after values |
| [code_reads](./code_reads.md) | | contract code read during execution |
| [codes](./codes.md) | | contract code at a block |
| [consolidation_requests](./consolidation_requests.md) | | EIP-7251 consolidation requests, from the consensus layer |
| [contracts](./contracts.md) | | contracts deployed, with deployer, factory and init code |
| [deposit_requests](./deposit_requests.md) | | EIP-6110 deposit requests, from the consensus layer |
| [erc20_balances](./erc20_balances.md) | | ERC-20 balance of an address at a block |
| [erc20_metadata](./erc20_metadata.md) | | ERC-20 name, symbol and decimals |
| [erc20_supplies](./erc20_supplies.md) | | ERC-20 total supply at a block |
| [erc20_transfers](./erc20_transfers.md) | | ERC-20 `Transfer` events |
| [erc20_approvals](./erc20_approvals.md) | | ERC-20 `Approval` events |
| [erc20_wrapper_events](./erc20_wrapper_events.md) | `wrapper_events`, `weth_events` | WETH-shape `Deposit` and `Withdrawal` events |
| [erc721_metadata](./erc721_metadata.md) | | ERC-721 name and symbol |
| [erc721_transfers](./erc721_transfers.md) | | ERC-721 `Transfer` events, with token id |
| [eth_calls](./eth_calls.md) | | `eth_call` outputs for a contract and call data at each block |
| [four_byte_counts](./four_byte_counts.md) | `4byte_counts` | count of each four-byte function selector called |
| [geth_calls](./geth_calls.md) | | call traces from geth's `callTracer` |
| [geth_code_diffs](./geth_code_diffs.md) | | contract code changes from geth's state diffs |
| [geth_balance_diffs](./geth_balance_diffs.md) | | ETH balance changes from geth's state diffs |
| [geth_storage_diffs](./geth_storage_diffs.md) | | storage slot changes from geth's state diffs |
| [geth_nonce_diffs](./geth_nonce_diffs.md) | | nonce changes from geth's state diffs |
| [geth_opcodes](./geth_opcodes.md) | | opcode-level steps from geth's struct logger |
| [javascript_traces](./javascript_traces.md) | `js_traces` | raw output of a custom javascript tracer |
| [logs](./logs.md) | `events` | event logs, optionally decoded into typed columns |
| [native_transfers](./native_transfers.md) | | ETH transfers, including those inside traces |
| [nonce_diffs](./nonce_diffs.md) | | nonce changes, as before and after values |
| [nonce_reads](./nonce_reads.md) | | nonces read during execution |
| [nonces](./nonces.md) | | nonce of an address at a block |
| [slots](./slots.md) | `storages` | value of a storage slot at a block |
| [storage_diffs](./storage_diffs.md) | `slot_diffs` | storage slot changes, as before and after values |
| [storage_reads](./storage_reads.md) | `slot_reads` | storage slots read during execution |
| [traces](./traces.md) | | parity-style call traces |
| [trace_calls](./trace_calls.md) | | `trace_call` results, alongside the call that produced them |
| [transactions](./transactions.md) | `txs` | one row per transaction |
| [vm_traces](./vm_traces.md) | `opcode_traces` | parity `vmTrace` opcode steps |
| [withdrawal_requests](./withdrawal_requests.md) | | EIP-7002 withdrawal requests, from the consensus layer |
| [withdrawals](./withdrawals.md) | | EIP-4895 validator withdrawals, one row each |

## Datasets that read the consensus layer

Five datasets are not served by an execution node at all.

| dataset | needs | why |
| --- | --- | --- |
| [blobs](./blobs.md) | `--beacon-rpc` or `--blob-archive` | the execution layer sees only a blob's versioned hash, never the blob |
| [deposit_requests](./deposit_requests.md) | `--beacon-rpc` | EIP-7685 puts `requests_hash` in the header and the requests nowhere |
| [withdrawal_requests](./withdrawal_requests.md) | `--beacon-rpc` | as above |
| [consolidation_requests](./consolidation_requests.md) | `--beacon-rpc` | as above |

[withdrawals](./withdrawals.md) is the exception that looks like it belongs
here and does not: EIP-4895 withdrawals are in the execution block body, so an
ordinary RPC url is enough.

## Dataset group names

A group name collects several datasets in one command.

| group | datasets |
| --- | --- |
| `blocks_and_transactions` | `blocks`, `transactions` |
| `call_trace_derivatives` | `contracts`, `native_transfers`, `traces` |
| `geth_state_diffs` | `geth_balance_diffs`, `geth_code_diffs`, `geth_nonce_diffs`, `geth_storage_diffs` |
| `log_events` | `logs`, `erc20_transfers`, `erc20_approvals`, `erc721_transfers`, `erc20_wrapper_events` |
| `state_diffs` | `balance_diffs`, `code_diffs`, `nonce_diffs`, `storage_diffs` |
| `state_reads` | `balance_reads`, `code_reads`, `nonce_reads`, `storage_reads` |

```bash
triodion blocks_and_transactions -b 18M:18.001M
```

## Notes

Some datasets need more than an execution-layer RPC url.

- `blobs` reads the consensus layer. It needs `--beacon-rpc`, and `--blob-archive` for anything
  older than about 18 days. See [blobs](./blobs.md).
- `traces`, `trace_calls`, `vm_traces`, `contracts`, `native_transfers` and the `state_diffs` group
  use parity-style `trace_*` methods.
- the `geth_*` datasets, `javascript_traces` and the `state_reads` group use geth's
  `debug_traceBlock*` methods, with the matching tracer enabled on the node.
