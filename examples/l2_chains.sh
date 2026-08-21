#!/usr/bin/env bash

# Collect blocks and transactions from the OP stack and the Arbitrum stack.
#
# Both stacks add EIP-2718 transaction type bytes that Ethereum does not
# define: 0x7e for an OP deposit, 0x64-0x6a and 0x78 for the Arbitrum types.
# Upstream modelled only Ethereum's 0x00-0x04, so a single unknown type byte
# failed the whole eth_getBlockByNumber response and the block returned no
# rows at all. Every OP-stack block opens with the L1-attributes deposit and
# every Arbitrum block with an internal transaction, so every block failed.
# triodion reads these chains through alloy's AnyNetwork, which keeps the
# unknown fields verbatim, so the shared columns are read the same way on
# every chain and the stack-specific ones are read by name.
#
# All of the columns below are opt-in. The default schema is the same on
# every chain, so an existing pipeline keeps its exact output layout.

#
# # parameters
#

# use installed triodion installation
EXECUTABLE=triodion

# use local cargo repo
# EXECUTABLE="cargo run --"

export OUTPUT_DIR="data"

# Public endpoints. They serve full history but are shared and slow.
export OP_RPC="https://mainnet.optimism.io"
export BASE_RPC="https://mainnet.base.org"
export ARBITRUM_RPC="https://arb1.arbitrum.io/rpc"

# Two blocks each. Raise these to any range your endpoint serves; the values
# below are chosen only because they are far behind every chain's head.
export OP_BLOCKS="110M:+2"
export BASE_BLOCKS="15M:+2"
export ARBITRUM_BLOCKS="200M:+2"

# Free public endpoints rate-limit hard, and the receipt fetch is the part
# that hurts: transactions asks for one receipt per transaction, so a busy
# Base block is thousands of calls. Without a limit you get HTTP 429 and
# partial data. Start low and raise it until the errors come back.
#
# Batch size needs no flag. These endpoints cap JSON-RPC batches at ten calls
# (OP Mainnet answers HTTP 413, Base answers -32014 "maximum 10 calls in 1
# batch"), and triodion splits an oversized batch and retries it. A rate
# limit is excluded from that, because sending more, smaller requests to a
# node that just asked for less makes it worse.
export RPS=5


#
# # OP stack: OP Mainnet
#

# blob_gas_used / excess_blob_gas / parent_beacon_block_root arrive with
# Ecotone, which is the OP stack tracking Ethereum's Cancun header.
$EXECUTABLE blocks \
    --blocks $OP_BLOCKS \
    --rpc $OP_RPC \
    --requests-per-second $RPS \
    --output-dir $OUTPUT_DIR/op_blocks \
    --include-columns blob_gas_used excess_blob_gas parent_beacon_block_root

# source_hash / mint / is_system_tx / deposit_receipt_version are the deposit
# (0x7e) fields; they are null on ordinary transactions. The l1_* columns come
# from the receipt and record what the transaction paid to post its data to
# L1. chain_family says which stack defined the type byte: "ethereum",
# "op_stack", "arbitrum" or "unknown".
#
# A deposit is not signed, so r, s, v and y_parity are null on those rows
# rather than the zeros the node reports.
$EXECUTABLE transactions \
    --blocks $OP_BLOCKS \
    --rpc $OP_RPC \
    --requests-per-second $RPS \
    --output-dir $OUTPUT_DIR/op_transactions \
    --include-columns \
        chain_family tx_chain_id y_parity \
        source_hash mint is_system_tx deposit_receipt_version \
        l1_fee l1_gas_used l1_gas_price l1_fee_scalar \
        l1_blob_base_fee l1_base_fee_scalar l1_blob_base_fee_scalar \
        operator_fee_scalar operator_fee_constant


#
# # OP stack: Base
#
# Same schema as OP Mainnet — Base is the same stack. Base blocks are busy,
# so this is where the receipt rate limit bites first.
#
# One --include-columns list serves both datasets. A named column that the
# dataset does not define is skipped rather than rejected, so the block
# columns go to blocks and the transaction columns go to transactions.

$EXECUTABLE blocks transactions \
    --blocks $BASE_BLOCKS \
    --rpc $BASE_RPC \
    --requests-per-second $RPS \
    --output-dir $OUTPUT_DIR/base \
    --subdirs datatype \
    --include-columns \
        blob_gas_used excess_blob_gas parent_beacon_block_root \
        chain_family source_hash mint is_system_tx deposit_receipt_version \
        l1_fee l1_gas_used l1_gas_price


#
# # Arbitrum stack: Arbitrum One
#

# l1_block_number is the L1 block this L2 block was sequenced against.
# send_root and send_count track the L2 -> L1 outbox accumulator.
$EXECUTABLE blocks \
    --blocks $ARBITRUM_BLOCKS \
    --rpc $ARBITRUM_RPC \
    --requests-per-second $RPS \
    --output-dir $OUTPUT_DIR/arbitrum_blocks \
    --include-columns l1_block_number send_root send_count

# gas_used_for_l1 / gas_used_for_l2 split Arbitrum's gas accounting.
# request_id, ticket_id and refund_to belong to the retryable-ticket types.
# The internal transaction (0x6a) that opens every block is unsigned, so its
# r, s, v and y_parity are null.
$EXECUTABLE transactions \
    --blocks $ARBITRUM_BLOCKS \
    --rpc $ARBITRUM_RPC \
    --requests-per-second $RPS \
    --output-dir $OUTPUT_DIR/arbitrum_transactions \
    --include-columns \
        chain_family tx_chain_id y_parity \
        gas_used_for_l1 gas_used_for_l2 request_id ticket_id refund_to


#
# # note on the v column
#
# v is not a default column, so nothing above collects it. If you add it,
# read it as the EIP-155 scalar: 27 or 28 unprotected, chain_id * 2 + 35 +
# parity when replay-protected, 0 or 1 for typed transactions. It used to be
# a bool holding the parity bit, which lost the chain id. The parity bit is
# now the separate y_parity column.
