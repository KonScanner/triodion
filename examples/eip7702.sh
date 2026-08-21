#!/usr/bin/env bash

# EIP-7702: account delegation, in two halves.
#
# An authorization is a signed statement by an externally owned account that
# calls to it should execute another contract's code. Applying one writes a
# 23-byte delegation designator -- 0xef0100 followed by the delegate address --
# as that account's code.
#
# So the event and the state live in different datasets:
#
#   authorizations   who signed what, and in which transaction
#   codes            what an account delegates to right now
#
# They join on authorizations.authority = codes.address.

#
# # parameters
#

EXECUTABLE=triodion

# Post-Prague. Type-0x04 transactions do not exist before it.
BLOCKS="25800355:+10"
OUTPUT_DIR="data/eip7702"


#
# # the authorizations
#

# One row per authorization tuple. transactions carries only n_authorizations,
# which is a count: it says a transaction carried three, not which accounts
# delegated or to what code.
$EXECUTABLE authorizations -b $BLOCKS -o $OUTPUT_DIR/authorizations

# The signature parts are opt-in.
$EXECUTABLE authorizations -b $BLOCKS -o $OUTPUT_DIR/authorizations_sig \
    -i y_parity r s

# SUBMITTED IS NOT APPLIED. A row here was included in a block. The protocol
# applies it only if the authority's nonce and chain id still match at
# execution time; a stale one is skipped and the carrying transaction still
# succeeds. Nothing in the transaction's data records which happened, so this
# dataset does not claim to. Compare the row's nonce against the authority's
# account nonce to tell them apart.
#
# Measured example: in mainnet block 25,800,355, authority
# 0xc4cbdbc0988fd5f419d5ed787ec2743b84be0d0b carries an authorization requiring
# nonce 632 while the account sits at 631. It was included and skipped, and the
# account kept the delegation it already had. 21 of that block's 22
# authorizations applied; this one did not.

# Two values that look like nulls and are not:
#   delegate_address of all zeros    -- the defined way to CLEAR a delegation
#   authorization_chain_id of 0      -- valid on EVERY chain, not "unknown"


#
# # the resulting state
#

# Since EIP-7702, "this address has code" no longer means "this address is a
# contract". A classifier that reads code-is-non-empty therefore counts every
# delegated EOA as a contract.
#
# is_delegated and delegate_address make the case visible. Both are opt-in, so
# the default output of `codes` is unchanged from before they existed.
$EXECUTABLE codes -b 25800355 -o $OUTPUT_DIR/codes \
    --address 0xbe736031f38b5843cd09fc9706984de5e2fdde1b \
    -i is_delegated delegate_address

# That address is a real delegated EOA at that block, and its delegate_address
# matches the authorization above -- which is the join working.


#
# # EIP-2930 access lists, while we are here
#

# transactions carries n_access_list_addresses and n_access_list_storage_keys.
# Counts again. access_lists explodes each list to one row per storage key.
#
# An entry that names an account and no storage keys is still a real entry --
# it warms the account and it costs gas -- so it gets a row with a null
# storage_key. Null there means "this entry listed no slots", never "unknown".
$EXECUTABLE access_lists -b $BLOCKS -o $OUTPUT_DIR/access_lists

# An access list is a DECLARATION made before execution, not a record of what
# execution touched. A listed slot may never be read, and a slot that is read
# may be absent from the list. For what was actually touched, use
# storage_reads and balance_reads.
