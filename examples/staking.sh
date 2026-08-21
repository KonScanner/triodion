#!/usr/bin/env bash

# Validator lifecycle: deposits in, withdrawals out, exits and consolidations.
#
# Four datasets cover the money and the intent, and they do NOT all come from
# the same place. That distinction is the point of this script.
#
#   withdrawals             EIP-4895   execution block body   RPC only
#   deposit_requests        EIP-6110   consensus block        needs --beacon-rpc
#   withdrawal_requests     EIP-7002   consensus block        needs --beacon-rpc
#   consolidation_requests  EIP-7251   consensus block        needs --beacon-rpc
#
# EIP-7685 puts a single requests_hash in the execution header and the requests
# themselves nowhere. eth_getBlockByNumber returns the commitment and nothing
# else, and a hash cannot be turned back into what it commits to. So the three
# request datasets read the beacon block and require --beacon-rpc.
#
# There is no archive fallback for them and none is needed: beacon blocks are
# not pruned the way blob sidecars are. The node-side limit is different -- a
# checkpoint-synced beacon node without backfill holds nothing before its
# checkpoint and answers 404, which triodion reports as an error naming that
# cause rather than as an empty result.
#
# withdrawals is the odd one out. It looks like a beacon dataset and is not:
# EIP-4895 withdrawals sit in the execution block body, so an ordinary RPC url
# is enough.

#
# # parameters
#

EXECUTABLE=triodion

# Post-Prague, or the three request datasets write no rows -- correctly, since
# execution requests did not exist before then.
BLOCKS="25800355:+50"
OUTPUT_DIR="data/staking"

# BEACON must be a consensus-layer REST endpoint, not a JSON-RPC url.
# Lighthouse and Nimbus default to :5052, Prysm to :3500.
BEACON="${BEACON_RPC_URL:-http://localhost:5052}"


#
# # execution layer: the payments
#

# One row per validator withdrawal. `blocks` carries only withdrawals_count and
# withdrawals_amount_gwei, and an aggregate cannot be taken apart again.
#
# Note the units. amount_gwei is gwei, which is what the protocol uses here;
# reading it as wei understates every payout by a factor of a billion. Add
# amount_wei if you need to join against value columns, which are in wei.
$EXECUTABLE withdrawals -b $BLOCKS -o $OUTPUT_DIR/withdrawals

$EXECUTABLE withdrawals -b $BLOCKS -o $OUTPUT_DIR/withdrawals_wei -i amount_wei

# A withdrawal has no sender, no gas cost, no receipt and no transaction hash,
# and it never executes -- so it appears in NO other dataset. An ETH-flow
# analysis built from traces and native_transfers alone is missing every
# validator payout on the chain.


#
# # consensus layer: the requests
#

# EIP-6110 deposits. withdrawal_credentials is 32 bytes whose first byte states
# the kind: 0x00 BLS, 0x01 execution address, 0x02 compounding. Only 0x01 and
# 0x02 contain an address, and withdrawal_address is null for 0x00 -- those
# bytes are a hashed BLS key, and reading 20 of them as an address would give a
# well-formed address belonging to nobody.
#
# withdrawal_address is the join key to the withdrawals dataset above.
$EXECUTABLE deposit_requests -b $BLOCKS -o $OUTPUT_DIR/deposit_requests \
    --beacon-rpc "$BEACON"

# EIP-7002 exits and partial withdrawals, triggered from the execution layer so
# that the holder of the withdrawal credentials can exit a validator without
# the validator key.
#
# TRAP: amount_gwei of 0 is a FULL EXIT, not an empty request. Summing that
# column reports zero for the largest withdrawals on the chain. is_full_exit is
# emitted beside it for exactly that reason.
$EXECUTABLE withdrawal_requests -b $BLOCKS -o $OUTPUT_DIR/withdrawal_requests \
    --beacon-rpc "$BEACON"

# EIP-7251 consolidations.
#
# TRAP: when source_pubkey == target_pubkey the request is not a consolidation
# at all. It upgrades that one validator's credentials from 0x01 to 0x02, the
# compounding kind, and moves no stake. These were the majority of such
# requests in the weeks after Prague, so a count that treats every row as a
# merge is wrong by a wide margin. Filter on is_credential_upgrade.
$EXECUTABLE consolidation_requests -b $BLOCKS -o $OUTPUT_DIR/consolidations \
    --beacon-rpc "$BEACON"


#
# # deposits before Prague
#
# EIP-6110 changed how a deposit reaches the beacon chain, not what a deposit
# is. Before Prague the consensus layer discovered deposits by voting on
# execution state; both forms originate in the same deposit-contract call. So
# for older history, read the contract's logs instead -- this works for the
# whole history of the chain and needs no beacon node.

$EXECUTABLE logs -b 20M:+1000 -o $OUTPUT_DIR/deposit_logs \
    --address 0x00000000219ab540356cBB839Cbe05303d7705Fa
