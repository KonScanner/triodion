use alloy::sol;

sol! {
    contract ERC20 {
        function name() external pure returns (string memory);
        function symbol() external pure returns (string memory);
        function decimals() external pure returns (uint8);
        function totalSupply() external view returns (uint);
        function balanceOf(address owner) external view returns (uint);
        function allowance(address owner, address spender) external view returns (uint);

        event Approval(address indexed owner, address indexed spender, uint value);
        event Transfer(address indexed from, address indexed to, uint value);
    }
}

sol! {
    contract ERC721 {
        event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
        event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);
        event ApprovalForAll(address indexed owner, address indexed operator, bool approved);
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function tokenURI(uint256 tokenId) external view returns (string memory);
    }
}

sol! {
    /// Supply-modifying events that ERC-20 wrappers and many DeFi tokens emit
    /// **in addition** to the standard ERC-20 Transfer (which already covers
    /// mints/burns via the zero-address sender/receiver convention). Captured
    /// at the event-signature level — any contract emitting these signatures
    /// is indexed regardless of whether it's a "real" WETH-style wrapper.
    contract ERC20Wrapper {
        /// WETH-style mint: depositing the underlying (ETH for WETH) returns
        /// the wrapped token. `wad` is the amount minted.
        event Deposit(address indexed dst, uint256 wad);

        /// WETH-style burn: redeeming the wrapped token for the underlying.
        /// `wad` is the amount burned.
        event Withdrawal(address indexed src, uint256 wad);
    }
}

sol! {
    /// ERC-1155 multi-token.
    ///
    /// `TransferBatch` carries two parallel arrays rather than scalars, which
    /// is why the transfers dataset explodes one row per token id instead of
    /// storing the arrays whole.
    ///
    /// `ApprovalForAll` is byte-identical to ERC-721's, so a log carrying this
    /// signature does not say which of the two standards the contract
    /// implements.
    contract ERC1155 {
        event TransferSingle(
            address indexed operator,
            address indexed from,
            address indexed to,
            uint256 id,
            uint256 value
        );
        event TransferBatch(
            address indexed operator,
            address indexed from,
            address indexed to,
            uint256[] ids,
            uint256[] values
        );
        event ApprovalForAll(address indexed account, address indexed operator, bool approved);
        event URI(string value, uint256 indexed id);

        function uri(uint256 id) external view returns (string memory);
        function balanceOf(address account, uint256 id) external view returns (uint256);
    }
}

sol! {
    /// ERC-165 standard interface detection.
    contract ERC165 {
        function supportsInterface(bytes4 interfaceId) external view returns (bool);
    }
}

sol! {
    /// ERC-1967 proxy storage slots.
    ///
    /// The slots themselves are constants rather than functions; see the
    /// `proxy_slots` dataset. These are the events a compliant proxy emits when
    /// one of those slots changes.
    contract ERC1967 {
        event Upgraded(address indexed implementation);
        event AdminChanged(address previousAdmin, address newAdmin);
        event BeaconUpgraded(address indexed beacon);
    }
}

sol! {
    /// ERC-4626 tokenised vault.
    ///
    /// `Deposit` here takes four arguments and is a different signature from
    /// the two-argument WETH-style `Deposit` in [`ERC20Wrapper`], so the two
    /// have different topic0 values and cannot be confused.
    contract ERC4626 {
        event Deposit(
            address indexed sender,
            address indexed owner,
            uint256 assets,
            uint256 shares
        );
        event Withdraw(
            address indexed sender,
            address indexed receiver,
            address indexed owner,
            uint256 assets,
            uint256 shares
        );

        function asset() external view returns (address);
        function totalAssets() external view returns (uint256);
        function totalSupply() external view returns (uint256);
        function convertToAssets(uint256 shares) external view returns (uint256);
    }
}

sol! {
    /// ERC-2612 permit: signature-based approvals.
    ///
    /// `permit` itself emits no event of its own — it emits the ordinary ERC-20
    /// `Approval` — so the only on-chain trace of a permit is the calldata
    /// selector and the incremented nonce.
    contract ERC2612 {
        function nonces(address owner) external view returns (uint256);
        function DOMAIN_SEPARATOR() external view returns (bytes32);
        function permit(
            address owner,
            address spender,
            uint256 value,
            uint256 deadline,
            uint8 v,
            bytes32 r,
            bytes32 s
        ) external;
    }
}

sol! {
    /// ERC-777 tokens with operators and data payloads.
    ///
    /// An ERC-777 token also emits a mirrored ERC-20 `Transfer` for every
    /// `Sent`, so counting both without care double-counts the same movement.
    contract ERC777 {
        event Sent(
            address indexed operator,
            address indexed from,
            address indexed to,
            uint256 amount,
            bytes data,
            bytes operatorData
        );
        event Minted(
            address indexed operator,
            address indexed to,
            uint256 amount,
            bytes data,
            bytes operatorData
        );
        event Burned(
            address indexed operator,
            address indexed from,
            uint256 amount,
            bytes data,
            bytes operatorData
        );
        event AuthorizedOperator(address indexed operator, address indexed tokenHolder);
        event RevokedOperator(address indexed operator, address indexed tokenHolder);
    }
}
