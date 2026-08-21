use crate::{datasets::*, define_datatypes, types::columns::ColumnData, ColumnType, *};
use polars::prelude::*;
use std::collections::HashMap;

define_datatypes!(
    AccessLists,
    AddressAppearances,
    ApprovalsForAll,
    Authorizations,
    BalanceDiffs,
    BalanceReads,
    Balances,
    Blobs,
    Blocks,
    CodeDiffs,
    CodeReads,
    Codes,
    ConsolidationRequests,
    ContractInterfaces,
    Contracts,
    DepositRequests,
    Erc1155Metadata,
    Erc1155Transfers,
    Erc20Allowances,
    Erc20Balances,
    Erc20Metadata,
    Erc20Supplies,
    Erc20Transfers,
    Erc20Approvals,
    Erc20WrapperEvents,
    Erc2612Nonces,
    Erc4626Metadata,
    Erc4626VaultEvents,
    Erc721Metadata,
    Erc721Transfers,
    Erc777Transfers,
    EthCalls,
    FourByteCounts,
    GethCalls,
    GethCodeDiffs,
    GethBalanceDiffs,
    GethStorageDiffs,
    GethNonceDiffs,
    GethOpcodes,
    JavascriptTraces,
    Logs,
    NativeTransfers,
    NonceDiffs,
    NonceReads,
    Nonces,
    ProxySlots,
    ProxyUpgrades,
    Slots,
    StorageDiffs,
    StorageReads,
    Traces,
    TraceCalls,
    Transactions,
    VmTraces,
    WithdrawalRequests,
    Withdrawals,
);

impl Datatype {
    fn alias_map() -> Result<HashMap<String, Datatype>, ParseError> {
        let mut map = HashMap::new();
        for datatype in Datatype::all() {
            let key = datatype.name();
            if map.contains_key(&key) {
                return Err(ParseError::ParseError("conflict in datatype names".to_string()))
            }
            map.insert(key, datatype);
            for key in datatype.aliases().into_iter() {
                if map.contains_key(key) {
                    return Err(ParseError::ParseError("conflict in datatype names".to_string()))
                }
                map.insert(key.to_owned(), datatype);
            }
        }
        Ok(map)
    }
}

impl std::str::FromStr for Datatype {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Datatype, ParseError> {
        let mut map = Datatype::alias_map()?;
        map.remove(s)
            .ok_or_else(|| ParseError::ParseError(format!("no datatype matches input: {}", s)))
    }
}
