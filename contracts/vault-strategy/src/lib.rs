#![no_std]
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, panic_with_error,
    symbol_short, token, vec, Address, BytesN, Env, FromVal, IntoVal, Map, MuxedAddress, String, Symbol, SymbolStr,
    Vec,
};
// Audit W2/W4.2: the share ledger (balances, allowances, total supply) uses the
// OpenZeppelin stellar-tokens storage primitives: per-holder persistent
// balance entries and a persistent total supply with TTL extension, and
// temporary allowance entries whose `live_until_ledger` is validated. Only the
// non-emitting primitives are used; events stay the golden-pinned SEP-41 shapes
// below. SEP-41 v0.5 allows either a single-value/vec or a map data format;
// OZ 0.8.0 emits maps, this vault emits single-value/vec - the same format as
// the Stellar Asset Contract - and its bytes are pinned by golden/events.txt.
use stellar_tokens::fungible::{
    total_supply::{decrease_total_supply, increase_total_supply, total_supply as share_supply},
    Base, INSTANCE_EXTEND_AMOUNT, INSTANCE_TTL_THRESHOLD,
};
// Audit W4.2b: SEP-56 asset movement and operator semantics come from the
// OpenZeppelin vault module (`deposit_internal` / `withdraw_internal`). Its
// single-NAV share maths is not used: deposits price at the median NAV and
// redemptions at the conservative NAV (VAULT-DESIGN dual NAV).
use stellar_tokens::vault::Vault as OzVault;

// Events (audit W4.1b: typed #[contractevent]s, byte-identical to the former
// Events::publish encodings; pinned by golden/events.txt). Lower-case names are
// defined by SEP-41 (transfer, approve, mint, burn) and SEP-56 (deposit,
// withdraw); everything else follows AGENT-RULES Rule 8.

/// SEP-41: topics [transfer, from, to]; data amount.
#[contractevent(topics = ["transfer"], data_format = "single-value")]
pub struct Transfer {
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub amount: i128,
}

/// SEP-41 (v0.5) transfer to a muxed account: topics [transfer, from, to]
/// (`to` is the underlying address); data map {amount, to_muxed_id}.
/// Same layout as OpenZeppelin stellar-tokens `MuxedTransfer`.
#[contractevent(topics = ["transfer"], data_format = "map")]
pub struct MuxedTransfer {
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub to_muxed_id: Option<u64>,
    pub amount: i128,
}

/// SEP-41: topics [approve, from, spender]; data [amount, live_until_ledger] (audit W6).
#[contractevent(topics = ["approve"], data_format = "vec")]
pub struct Approve {
    #[topic]
    pub from: Address,
    #[topic]
    pub spender: Address,
    pub amount: i128,
    pub live_until_ledger: u32,
}

/// SEP-41: topics [mint, to]; data amount.
#[contractevent(topics = ["mint"], data_format = "single-value")]
pub struct Mint {
    #[topic]
    pub to: Address,
    pub amount: i128,
}

/// SEP-41: topics [burn, from]; data amount.
#[contractevent(topics = ["burn"], data_format = "single-value")]
pub struct Burn {
    #[topic]
    pub from: Address,
    pub amount: i128,
}

/// Topics: [REBALANCE_STEP, proposer]. Data: [asset_in, asset_out, spent, received] (audit W1).
#[contractevent(topics = ["REBALANCE_STEP"], data_format = "vec")]
pub struct RebalanceStep {
    #[topic]
    pub proposer: Address,
    pub asset_in: Address,
    pub asset_out: Address,
    pub spent: i128,
    pub received: i128,
}

/// Topics: [REBALANCE_COMPLETE, proposer]. Data: number of executed steps.
#[contractevent(topics = ["REBALANCE_COMPLETE"], data_format = "single-value")]
pub struct RebalanceComplete {
    #[topic]
    pub proposer: Address,
    pub steps: u32,
}

/// Topics: [ROUTER_PROPOSED, route]. Data: [router, effective_at].
#[contractevent(topics = ["ROUTER_PROPOSED"], data_format = "vec")]
pub struct RouterProposed {
    #[topic]
    pub route: u32,
    pub router: Address,
    pub effective_at: u64,
}

/// SEP-56 v0.1.2 `Deposit`: topics [deposit, operator, from, receiver]; data
/// map {assets, shares} - the encoding of the standard's own #[contractevent]
/// struct (default map format) and of the OpenZeppelin reference implementation.
#[contractevent(topics = ["deposit"])]
pub struct Deposit {
    #[topic]
    pub operator: Address,
    #[topic]
    pub from: Address,
    #[topic]
    pub receiver: Address,
    pub assets: i128,
    pub shares: i128,
}

/// SEP-56 v0.1.2 `Withdraw`: topics [withdraw, operator, receiver, owner]; data
/// map {assets, shares} (see `Deposit`).
#[contractevent(topics = ["withdraw"])]
pub struct Withdraw {
    #[topic]
    pub operator: Address,
    #[topic]
    pub receiver: Address,
    #[topic]
    pub owner: Address,
    pub assets: i128,
    pub shares: i128,
}

/// Topics [FEE_ACCRUAL, treasury]; data shares minted to the treasury.
#[contractevent(topics = ["FEE_ACCRUAL"], data_format = "single-value")]
pub struct FeeAccrual {
    #[topic]
    pub treasury: Address,
    pub shares: i128,
}

/// Topics [VAULT_INITIALIZED, admin]; data ledger timestamp.
#[contractevent(topics = ["VAULT_INITIALIZED"], data_format = "single-value")]
pub struct VaultInitialized {
    #[topic]
    pub admin: Address,
    pub timestamp: u64,
}

/// Topics [CAPS_LOWERED, admin]; data (tvl_cap, per_user_cap).
#[contractevent(topics = ["CAPS_LOWERED"], data_format = "vec")]
pub struct CapsLowered {
    #[topic]
    pub admin: Address,
    pub tvl_cap: i128,
    pub per_user_cap: i128,
}

/// Topics [SLEEVES_UPDATED, admin]; data number of sleeves.
#[contractevent(topics = ["SLEEVES_UPDATED"], data_format = "single-value")]
pub struct SleevesUpdated {
    #[topic]
    pub admin: Address,
    pub count: u32,
}

/// Topics [VAULT_PAUSED, by]; data ledger timestamp (by = pause authority or admin).
#[contractevent(topics = ["VAULT_PAUSED"], data_format = "single-value")]
pub struct VaultPaused {
    #[topic]
    pub by: Address,
    pub timestamp: u64,
}

/// Topics [VAULT_UNPAUSED, admin]; data ledger timestamp.
#[contractevent(topics = ["VAULT_UNPAUSED"], data_format = "single-value")]
pub struct VaultUnpaused {
    #[topic]
    pub admin: Address,
    pub timestamp: u64,
}

/// Topics [UPGRADE_PROPOSED, admin]; data (wasm_hash, executable_at).
#[contractevent(topics = ["UPGRADE_PROPOSED"], data_format = "vec")]
pub struct UpgradeProposed {
    #[topic]
    pub admin: Address,
    pub wasm_hash: BytesN<32>,
    pub executable_at: u64,
}

/// Topics [UPGRADE, admin]; data wasm hash.
#[contractevent(topics = ["UPGRADE"], data_format = "single-value")]
pub struct Upgrade {
    #[topic]
    pub admin: Address,
    pub wasm_hash: BytesN<32>,
}

/// Topics [UPGRADE_CANCELLED, admin]; data wasm hash.
#[contractevent(topics = ["UPGRADE_CANCELLED"], data_format = "single-value")]
pub struct UpgradeCancelled {
    #[topic]
    pub admin: Address,
    pub wasm_hash: BytesN<32>,
}

/// Topics [POLICY_BOUND, owner]; data policy hash.
#[contractevent(topics = ["POLICY_BOUND"], data_format = "single-value")]
pub struct PolicyBound {
    #[topic]
    pub owner: Address,
    pub policy_hash: BytesN<32>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    VaultPaused = 4,
    ZeroAmount = 5,
    InsufficientBalance = 6,
    InsufficientAllowance = 7,
    ExceedsRedemptionCapacity = 8,
    LiquidityFloorBreached = 9,
    OracleUnavailable = 10,
    AssetNotRegistered = 11,
    InvalidBps = 12,
    SleeveNotFound = 13,
    MathOverflow = 14,
    /// Sleeve holdings are recorded only by value-moving flows; they can never be set directly.
    BalanceNotSettable = 15,
    /// A sleeve cannot be removed while the vault still holds its asset.
    SleeveHasHoldings = 16,
    /// Malformed sleeve set (duplicate asset, cash sleeve not the numéraire, not exactly one cash sleeve).
    InvalidSleeveConfig = 17,
    /// Deposit would push total vault value above `tvl_cap` (AGENT-RULES Rule 6).
    TvlCapExceeded = 18,
    /// Deposit would push the receiver's position above `per_user_cap`.
    UserCapExceeded = 19,
    /// Caps must be > 0 with per_user_cap <= tvl_cap; lower_caps may only decrease.
    InvalidCaps = 20,
    /// The pending upgrade's 48h timelock has not elapsed.
    UpgradeNotReady = 21,
    /// No upgrade has been proposed.
    NoPendingUpgrade = 22,
    /// The policy contract rejected the rebalance proposal (audit W1).
    RebalanceRejected = 23,
    /// No executor is configured for the proposal's trade route.
    RouteUnavailable = 24,
    /// A swap returned less than the proposal's min_amount_out, or spent more
    /// than amount_in, as measured on the vault's own balances.
    SlippageExceeded = 25,
    /// The pending router change's 48h delay has not elapsed / none pending.
    RouterNotReady = 26,
    /// After the trades, the vault's actual holdings (measured balances at
    /// per-asset median prices) breach the signed policy's bands, caps or
    /// liquidity floor (audit W1b). The whole rebalance reverts.
    AllocationOutOfPolicy = 27,
    /// A sleeve's target weight differs from the signed policy's target, or
    /// the policy does not declare the sleeve at all (audit W1b).
    SleeveTargetMismatch = 28,
}

/// Timelock between proposing and executing a WASM upgrade (CONTRACTS-SPEC:
/// "admin multisig + 48h timelock"; audit W3).
pub const UPGRADE_TIMELOCK_SECONDS: u64 = 172_800;

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PendingUpgrade {
    pub wasm_hash: BytesN<32>,
    pub proposed_at: u64,
    pub executable_at: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
pub enum SleeveType {
    CashStable = 0,
    UsdGovFunds = 1,
    EurSovereign = 2,
    YieldDollar = 3,
    Lending = 4,
    Equity = 5,
    Xlm = 6,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct SleeveState {
    pub asset_id: String,
    pub contract_address: Address,
    pub target_weight_bps: u32,
    pub band_bps: u32,
    pub current_balance: i128,
    pub sleeve_type: SleeveType,
    pub liquidity_tier: u32,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VaultConfig {
    pub admin: Address,
    pub pause_authority: Address,
    pub numeraire: Address,
    pub policy_contract: Address,
    pub oracle_contract: Address,
    pub registry_contract: Address,
    pub treasury: Address,
    pub management_fee_bps: u32,
    /// Maximum total vault value in numéraire units (AGENT-RULES Rule 6;
    /// ROADMAP 3.2 mainnet beta: $1M). Can only be lowered in-place.
    pub tvl_cap: i128,
    /// Maximum value any single receiver may hold via deposit/mint.
    pub per_user_cap: i128,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RedemptionCapacity {
    pub instant: i128,
    pub in_24h: i128,
    pub in_1w: i128,
    pub longer: i128,
    pub computed_at: u64,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RiskSheet {
    pub recommended_ltv_bps: u32,
    pub recommended_haircut_bps: u32,
    pub weighted_avg_redemption_hours: u32,
    pub computed_at: u64,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ListingKit {
    pub share_token: Address,
    pub numeraire: Address,
    pub oracle_contract: Address,
    pub recommended_ltv_bps: u32,
    pub recommended_haircut_bps: u32,
    pub liquidation_threshold_bps: u32,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
    pub source: Symbol,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct OracleSource {
    pub source_id: Symbol,
    pub asset: Address,
    pub sleeve: Symbol,
    pub price: i128,
    pub timestamp: u64,
    pub staleness_limit_seconds: u64,
    pub min_liquidity_depth_usd: i128,
    pub current_liquidity_depth_usd: i128,
    pub is_active: bool,
    /// Mirror of the oracle's OracleSource (audit O7a): the source's feeder.
    pub feeder: Address,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum DataKey {
    Config,
    Name,
    Symbol,
    Decimals,
    Sleeves,
    IsPaused,
    LastFeeAccrual,
    PendingUpgrade,
    /// Address allowed to submit rebalance proposals (audit W1).
    Keeper,
    /// Executor contract per trade route (u32 = TradeRoute discriminant).
    Router(u32),
    PendingRouter(u32),
}

// Rebalance proposal types - mirrors of constella-policy (keep in lockstep;
// contracttype decoding is lenient, so a drift would not fail loudly).
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum TradeRoute {
    Issuer = 0,
    Soroswap = 1,
    Aquarius = 2,
    Sdex = 3,
    /// Blend v2 supply / withdraw (decision D-H), lockstep with the policy.
    Blend = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeOperation {
    pub asset_in: Address,
    pub asset_out: Address,
    pub amount_in: i128,
    pub min_amount_out: i128,
    pub route: TradeRoute,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebalanceProposal {
    pub vault: Address,
    pub policy_hash: BytesN<32>,
    pub operations: Vec<TradeOperation>,
    pub weights_after: Vec<(Symbol, u32)>,
    pub deadline: u64,
    pub proposer: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub code: u32,
    pub message: Symbol,
}

/// A router change waiting out the 48h delay.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRouter {
    pub router: Address,
    pub effective_at: u64,
}

/// Soroswap router (github.com/soroswap/core): the subset the vault calls.
/// `swap_exact_tokens_for_tokens` requires `to.require_auth()` and moves
/// `amount_in` FROM `to` into the pair with a token transfer it initiates.
#[contractclient(name = "SoroswapRouterClient")]
pub trait SoroswapRouterInterface {
    fn swap_exact_tokens_for_tokens(env: Env, amount_in: i128, amount_out_min: i128, path: Vec<Address>, to: Address, deadline: u64) -> Vec<i128>;
    fn router_pair_for(env: Env, token_a: Address, token_b: Address) -> Address;
}

// Cross-Contract Client Traits for Decoupled WASM Compilation
#[contractclient(name = "NavOracleClient")]
pub trait NavOracleInterface {
    fn get_collateral_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>;
    fn get_deposit_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>;
    /// Median of the asset's valid sources (audit O6/W1: per-asset pricing).
    fn asset_price(env: Env, asset: Address) -> Option<PriceData>;
    /// Minimum of the asset's valid sources (Rule 3 collateral price).
    fn collateral_price(env: Env, asset: Address) -> Option<PriceData>;
    fn get_source(env: Env, source_id: Symbol) -> Option<OracleSource>;
    fn get_sleeve_sources(env: Env, sleeve_id: Symbol) -> Vec<Symbol>;
    /// Decision D-H: the Blend position priced under `pool`, if any. The
    /// oracle only registers pools of the official Blend factory.
    fn blend_position(env: Env, pool: Address) -> Option<BlendPositionRef>;
}

/// Mirror of `constella_nav_oracle::BlendPosition`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendPositionRef {
    pub pool: Address,
    pub underlying: Address,
}

/// Mirrors of Blend v2 types (blend-contracts-v2 `pool/src/storage.rs`,
/// `pool/src/pool/{reserve,user,actions}.rs`); decoded by field name.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendReserveConfigRef {
    pub index: u32,
    pub decimals: u32,
    pub c_factor: u32,
    pub l_factor: u32,
    pub util: u32,
    pub max_util: u32,
    pub r_base: u32,
    pub r_one: u32,
    pub r_two: u32,
    pub r_three: u32,
    pub reactivity: u32,
    pub supply_cap: i128,
    pub enabled: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendReserveDataRef {
    pub d_rate: i128,
    pub b_rate: i128,
    pub ir_mod: i128,
    pub b_supply: i128,
    pub d_supply: i128,
    pub backstop_credit: i128,
    pub last_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendReserveRef {
    pub asset: Address,
    pub config: BlendReserveConfigRef,
    pub data: BlendReserveDataRef,
    pub scalar: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendPositionsRef {
    pub liabilities: Map<u32, i128>,
    pub collateral: Map<u32, i128>,
    pub supply: Map<u32, i128>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendRequestRef {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

/// Blend v2 request types used by the vault: plain (non-collateral) supply
/// and withdraw only - the vault never borrows, so it has no health factor
/// and cannot be liquidated.
const BLEND_SUPPLY: u32 = 0;
const BLEND_WITHDRAW: u32 = 1;
const BLEND_RATE_SCALE: i128 = 1_000_000_000_000;

#[contractclient(name = "BlendPoolClient")]
pub trait BlendPoolInterface {
    fn get_reserve(env: Env, asset: Address) -> BlendReserveRef;
    fn get_positions(env: Env, address: Address) -> BlendPositionsRef;
    fn submit(env: Env, from: Address, spender: Address, to: Address, requests: Vec<BlendRequestRef>) -> BlendPositionsRef;
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryAssetEntryRef {
    pub contract_address: Address,
    pub asset_id: Symbol,
    pub issuer: Symbol,
    pub sleeve_type: u32,
    pub liquidity_tier: u32,
    pub permissionless: bool,
    pub oracle_sources: Vec<Symbol>,
    pub eligible_jurisdictions: Vec<Symbol>,
    pub blocked_jurisdictions: Vec<Symbol>,
    pub investor_classes_required: Vec<Symbol>,
    pub is_active: bool,
    pub added_at: u64,
}

#[contractclient(name = "AssetRegistryClient")]
pub trait AssetRegistryInterface {
    fn is_registered(env: Env, contract_address: Address) -> bool;
    fn get_asset_by_address(env: Env, contract_address: Address) -> Option<RegistryAssetEntryRef>;
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PolicyConfigRef {
    pub vault: Address,
    pub policy_hash: BytesN<32>,
    pub version: u32,
    pub registry_address: Address,
    pub min_liquidity_sleeve_bps: u32,
    pub max_single_asset_bps: u32,
    pub max_issuer_bps: u32,
    pub max_slippage_bps: u32,
    pub rebalance_band_bps: u32,
    pub is_active: bool,
    pub owner: Address,
    pub terms_hash: BytesN<32>,
}

/// Mirror of `constella_policy::SleeveTarget` (decoupled to avoid WASM symbol clashes).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PolicySleeveTargetRef {
    pub asset_id: Symbol,
    pub target_weight_bps: u32,
    pub min_band_bps: u32,
    pub max_band_bps: u32,
    pub sleeve_type: Symbol,
}

#[contractclient(name = "PolicyClient")]
pub trait PolicyInterface {
    fn get_policy_config(env: Env, vault: Address) -> Option<PolicyConfigRef>;
    fn validate_rebalance(env: Env, vault: Address, proposal: RebalanceProposal) -> ValidationResult;
    fn validate_holdings(env: Env, vault: Address, holdings: Vec<(Symbol, i128)>) -> ValidationResult;
    fn get_sleeve_targets(env: Env, vault: Address) -> Vec<PolicySleeveTargetRef>;
    fn register_policy(
        env: Env,
        vault: Address,
        owner: Address,
        policy_hash: BytesN<32>,
        registry_address: Address,
        min_liquidity_sleeve_bps: u32,
        max_single_asset_bps: u32,
        max_issuer_bps: u32,
        max_slippage_bps: u32,
        rebalance_band_bps: u32,
        sleeves: Vec<PolicySleeveTargetRef>,
    );
}

const SECONDS_PER_YEAR: u64 = 31_536_000;

fn get_config(env: &Env) -> VaultConfig {
    match env.storage().instance().get(&DataKey::Config) {
        Some(c) => {
            // Audit W2: keep the vault's instance entry live while it is used.
            env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_EXTEND_AMOUNT);
            c
        }
        None => panic_with_error!(env, VaultError::NotInitialized),
    }
}

fn is_vault_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::IsPaused)
        .unwrap_or(false)
}

fn compute_sleeve_symbol(env: &Env, sleeve_type: SleeveType) -> Symbol {
    match sleeve_type {
        SleeveType::CashStable => Symbol::new(env, "CASH_STABLE"),
        SleeveType::UsdGovFunds => Symbol::new(env, "USD_GOV_FUNDS"),
        SleeveType::EurSovereign => Symbol::new(env, "EUR_SOVEREIGN"),
        SleeveType::YieldDollar => Symbol::new(env, "YIELD_DOLLAR"),
        SleeveType::Lending => Symbol::new(env, "LENDING"),
        SleeveType::Equity => Symbol::new(env, "EQUITY"),
        SleeveType::Xlm => Symbol::new(env, "XLM"),
    }
}

/// Management-fee shares owed right now and the accrual timestamp to store
/// once they are minted. Pure: views use it to price pending fees exactly as
/// the next state-changing call will mint them (SEP-56 previews).
fn pending_fee_shares(env: &Env, config: &VaultConfig) -> (i128, u64) {
    let last_accrual: u64 = env
        .storage()
        .instance()
        .get(&DataKey::LastFeeAccrual)
        .unwrap_or(0);
    let now = env.ledger().timestamp();
    if now <= last_accrual || config.management_fee_bps == 0 {
        return (0, last_accrual);
    }

    let elapsed = now - last_accrual;
    let total_shares: i128 = share_supply(env);
    if total_shares == 0 {
        return (0, now);
    }

    let numerator = total_shares
        .checked_mul(config.management_fee_bps as i128)
        .and_then(|v| v.checked_mul(elapsed as i128));
    let denominator = (10_000i128).checked_mul(SECONDS_PER_YEAR as i128);

    let d_shares = match (numerator, denominator) {
        (Some(num), Some(den)) if den > 0 => num / den,
        _ => 0,
    };
    if d_shares <= 0 {
        return (0, last_accrual);
    }

    // Carry the sub-share remainder of elapsed time into the next accrual.
    let new_last = match (denominator, total_shares.checked_mul(config.management_fee_bps as i128)) {
        (Some(den), Some(fee_rate)) if fee_rate > 0 => last_accrual + ((d_shares * den) / fee_rate) as u64,
        _ => now,
    };
    (d_shares, new_last)
}

fn accrue_fees_internal(env: &Env, config: &VaultConfig) -> i128 {
    let last_accrual: u64 = env
        .storage()
        .instance()
        .get(&DataKey::LastFeeAccrual)
        .unwrap_or(0);
    let (d_shares, new_last) = pending_fee_shares(env, config);
    if new_last != last_accrual {
        env.storage().instance().set(&DataKey::LastFeeAccrual, &new_last);
    }
    if d_shares > 0 {
        // Audit W6: fee shares are minted like any other shares, with a SEP-41
        // mint event, so indexers can reconstruct every balance from events.
        mint_shares_internal(env, &config.treasury, d_shares);
        FeeAccrual { treasury: config.treasury.clone(), shares: d_shares }.publish(env);
    }
    d_shares
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavKind {
    /// Median of valid sources — used to mint shares.
    Deposit,
    /// Minimum of valid sources — used for redemptions and lender views.
    Collateral,
}

/// Collateral haircut per sleeve type, from COLLATERAL-PASSPORT.md Pillar 3:
/// cash 10%, T-bill funds (USD/EUR) 15%, yield-dollar 25%, lending 30% (upper
/// bound of the 25-30% band). The spec gives no figure for equity or XLM, so
/// they carry no borrowing power (100%) rather than an invented number.
fn sleeve_haircut_bps(sleeve_type: SleeveType) -> i128 {
    match sleeve_type {
        SleeveType::CashStable => 1000,
        SleeveType::UsdGovFunds | SleeveType::EurSovereign => 1500,
        SleeveType::YieldDollar => 2500,
        SleeveType::Lending => 3000,
        SleeveType::Equity | SleeveType::Xlm => 10000,
    }
}

/// The registry's asset ID for `asset` as a display label (audit M4). The
/// asset must be an active registry entry (closed universe), else
/// `AssetNotRegistered`.
fn registry_label(env: &Env, registry: &AssetRegistryClient, asset: &Address) -> (String, u32) {
    let entry = match registry.get_asset_by_address(asset) {
        Some(e) if e.is_active => e,
        _ => panic_with_error!(env, VaultError::AssetNotRegistered),
    };
    let sym: SymbolStr = SymbolStr::from_val(env, &entry.asset_id.to_symbol_val());
    let bytes: &[u8] = sym.as_ref();
    let label = match core::str::from_utf8(bytes) {
        Ok(text) => String::from_str(env, text),
        Err(_) => panic_with_error!(env, VaultError::InvalidSleeveConfig),
    };
    (label, entry.sleeve_type)
}

/// Registry sleeve-type code for cash (constella_registry::SleeveType::Cash).
const REGISTRY_SLEEVE_CASH: u32 = 0;

fn share_scale(env: &Env) -> i128 {
    let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
    10i128.pow(decimals)
}

/// Value of one sleeve in numéraire, and the oracle timestamp used.
///
/// Audit V4: this is the ONLY place a sleeve is valued. There is no par
/// fallback: cash counts at face value, an empty sleeve counts as exactly 0
/// (audit V3), and a funded non-cash sleeve must have a valid oracle price or
/// the caller gets `OracleUnavailable` (NAV-ORACLE Rule 5 hard stop).
fn sleeve_value(
    env: &Env,
    oracle: &NavOracleClient,
    sleeve: &SleeveState,
    kind: NavKind,
    scale: i128,
) -> Result<(i128, u64), VaultError> {
    let held = sleeve_holdings(env, sleeve);
    if sleeve.sleeve_type == SleeveType::CashStable {
        return Ok((held, 0));
    }
    if held == 0 {
        return Ok((0, 0));
    }
    // Audit O6/W1: price the sleeve's own asset. Sleeve-type keys could make
    // two assets of one type share a price; a multi-asset oracle sleeve has no
    // sleeve-level price at all.
    let pd = match kind {
        NavKind::Deposit => oracle.asset_price(&sleeve.contract_address),
        NavKind::Collateral => oracle.collateral_price(&sleeve.contract_address),
    }
    .ok_or(VaultError::OracleUnavailable)?;
    if pd.price <= 0 {
        return Err(VaultError::OracleUnavailable);
    }
    let value = held.checked_mul(pd.price).ok_or(VaultError::MathOverflow)? / scale;
    Ok((value, pd.timestamp))
}

/// Audit W1b: the signed policy's CURRENT target for each sleeve, in sleeve
/// order (0 for a sleeve the policy does not declare, e.g. after an
/// amendment dropped it), or None while no policy is bound. Read live, so an
/// activated amendment takes effect without anyone re-running set_sleeves.
fn signed_targets(env: &Env, config: &VaultConfig, sleeves: &Vec<SleeveState>) -> Option<Vec<u32>> {
    let policy = PolicyClient::new(env, &config.policy_contract);
    let me = env.current_contract_address();
    policy.get_policy_config(&me)?;
    let targets = policy.get_sleeve_targets(&me);
    let registry = AssetRegistryClient::new(env, &config.registry_contract);
    let mut out = Vec::new(env);
    for sleeve in sleeves.iter() {
        let weight = match registry.get_asset_by_address(&sleeve.contract_address) {
            Some(entry) => targets.iter().find(|t| t.asset_id == entry.asset_id).map(|t| t.target_weight_bps).unwrap_or(0),
            None => 0,
        };
        out.push_back(weight);
    }
    Some(out)
}

/// Audit W1b: value every sleeve as actually held (measured balance, capped
/// by the booked balance, at its asset's median price; cash at face value)
/// under its registry asset ID, and require the signed policy to accept the
/// result. A missing price stops with `OracleUnavailable` (Rule 5); a policy
/// rejection stops with `AllocationOutOfPolicy`. Panics revert the trades.
fn check_actual_allocation(env: &Env, config: &VaultConfig, sleeves: &Vec<SleeveState>) {
    let oracle = NavOracleClient::new(env, &config.oracle_contract);
    let registry = AssetRegistryClient::new(env, &config.registry_contract);
    let scale = share_scale(env);
    let mut holdings: Vec<(Symbol, i128)> = Vec::new(env);
    for sleeve in sleeves.iter() {
        let id = match registry.get_asset_by_address(&sleeve.contract_address) {
            Some(e) if e.is_active => e.asset_id,
            _ => panic_with_error!(env, VaultError::AssetNotRegistered),
        };
        let value = match sleeve_value(env, &oracle, &sleeve, NavKind::Deposit, scale) {
            Ok((v, _)) => v,
            Err(e) => panic_with_error!(env, e),
        };
        holdings.push_back((id, value));
    }
    let verdict = PolicyClient::new(env, &config.policy_contract)
        .validate_holdings(&env.current_contract_address(), &holdings);
    if !verdict.is_valid {
        panic_with_error!(env, VaultError::AllocationOutOfPolicy);
    }
}

/// Total value of all sleeves and the latest oracle timestamp involved.
fn holdings_value(env: &Env, config: &VaultConfig, kind: NavKind) -> Result<(i128, u64), VaultError> {
    let sleeves: Vec<SleeveState> = env
        .storage()
        .instance()
        .get(&DataKey::Sleeves)
        .unwrap_or_else(|| Vec::new(env));
    let oracle = NavOracleClient::new(env, &config.oracle_contract);
    let scale = share_scale(env);
    let mut total = 0i128;
    let mut latest_ts = env.ledger().timestamp();
    for sleeve in sleeves.iter() {
        let (value, ts) = sleeve_value(env, &oracle, &sleeve, kind, scale)?;
        total = total.checked_add(value).ok_or(VaultError::MathOverflow)?;
        if ts > latest_ts {
            latest_ts = ts;
        }
    }
    Ok((total, latest_ts))
}

/// NAV per share for `total_shares` outstanding shares.
fn compute_nav(env: &Env, config: &VaultConfig, kind: NavKind, total_shares: i128) -> Option<PriceData> {
    let scale = share_scale(env);
    let has_sleeves = env
        .storage()
        .instance()
        .get::<_, Vec<SleeveState>>(&DataKey::Sleeves)
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    // Audit V5: the share price is ALWAYS derived from what the vault holds.
    // There is no oracle override (the former "VAULT" pseudo-sleeve let the
    // oracle admin set NAV per share directly).
    let (total_value, latest_ts) = holdings_value(env, config, kind).ok()?;
    let source = match kind {
        NavKind::Deposit => Symbol::new(env, "MEDIAN_NAV"),
        NavKind::Collateral => Symbol::new(env, "CONSERVATIVE_NAV"),
    };
    let price = if !has_sleeves || total_shares == 0 {
        scale
    } else {
        total_value.checked_mul(scale)? / total_shares
    };
    Some(PriceData {
        price,
        timestamp: latest_ts,
        source,
    })
}

/// Shares outstanding once pending management fees are minted.
fn supply_with_pending_fees(env: &Env, config: &VaultConfig) -> Option<i128> {
    share_supply(env).checked_add(pending_fee_shares(env, config).0)
}

/// View NAV (median): includes pending fee shares, so it equals the NAV the
/// next deposit/mint will execute at.
fn compute_deposit_nav(env: &Env, config: &VaultConfig) -> Option<PriceData> {
    compute_nav(env, config, NavKind::Deposit, supply_with_pending_fees(env, config)?)
}

/// View NAV (conservative): includes pending fee shares.
fn compute_conservative_nav(env: &Env, config: &VaultConfig) -> Option<PriceData> {
    compute_nav(env, config, NavKind::Collateral, supply_with_pending_fees(env, config)?)
}

/// Execution NAV, read right after `accrue_fees_internal` (fees already minted).
fn execution_nav(env: &Env, config: &VaultConfig, kind: NavKind) -> i128 {
    match compute_nav(env, config, kind, share_supply(env)) {
        Some(pd) if pd.price > 0 => pd.price,
        _ => panic_with_error!(env, VaultError::OracleUnavailable),
    }
}

/// View NAV price, panicking when unpriced (as execution would).
fn view_nav(env: &Env, config: &VaultConfig, kind: NavKind) -> i128 {
    let pd = match kind {
        NavKind::Deposit => compute_deposit_nav(env, config),
        NavKind::Collateral => compute_conservative_nav(env, config),
    };
    match pd {
        Some(pd) if pd.price > 0 => pd.price,
        _ => panic_with_error!(env, VaultError::OracleUnavailable),
    }
}

fn mul_div(env: &Env, a: i128, b: i128, d: i128, round_up: bool) -> i128 {
    let p = match a.checked_mul(b) {
        Some(p) => p,
        None => panic_with_error!(env, VaultError::MathOverflow),
    };
    if round_up { (p + d - 1) / d } else { p / d }
}

// SEP-56 conversions. Rounding always favours the vault (ERC-4626): deposit
// and redeem round down, mint and withdraw round up.
fn shares_for_deposit(env: &Env, assets: i128, nav: i128) -> i128 {
    mul_div(env, assets, share_scale(env), nav, false)
}
fn assets_for_mint(env: &Env, shares: i128, nav: i128) -> i128 {
    mul_div(env, shares, nav, share_scale(env), true)
}
fn assets_for_redeem(env: &Env, shares: i128, nav: i128) -> i128 {
    mul_div(env, shares, nav, share_scale(env), false)
}
fn shares_for_withdraw(env: &Env, assets: i128, nav: i128) -> i128 {
    mul_div(env, assets, share_scale(env), nav, true)
}

/// Moves `from`'s numeraire in (OZ `deposit_internal`: direct transfer when
/// operator == from, otherwise `transfer_from` under `from`'s allowance to the
/// operator), mints `shares` to `receiver`, books the cash sleeve, emits events.
fn deposit_shares(env: &Env, receiver: &Address, from: &Address, operator: &Address, assets: i128, shares: i128) {
    OzVault::deposit_internal(env, receiver, assets, shares, from, operator);
    update_cash_sleeve_balance(env, assets, true);
    Mint { to: receiver.clone(), amount: shares }.publish(env);
    Deposit { operator: operator.clone(), from: from.clone(), receiver: receiver.clone(), assets, shares }.publish(env);
}

/// Instant-redemption guards: enough cash, and the policy liquidity floor holds.
fn check_redemption_liquidity(env: &Env, config: &VaultConfig, assets: i128) {
    let cash_balance = get_cash_sleeve_balance(env);
    if assets > cash_balance {
        panic_with_error!(env, VaultError::ExceedsRedemptionCapacity);
    }
    let total_a = StrategyVault::total_assets(env.clone());
    let floor_bps = get_policy_floor_bps(env, config);
    if total_a > assets && floor_bps > 0 {
        let remaining_assets = total_a - assets;
        let remaining_cash = cash_balance - assets;
        let lhs = remaining_cash.checked_mul(10_000);
        let rhs = remaining_assets.checked_mul(floor_bps as i128);
        match (lhs, rhs) {
            (Some(l), Some(r)) if l >= r => {}
            (Some(_), Some(_)) => panic_with_error!(env, VaultError::LiquidityFloorBreached),
            _ => panic_with_error!(env, VaultError::MathOverflow),
        }
    }
}

/// Burns `owner`'s shares and pays `receiver` (OZ `withdraw_internal`, which
/// spends the owner's share allowance when operator != owner), books the cash
/// sleeve, emits events. Shortfalls keep the vault's public error codes.
fn withdraw_shares(env: &Env, receiver: &Address, owner: &Address, operator: &Address, assets: i128, shares: i128) {
    if operator != owner && Base::allowance(env, owner, operator) < shares {
        panic_with_error!(env, VaultError::InsufficientAllowance);
    }
    if Base::balance(env, owner) < shares {
        panic_with_error!(env, VaultError::InsufficientBalance);
    }
    update_cash_sleeve_balance(env, assets, false);
    OzVault::withdraw_internal(env, receiver, owner, assets, shares, operator);
    Burn { from: owner.clone(), amount: shares }.publish(env);
    Withdraw { operator: operator.clone(), receiver: receiver.clone(), owner: owner.clone(), assets, shares }.publish(env);
}

/// Tokens of `asset` actually held by this vault right now.
fn real_balance(env: &Env, asset: &Address) -> i128 {
    token::Client::new(env, asset).balance(&env.current_contract_address())
}

/// The Blend position behind a sleeve, if it is one: only `Lending` sleeves
/// are looked up, and the oracle decides (single source of truth, D-H).
fn blend_of(env: &Env, sleeve: &SleeveState) -> Option<BlendPositionRef> {
    if sleeve.sleeve_type != SleeveType::Lending {
        return None;
    }
    NavOracleClient::new(env, &get_config(env).oracle_contract).blend_position(&sleeve.contract_address)
}

/// The vault's plain-supply bTokens in a Blend reserve (never collateral).
fn blend_b_tokens(env: &Env, pool: &BlendPoolClient, index: u32) -> i128 {
    pool.get_positions(&env.current_contract_address()).supply.get(index).unwrap_or(0)
}

/// What the vault really holds of a sleeve: bTokens for a Blend position,
/// otherwise its token balance.
fn real_holding(env: &Env, sleeve: &SleeveState) -> i128 {
    match blend_of(env, sleeve) {
        Some(pos) => {
            let pool = BlendPoolClient::new(env, &pos.pool);
            let index = pool.get_reserve(&pos.underlying).config.index;
            blend_b_tokens(env, &pool, index)
        }
        None => real_balance(env, &sleeve.contract_address),
    }
}

/// Decision D-H: one Blend leg. Supply when `asset_out` is a Blend position
/// whose underlying is `asset_in`; withdraw for the reverse. Returns
/// (spent, received) MEASURED on the vault's own balances - token balance and
/// bToken position - never taken from the pool's return value.
fn blend_leg(env: &Env, config: &VaultConfig, op: &TradeOperation) -> (i128, i128) {
    let oracle = NavOracleClient::new(env, &config.oracle_contract);
    let (pos, supplying) = match (oracle.blend_position(&op.asset_out), oracle.blend_position(&op.asset_in)) {
        (Some(p), None) if p.underlying == op.asset_in => (p, true),
        (None, Some(p)) if p.underlying == op.asset_out => (p, false),
        _ => panic_with_error!(env, VaultError::RouteUnavailable),
    };
    let me = env.current_contract_address();
    let pool = BlendPoolClient::new(env, &pos.pool);
    let reserve = pool.get_reserve(&pos.underlying);
    let index = reserve.config.index;
    let underlying = token::Client::new(env, &pos.underlying);
    let (b_before, u_before) = (blend_b_tokens(env, &pool, index), underlying.balance(&me));
    let request = if supplying {
        // The pool pulls `amount_in` from the vault: authorize exactly that.
        env.authorize_as_current_contract(vec![
            env,
            InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: pos.underlying.clone(),
                    fn_name: symbol_short!("transfer"),
                    args: (me.clone(), pos.pool.clone(), op.amount_in).into_val(env),
                },
                sub_invocations: vec![env],
            }),
        ]);
        BlendRequestRef { request_type: BLEND_SUPPLY, address: pos.underlying.clone(), amount: op.amount_in }
    } else {
        // Blend withdraws in underlying units and burns ceil(amount / b_rate)
        // bTokens; rounding the conversion down keeps the burn <= amount_in.
        let amount = match op.amount_in.checked_mul(reserve.data.b_rate) {
            Some(v) => v / BLEND_RATE_SCALE,
            None => panic_with_error!(env, VaultError::MathOverflow),
        };
        BlendRequestRef { request_type: BLEND_WITHDRAW, address: pos.underlying.clone(), amount }
    };
    pool.submit(&me, &me, &me, &vec![env, request]);
    let (b_after, u_after) = (blend_b_tokens(env, &pool, index), underlying.balance(&me));
    if supplying {
        (u_before - u_after, b_after - b_before)
    } else {
        (b_before - b_after, u_after - u_before)
    }
}

/// Holdings used for every valuation: min(recorded, actually held).
///
/// Audit V2: recorded balances change only through value-moving flows
/// (deposit / mint / redeem / withdraw, and future policy-validated
/// rebalances). Capping by the real token balance means an issuer clawback or
/// freeze-and-burn lowers NAV immediately, while unsolicited transfers into the
/// vault never raise it (no donation-based share-price manipulation).
fn sleeve_holdings(env: &Env, sleeve: &SleeveState) -> i128 {
    let held = real_holding(env, sleeve);
    if held < sleeve.current_balance {
        held
    } else {
        sleeve.current_balance
    }
}

fn get_policy_floor_bps(env: &Env, config: &VaultConfig) -> u32 {
    let policy = PolicyClient::new(env, &config.policy_contract);
    if let Some(cfg) = policy.get_policy_config(&env.current_contract_address()) {
        cfg.min_liquidity_sleeve_bps
    } else {
        1000 // 10% default
    }
}

fn calculate_instant_capacity(cash_balance: i128, total_assets: i128, floor_bps: u32) -> i128 {
    if cash_balance <= 0 || total_assets <= 0 {
        return 0;
    }
    if floor_bps == 0 {
        return cash_balance;
    }
    if floor_bps >= 10000 {
        return 0;
    }

    let num = match cash_balance
        .checked_mul(10000)
        .and_then(|v| v.checked_sub(total_assets.checked_mul(floor_bps as i128)?))
    {
        Some(n) => n,
        None => return 0,
    };

    if num <= 0 {
        return 0;
    }

    let den = 10000i128 - floor_bps as i128;
    let max_instant = num / den;
    if max_instant > cash_balance {
        cash_balance
    } else {
        max_instant
    }
}

fn get_cash_sleeve_balance(env: &Env) -> i128 {
    let sleeves: Vec<SleeveState> = env
        .storage()
        .instance()
        .get(&DataKey::Sleeves)
        .unwrap_or_else(|| Vec::new(env));
    for s in sleeves.iter() {
        if s.sleeve_type == SleeveType::CashStable {
            return sleeve_holdings(env, &s);
        }
    }
    0
}

fn update_cash_sleeve_balance(env: &Env, amount: i128, is_deposit: bool) {
    let mut sleeves: Vec<SleeveState> = env
        .storage()
        .instance()
        .get(&DataKey::Sleeves)
        .unwrap_or_else(|| Vec::new(env));
    for i in 0..sleeves.len() {
        let mut s = sleeves.get(i).unwrap();
        if s.sleeve_type == SleeveType::CashStable {
            if is_deposit {
                s.current_balance += amount;
            } else {
                s.current_balance -= amount;
            }
            sleeves.set(i, s);
            env.storage().instance().set(&DataKey::Sleeves, &sleeves);
            return;
        }
    }
}

fn mint_shares_internal(env: &Env, receiver: &Address, shares: i128) {
    increase_total_supply(env, shares);
    Base::update(env, None, Some(receiver), shares);

    Mint { to: receiver.clone(), amount: shares }.publish(env);
}

/// Moves `amount` shares from `from` to `to` (SEP-41 transfer semantics).
///
/// Audit V1: the previous inline implementation read `to`'s balance before
/// debiting `from`, so `from == to` credited a stale balance and minted
/// `amount` new shares. Here the sender is always debited first and a
/// self-transfer is a balance no-op (the caller still had to authorize it and
/// must own at least `amount`).
fn move_shares(env: &Env, from: &Address, to: &Address, amount: i128) {
    if Base::balance(env, from) < amount {
        panic_with_error!(env, VaultError::InsufficientBalance);
    }
    if from == to {
        return;
    }
    Base::update(env, Some(from), Some(to), amount);
}

fn burn_shares_internal(env: &Env, owner: &Address, shares: i128) {
    if Base::balance(env, owner) < shares {
        panic_with_error!(env, VaultError::InsufficientBalance);
    }
    Base::update(env, Some(owner), None, shares);
    decrease_total_supply(env, shares);

    Burn { from: owner.clone(), amount: shares }.publish(env);
}

/// Consumes `amount` of `spender`'s allowance over `owner`'s shares, keeping
/// the vault's public error code for a shortfall (expired counts as zero).
fn spend_share_allowance(env: &Env, owner: &Address, spender: &Address, amount: i128) {
    if Base::allowance(env, owner, spender) < amount {
        panic_with_error!(env, VaultError::InsufficientAllowance);
    }
    Base::spend_allowance(env, owner, spender, amount);
}

/// Remaining room under (tvl_cap, per_user_cap) for `receiver`, in numéraire
/// units, valued at the deposit NAV the depositor would pay (audit W3).
fn cap_room(env: &Env, config: &VaultConfig, receiver: &Address, deposit_nav: i128, scale: i128) -> (i128, i128) {
    let tvl = match holdings_value(env, config, NavKind::Deposit) {
        Ok((v, _)) => v,
        Err(e) => panic_with_error!(env, e),
    };
    let held_shares: i128 = Base::balance(env, receiver);
    let position = match held_shares.checked_mul(deposit_nav) {
        Some(v) => v / scale,
        None => panic_with_error!(env, VaultError::MathOverflow),
    };
    let tvl_room = (config.tvl_cap - tvl).max(0);
    let user_room = (config.per_user_cap - position).max(0);
    (tvl_room, user_room)
}

fn enforce_caps(env: &Env, config: &VaultConfig, receiver: &Address, assets: i128, deposit_nav: i128, scale: i128) {
    let (tvl_room, user_room) = cap_room(env, config, receiver, deposit_nav, scale);
    if assets > tvl_room {
        panic_with_error!(env, VaultError::TvlCapExceeded);
    }
    if assets > user_room {
        panic_with_error!(env, VaultError::UserCapExceeded);
    }
}

/// Delay before a router (which can move vault funds) takes effect: 48 h,
/// the same window as upgrades, policy amendments and oracle source changes.
pub const ROUTER_CHANGE_DELAY_SECONDS: u64 = 48 * 3600;

fn sleeve_index(sleeves: &Vec<SleeveState>, asset: &Address) -> u32 {
    for i in 0..sleeves.len() {
        if sleeves.get_unchecked(i).contract_address == *asset {
            return i;
        }
    }
    panic!()
}

/// Execute one Soroswap swap of exactly `amount_in`, measuring the result on
/// the vault's own balances. Returns (spent, received).
fn swap_on_soroswap(env: &Env, router: &Address, op: &TradeOperation, deadline: u64) -> (i128, i128) {
    let me = env.current_contract_address();
    let token_in = token::Client::new(env, &op.asset_in);
    let token_out = token::Client::new(env, &op.asset_out);
    let (in_before, out_before) = (token_in.balance(&me), token_out.balance(&me));
    let client = SoroswapRouterClient::new(env, router);
    let pair = client.router_pair_for(&op.asset_in, &op.asset_out);
    // The router moves `amount_in` from the vault into the pair; authorize
    // exactly that one transfer (contract-to-contract auth), nothing else.
    env.authorize_as_current_contract(vec![
        env,
        InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: op.asset_in.clone(),
                fn_name: symbol_short!("transfer"),
                args: (me.clone(), pair, op.amount_in).into_val(env),
            },
            sub_invocations: vec![env],
        }),
    ]);
    let path = vec![env, op.asset_in.clone(), op.asset_out.clone()];
    client.swap_exact_tokens_for_tokens(&op.amount_in, &op.min_amount_out, &path, &me, &deadline);
    let spent = in_before - token_in.balance(&me);
    let received = token_out.balance(&me) - out_before;
    (spent, received)
}

#[contract]
pub struct StrategyVault;

#[contractimpl]
impl StrategyVault {
    /// Runs atomically as part of deployment, so no one can claim an
    /// uninitialized vault (audit M3). There is no `initialize` entrypoint;
    /// invalid settings make the deployment itself fail.
    pub fn __constructor(
        env: Env,
        config: VaultConfig,
        name: String,
        symbol: String,
        decimals: u32,
    ) {
        if config.management_fee_bps > 10000 || decimals > 18 {
            panic_with_error!(&env, VaultError::InvalidBps);
        }
        if config.tvl_cap <= 0 || config.per_user_cap <= 0 || config.per_user_cap > config.tvl_cap {
            panic_with_error!(&env, VaultError::InvalidCaps);
        }

        let admin = config.admin.clone();
        let numeraire = config.numeraire.clone();

        // Audit M4: the numéraire must be an active, registered CASH asset
        // (closed universe; VAULT-DESIGN: USDC or EURC), and the cash sleeve is
        // labelled with its registry ID instead of a hard-coded "USDC".
        let registry = AssetRegistryClient::new(&env, &config.registry_contract);
        let (cash_label, cash_type) = registry_label(&env, &registry, &numeraire);
        if cash_type != REGISTRY_SLEEVE_CASH {
            panic_with_error!(&env, VaultError::InvalidSleeveConfig);
        }

        env.storage().instance().set(&DataKey::Config, &config);
        OzVault::set_asset(&env, config.numeraire.clone());
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::IsPaused, &false);
        env.storage()
            .instance()
            .set(&DataKey::LastFeeAccrual, &env.ledger().timestamp());

        let mut sleeves = Vec::new(&env);
        sleeves.push_back(SleeveState {
            asset_id: cash_label,
            contract_address: numeraire,
            target_weight_bps: 10000,
            band_bps: 0,
            current_balance: 0,
            sleeve_type: SleeveType::CashStable,
            liquidity_tier: 0,
        });
        env.storage().instance().set(&DataKey::Sleeves, &sleeves);

        VaultInitialized { admin, timestamp: env.ledger().timestamp() }.publish(&env);
    }

    // SEP-56 Vault Operations
    /// SEP-56 deposit: `operator` authorizes; `from` pays (directly when it is
    /// the operator, otherwise via its numeraire allowance to the operator);
    /// `receiver` gets the shares, priced at the median (deposit) NAV.
    pub fn deposit(env: Env, assets: i128, receiver: Address, from: Address, operator: Address) -> i128 {
        operator.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if assets <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let nav = execution_nav(&env, &config, NavKind::Deposit);
        let shares = shares_for_deposit(&env, assets, nav);
        if shares <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        enforce_caps(&env, &config, &receiver, assets, nav, share_scale(&env));
        deposit_shares(&env, &receiver, &from, &operator, assets, shares);
        shares
    }

    /// SEP-56 mint: exactly `shares` to `receiver`; returns the assets paid.
    pub fn mint(env: Env, shares: i128, receiver: Address, from: Address, operator: Address) -> i128 {
        operator.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if shares <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let nav = execution_nav(&env, &config, NavKind::Deposit);
        let assets = assets_for_mint(&env, shares, nav);
        if assets <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        enforce_caps(&env, &config, &receiver, assets, nav, share_scale(&env));
        deposit_shares(&env, &receiver, &from, &operator, assets, shares);
        assets
    }

    /// SEP-56 redeem: burns exactly `shares` of `owner` (operator == owner, or
    /// within the owner's share allowance), pays `receiver` at the
    /// conservative NAV, subject to cash and the policy liquidity floor.
    pub fn redeem(env: Env, shares: i128, receiver: Address, owner: Address, operator: Address) -> i128 {
        operator.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if shares <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let nav = execution_nav(&env, &config, NavKind::Collateral);
        let assets = assets_for_redeem(&env, shares, nav);
        if assets <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        check_redemption_liquidity(&env, &config, assets);
        withdraw_shares(&env, &receiver, &owner, &operator, assets, shares);
        assets
    }

    /// SEP-56 withdraw: pays exactly `assets`; returns the shares burned.
    pub fn withdraw(env: Env, assets: i128, receiver: Address, owner: Address, operator: Address) -> i128 {
        operator.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if assets <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let nav = execution_nav(&env, &config, NavKind::Collateral);
        let shares = shares_for_withdraw(&env, assets, nav);
        check_redemption_liquidity(&env, &config, assets);
        withdraw_shares(&env, &receiver, &owner, &operator, assets, shares);
        shares
    }

    /// SEP-56: shares `deposit(assets)` would mint now (fees included).
    pub fn preview_deposit(env: Env, assets: i128) -> i128 {
        if assets <= 0 {
            return 0;
        }
        let config = get_config(&env);
        shares_for_deposit(&env, assets, view_nav(&env, &config, NavKind::Deposit))
    }

    /// SEP-56: assets `mint(shares)` would cost now.
    pub fn preview_mint(env: Env, shares: i128) -> i128 {
        if shares <= 0 {
            return 0;
        }
        let config = get_config(&env);
        assets_for_mint(&env, shares, view_nav(&env, &config, NavKind::Deposit))
    }

    /// SEP-56: shares `withdraw(assets)` would burn now.
    pub fn preview_withdraw(env: Env, assets: i128) -> i128 {
        if assets <= 0 {
            return 0;
        }
        let config = get_config(&env);
        shares_for_withdraw(&env, assets, view_nav(&env, &config, NavKind::Collateral))
    }

    /// SEP-56: assets `redeem(shares)` would pay now.
    pub fn preview_redeem(env: Env, shares: i128) -> i128 {
        if shares <= 0 {
            return 0;
        }
        let config = get_config(&env);
        assets_for_redeem(&env, shares, view_nav(&env, &config, NavKind::Collateral))
    }

    pub fn total_assets(env: Env) -> i128 {
        let config = get_config(&env);
        match holdings_value(&env, &config, NavKind::Collateral) {
            Ok((total, _)) => total,
            Err(e) => panic_with_error!(&env, e),
        }
    }

    pub fn total_supply(env: Env) -> i128 {
        share_supply(&env)
    }

    pub fn convert_to_shares(env: Env, assets: i128) -> i128 {
        if assets <= 0 {
            return 0;
        }
        let config = get_config(&env);
        let deposit_nav = match compute_deposit_nav(&env, &config) {
            Some(pd) => pd.price,
            None => panic_with_error!(&env, VaultError::OracleUnavailable),
        };
        if deposit_nav <= 0 {
            panic_with_error!(&env, VaultError::OracleUnavailable);
        }
        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);
        (assets.checked_mul(scale).unwrap()) / deposit_nav
    }

    pub fn convert_to_assets(env: Env, shares: i128) -> i128 {
        if shares <= 0 {
            return 0;
        }
        let config = get_config(&env);
        let conservative_nav = match compute_conservative_nav(&env, &config) {
            Some(pd) => pd.price,
            None => panic_with_error!(&env, VaultError::OracleUnavailable),
        };
        if conservative_nav <= 0 {
            panic_with_error!(&env, VaultError::OracleUnavailable);
        }
        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);
        (shares.checked_mul(conservative_nav).unwrap()) / scale
    }

    /// SEP-56 `query_asset`: the numéraire.
    pub fn query_asset(env: Env) -> Address {
        get_config(&env).numeraire
    }

    /// SEP-56: the most `receiver` can deposit right now (audit W3: real caps).
    pub fn max_deposit(env: Env, receiver: Address) -> i128 {
        if is_vault_paused(&env) {
            return 0;
        }
        let config = get_config(&env);
        let nav = match compute_deposit_nav(&env, &config) {
            Some(pd) if pd.price > 0 => pd.price,
            _ => return 0,
        };
        let (tvl_room, user_room) = cap_room(&env, &config, &receiver, nav, share_scale(&env));
        tvl_room.min(user_room)
    }

    /// SEP-56: the most shares `receiver` can mint right now (mint rounds the
    /// asset cost up, so floor(room / nav) shares always fit).
    pub fn max_mint(env: Env, receiver: Address) -> i128 {
        if is_vault_paused(&env) {
            return 0;
        }
        let config = get_config(&env);
        let nav = match compute_deposit_nav(&env, &config) {
            Some(pd) if pd.price > 0 => pd.price,
            _ => return 0,
        };
        let scale = share_scale(&env);
        let (tvl_room, user_room) = cap_room(&env, &config, &receiver, nav, scale);
        match tvl_room.min(user_room).checked_mul(scale) {
            Some(v) => v / nav,
            None => panic_with_error!(&env, VaultError::MathOverflow),
        }
    }

    /// Lower the deposit caps immediately (risk-reducing). Raising caps is a
    /// governance action (ROADMAP 4.6) only possible through a timelocked upgrade.
    pub fn lower_caps(env: Env, tvl_cap: i128, per_user_cap: i128) {
        let mut config = get_config(&env);
        config.admin.require_auth();
        if tvl_cap <= 0
            || per_user_cap <= 0
            || per_user_cap > tvl_cap
            || tvl_cap > config.tvl_cap
            || per_user_cap > config.per_user_cap
        {
            panic_with_error!(&env, VaultError::InvalidCaps);
        }
        config.tvl_cap = tvl_cap;
        config.per_user_cap = per_user_cap;
        env.storage().instance().set(&DataKey::Config, &config);
        CapsLowered { admin: config.admin, tvl_cap, per_user_cap }.publish(&env);
    }

    pub fn max_withdraw(env: Env, owner: Address) -> i128 {
        if is_vault_paused(&env) {
            return 0;
        }
        let owner_bal: i128 = Base::balance(&env, &owner);
        let owner_assets = Self::convert_to_assets(env.clone(), owner_bal);
        let cap = Self::get_redemption_capacity(env);
        if owner_assets < cap.instant {
            owner_assets
        } else {
            cap.instant
        }
    }

    pub fn max_redeem(env: Env, owner: Address) -> i128 {
        if is_vault_paused(&env) {
            return 0;
        }
        let owner_bal: i128 = Base::balance(&env, &owner);
        let cap = Self::get_redemption_capacity(env.clone());
        let config = get_config(&env);
        let conservative_nav = match compute_conservative_nav(&env, &config) {
            Some(pd) => pd.price,
            None => return 0,
        };
        if conservative_nav <= 0 {
            return 0;
        }
        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);
        let cap_shares = match cap.instant.checked_mul(scale) {
            Some(v) => v / conservative_nav,
            None => 0,
        };
        if owner_bal < cap_shares {
            owner_bal
        } else {
            cap_shares
        }
    }

    // SEP-41 Fungible Token Standard
    pub fn balance(env: Env, id: Address) -> i128 {
        Base::balance(&env, &id)
    }

    /// SEP-41 transfer. `to` may be a muxed account (exchange / custodian
    /// sub-account): shares are credited to the underlying address and the
    /// muxed id is reported in the event only.
    pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        from.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if amount <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let to_addr = to.address();
        move_shares(&env, &from, &to_addr, amount);

        match to.id() {
            Some(id) => MuxedTransfer { from, to: to_addr, to_muxed_id: Some(id), amount }.publish(&env),
            None => Transfer { from, to: to_addr, amount }.publish(&env),
        }
    }

    /// SEP-41 approve. `expiration_ledger` (SEP-41 `live_until_ledger`) is
    /// validated by the OpenZeppelin allowance storage (audit W6): a non-zero
    /// allowance cannot already be expired, and none may outlive the network's
    /// maximum entry TTL (error #102 `InvalidLiveUntilLedger`).
    pub fn approve(env: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32) {
        from.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if amount < 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        get_config(&env);
        Base::set_allowance(&env, &from, &spender, amount, expiration_ledger);

        Approve { from, spender, amount, live_until_ledger: expiration_ledger }.publish(&env);
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        Base::allowance(&env, &from, &spender)
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if amount <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        spend_share_allowance(&env, &from, &spender, amount);
        move_shares(&env, &from, &to, amount);

        Transfer { from, to, amount }.publish(&env);
    }

    /// SEP-41 burn: destroys `amount` of the caller's own shares. No assets
    /// leave the vault, so the remaining holders' NAV per share rises.
    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if amount <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        burn_shares_internal(&env, &from, amount);
    }

    /// SEP-41 burn_from: `spender` burns `from`'s shares within its allowance.
    pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if amount <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        spend_share_allowance(&env, &from, &spender, amount);
        burn_shares_internal(&env, &from, amount);
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::Decimals)
            .unwrap_or(7)
    }

    pub fn name(env: Env) -> String {
        match env.storage().instance().get(&DataKey::Name) {
            Some(n) => n,
            None => panic_with_error!(&env, VaultError::NotInitialized),
        }
    }

    pub fn symbol(env: Env) -> String {
        match env.storage().instance().get(&DataKey::Symbol) {
            Some(s) => s,
            None => panic_with_error!(&env, VaultError::NotInitialized),
        }
    }

    // Collateral Passport (View Functions)
    pub fn get_nav_conservative(env: Env) -> Option<PriceData> {
        let config = get_config(&env);
        compute_conservative_nav(&env, &config)
    }

    pub fn get_nav_deposit(env: Env) -> Option<PriceData> {
        let config = get_config(&env);
        compute_deposit_nav(&env, &config)
    }

    pub fn get_redemption_capacity(env: Env) -> RedemptionCapacity {
        let config = get_config(&env);
        let sleeves: Vec<SleeveState> = env
            .storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env));
        let scale = share_scale(&env);
        let oracle = NavOracleClient::new(&env, &config.oracle_contract);
        let add = |acc: i128, v: i128| -> i128 {
            match acc.checked_add(v) {
                Some(x) => x,
                None => panic_with_error!(&env, VaultError::MathOverflow),
            }
        };

        let mut cash_balance = 0i128;
        let mut in_24h = 0i128;
        let mut in_1w = 0i128;
        let mut longer = 0i128;

        for sleeve in sleeves.iter() {
            let value = match sleeve_value(&env, &oracle, &sleeve, NavKind::Collateral, scale) {
                Ok((v, _)) => v,
                Err(e) => panic_with_error!(&env, e),
            };
            if sleeve.sleeve_type == SleeveType::CashStable {
                cash_balance = add(cash_balance, value);
            } else {
                match sleeve.liquidity_tier {
                    0 | 1 => in_24h = add(in_24h, value),
                    2 => in_1w = add(in_1w, value),
                    _ => longer = add(longer, value),
                }
            }
        }

        let total_a = Self::total_assets(env.clone());
        let floor_bps = get_policy_floor_bps(&env, &config);
        let instant = calculate_instant_capacity(cash_balance, total_a, floor_bps);

        RedemptionCapacity {
            instant,
            in_24h,
            in_1w,
            longer,
            computed_at: env.ledger().timestamp(),
        }
    }

    /// Collateral Passport risk sheet (audit W7). The haircut is the
    /// value-weighted Pillar 3 haircut of the basket (target-weighted while the
    /// vault is empty), rounded UP so it always errs on the conservative side.
    pub fn get_risk_sheet(env: Env) -> RiskSheet {
        let config = get_config(&env);
        let sleeves: Vec<SleeveState> = env
            .storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env));
        let scale = share_scale(&env);
        let oracle = NavOracleClient::new(&env, &config.oracle_contract);
        let overflow = |v: Option<i128>| -> i128 {
            match v {
                Some(x) => x,
                None => panic_with_error!(&env, VaultError::MathOverflow),
            }
        };

        let mut values: Vec<i128> = Vec::new(&env);
        let mut total = 0i128;
        let mut value_hours = 0i128;
        for sleeve in sleeves.iter() {
            let value = match sleeve_value(&env, &oracle, &sleeve, NavKind::Collateral, scale) {
                Ok((v, _)) => v,
                Err(e) => panic_with_error!(&env, e),
            };
            let tier_hours: i128 = match sleeve.liquidity_tier {
                0 => 0,
                1 => 24,
                2 => 168,
                _ => 720,
            };
            total = overflow(total.checked_add(value));
            value_hours = overflow(value.checked_mul(tier_hours).and_then(|vh| value_hours.checked_add(vh)));
            values.push_back(value);
        }

        // Empty vault: weight by the signed policy's current targets (audit
        // W1b); the stored targets only before any policy is bound.
        let signed = if total > 0 { None } else { signed_targets(&env, &config, &sleeves) };
        let mut weight_sum = 0i128;
        let mut weighted_haircut = 0i128;
        for i in 0..sleeves.len() {
            let sleeve = sleeves.get(i).unwrap();
            let weight = if total > 0 {
                values.get(i).unwrap()
            } else {
                match &signed {
                    Some(t) => t.get_unchecked(i) as i128,
                    None => sleeve.target_weight_bps as i128,
                }
            };
            weight_sum = overflow(weight_sum.checked_add(weight));
            weighted_haircut = overflow(
                weight
                    .checked_mul(sleeve_haircut_bps(sleeve.sleeve_type))
                    .and_then(|x| weighted_haircut.checked_add(x)),
            );
        }
        let haircut = if weight_sum == 0 {
            10000i128
        } else {
            (weighted_haircut + weight_sum - 1) / weight_sum
        };
        let haircut = haircut.clamp(0, 10000) as u32;

        RiskSheet {
            recommended_ltv_bps: 10000 - haircut,
            recommended_haircut_bps: haircut,
            weighted_avg_redemption_hours: if total > 0 { (value_hours / total) as u32 } else { 0 },
            computed_at: env.ledger().timestamp(),
        }
    }

    /// Pillar 4 (audit W7): an open share class (every constituent
    /// permissionless in the registry) may be liquidated by anyone. If any
    /// constituent is permissioned, only SEP-57-verified liquidators qualify;
    /// that verifier is a Phase 3 deliverable, so until it exists no address
    /// is eligible. Assets missing from the registry count as permissioned.
    pub fn is_eligible_liquidator(env: Env, _liquidator: Address) -> bool {
        let config = get_config(&env);
        let registry = AssetRegistryClient::new(&env, &config.registry_contract);
        let sleeves: Vec<SleeveState> = env
            .storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env));
        for sleeve in sleeves.iter() {
            match registry.get_asset_by_address(&sleeve.contract_address) {
                Some(entry) if entry.permissionless => {}
                _ => return false,
            }
        }
        true
    }

    pub fn get_listing_kit(env: Env) -> ListingKit {
        let config = get_config(&env);
        let risk = Self::get_risk_sheet(env.clone());
        ListingKit {
            share_token: env.current_contract_address(),
            numeraire: config.numeraire,
            oracle_contract: config.oracle_contract,
            recommended_ltv_bps: risk.recommended_ltv_bps,
            recommended_haircut_bps: risk.recommended_haircut_bps,
            // Blend-style single collateral factor: the recommended LTV is
            // both the borrow limit and the liquidation point.
            liquidation_threshold_bps: risk.recommended_ltv_bps,
        }
    }

    pub fn get_oracle_status(env: Env) -> Vec<OracleSource> {
        let config = get_config(&env);
        let sleeves: Vec<SleeveState> = env
            .storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env));
        let mut sources: Vec<OracleSource> = Vec::new(&env);
        let oracle = NavOracleClient::new(&env, &config.oracle_contract);
        let registry = AssetRegistryClient::new(&env, &config.registry_contract);

        for sleeve in sleeves.iter() {
            let sym = compute_sleeve_symbol(&env, sleeve.sleeve_type);
            let sleeve_src_ids = oracle.get_sleeve_sources(&sym);
            for src_id in sleeve_src_ids.iter() {
                if let Some(src) = oracle.get_source(&src_id) {
                    let mut exists = false;
                    for s in sources.iter() {
                        if s.source_id == src.source_id {
                            exists = true;
                            break;
                        }
                    }
                    if !exists {
                        sources.push_back(src);
                    }
                }
            }
            if let Some(src) = oracle.get_source(&sym) {
                let mut exists = false;
                for s in sources.iter() {
                    if s.source_id == src.source_id {
                        exists = true;
                        break;
                    }
                }
                if !exists {
                    sources.push_back(src);
                }
            }

            if let Some(entry) = registry.get_asset_by_address(&sleeve.contract_address) {
                for src_id in entry.oracle_sources.iter() {
                    if let Some(src) = oracle.get_source(&src_id) {
                        let mut exists = false;
                        for s in sources.iter() {
                            if s.source_id == src.source_id {
                                exists = true;
                                break;
                            }
                        }
                        if !exists {
                            sources.push_back(src);
                        }
                    }
                }
            }
        }

        sources
    }

    // Sleeve Management & Info
    /// The sleeve layout. Once a policy is bound, `target_weight_bps` shows
    /// the signed policy's CURRENT target (audit W1b), not a stored copy.
    pub fn get_sleeves(env: Env) -> Vec<SleeveState> {
        let mut sleeves: Vec<SleeveState> = env
            .storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env));
        if let Some(targets) = signed_targets(&env, &get_config(&env), &sleeves) {
            for i in 0..sleeves.len() {
                let mut s = sleeves.get_unchecked(i);
                s.target_weight_bps = targets.get_unchecked(i);
                sleeves.set(i, s);
            }
        }
        sleeves
    }

    /// Replace the sleeve layout (targets, bands, tiers). Admin only.
    ///
    /// Audit V2: `current_balance` is NOT settable. Each entry must carry the
    /// balance the vault has already recorded for that asset (0 for a new
    /// sleeve); any other value is rejected with `BalanceNotSettable`. A sleeve
    /// whose asset the vault still holds (recorded or real) cannot be removed.
    ///
    /// Audit W1b: once a policy is bound, the signed policy is the source of
    /// truth for targets - every sleeve must be declared there with exactly
    /// the signed target weight (`SleeveTargetMismatch` otherwise).
    pub fn set_sleeves(env: Env, sleeves: Vec<SleeveState>) {
        let config = get_config(&env);
        config.admin.require_auth();
        let policy = PolicyClient::new(&env, &config.policy_contract);
        let signed = if policy.get_policy_config(&env.current_contract_address()).is_some() {
            Some(policy.get_sleeve_targets(&env.current_contract_address()))
        } else {
            None
        };

        let existing: Vec<SleeveState> = env
            .storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env));
        let registry = AssetRegistryClient::new(&env, &config.registry_contract);

        let mut sum_weights = 0u32;
        let mut cash_count = 0u32;
        for i in 0..sleeves.len() {
            let s = sleeves.get(i).unwrap();
            if s.liquidity_tier > 3 || s.band_bps > 10000 || s.target_weight_bps > 10000 {
                panic_with_error!(&env, VaultError::InvalidBps);
            }
            for j in (i + 1)..sleeves.len() {
                if sleeves.get(j).unwrap().contract_address == s.contract_address {
                    panic_with_error!(&env, VaultError::InvalidSleeveConfig);
                }
            }
            if s.sleeve_type == SleeveType::CashStable {
                cash_count += 1;
                if s.contract_address != config.numeraire {
                    panic_with_error!(&env, VaultError::InvalidSleeveConfig);
                }
            }
            // Audit M4: every sleeve (cash included) must be an active registry
            // entry and carry exactly its registry ID as label.
            let (label, _) = registry_label(&env, &registry, &s.contract_address);
            if s.asset_id != label {
                panic_with_error!(&env, VaultError::InvalidSleeveConfig);
            }
            if let Some(targets) = &signed {
                let id = match registry.get_asset_by_address(&s.contract_address) {
                    Some(e) => e.asset_id,
                    None => panic_with_error!(&env, VaultError::AssetNotRegistered),
                };
                match targets.iter().find(|t| t.asset_id == id) {
                    Some(t) if t.target_weight_bps == s.target_weight_bps => {}
                    _ => panic_with_error!(&env, VaultError::SleeveTargetMismatch),
                }
            }

            let mut recorded = 0i128;
            for e in existing.iter() {
                if e.contract_address == s.contract_address {
                    recorded = e.current_balance;
                    break;
                }
            }
            if s.current_balance != recorded {
                panic_with_error!(&env, VaultError::BalanceNotSettable);
            }
            sum_weights = sum_weights.saturating_add(s.target_weight_bps);
        }

        if cash_count != 1 {
            panic_with_error!(&env, VaultError::InvalidSleeveConfig);
        }
        if sum_weights != 10000 {
            panic_with_error!(&env, VaultError::InvalidBps);
        }

        // No removal of a sleeve whose asset the vault still holds.
        for e in existing.iter() {
            let mut kept = false;
            for s in sleeves.iter() {
                if s.contract_address == e.contract_address {
                    kept = true;
                    break;
                }
            }
            if !kept && (e.current_balance != 0 || real_holding(&env, &e) != 0) {
                panic_with_error!(&env, VaultError::SleeveHasHoldings);
            }
        }

        env.storage().instance().set(&DataKey::Sleeves, &sleeves);
        SleevesUpdated { admin: config.admin, count: sleeves.len() }.publish(&env);
    }

    pub fn get_config(env: Env) -> VaultConfig {
        get_config(&env)
    }

    // Fee Engine
    pub fn accrue_fees(env: Env) -> i128 {
        let config = get_config(&env);
        accrue_fees_internal(&env, &config)
    }

    pub fn get_last_fee_accrual(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::LastFeeAccrual)
            .unwrap_or(0)
    }

    // Circuit Breakers & Admin
    pub fn pause(env: Env) {
        let config = get_config(&env);
        config.pause_authority.require_auth();

        env.storage().instance().set(&DataKey::IsPaused, &true);
        VaultPaused { by: config.pause_authority, timestamp: env.ledger().timestamp() }.publish(&env);
    }

    pub fn unpause(env: Env) {
        let config = get_config(&env);
        config.admin.require_auth();

        env.storage().instance().set(&DataKey::IsPaused, &false);
        VaultUnpaused { admin: config.admin, timestamp: env.ledger().timestamp() }.publish(&env);
    }

    pub fn is_paused(env: Env) -> bool {
        is_vault_paused(&env)
    }

    /// Schedule a WASM upgrade (admin). Executable only after
    /// UPGRADE_TIMELOCK_SECONDS so holders can review it and exit (audit W3).
    pub fn propose_upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let config = get_config(&env);
        config.admin.require_auth();
        let now = env.ledger().timestamp();
        let pending = PendingUpgrade {
            wasm_hash: new_wasm_hash.clone(),
            proposed_at: now,
            executable_at: now + UPGRADE_TIMELOCK_SECONDS,
        };
        env.storage().instance().set(&DataKey::PendingUpgrade, &pending);
        UpgradeProposed {
            admin: config.admin,
            wasm_hash: new_wasm_hash,
            executable_at: pending.executable_at,
        }
        .publish(&env);
    }

    /// Execute the proposed upgrade once its timelock has elapsed (admin).
    pub fn execute_upgrade(env: Env) {
        let config = get_config(&env);
        config.admin.require_auth();
        let pending: PendingUpgrade = match env.storage().instance().get(&DataKey::PendingUpgrade) {
            Some(p) => p,
            None => panic_with_error!(&env, VaultError::NoPendingUpgrade),
        };
        if env.ledger().timestamp() < pending.executable_at {
            panic_with_error!(&env, VaultError::UpgradeNotReady);
        }
        env.storage().instance().remove(&DataKey::PendingUpgrade);
        Upgrade { admin: config.admin, wasm_hash: pending.wasm_hash.clone() }.publish(&env);
        env.deployer()
            .update_current_contract(soroban_sdk::ContractExecutable::Wasm(pending.wasm_hash));
    }

    /// Drop a proposed upgrade (admin).
    pub fn cancel_upgrade(env: Env) {
        let config = get_config(&env);
        config.admin.require_auth();
        let pending: PendingUpgrade = match env.storage().instance().get(&DataKey::PendingUpgrade) {
            Some(p) => p,
            None => panic_with_error!(&env, VaultError::NoPendingUpgrade),
        };
        env.storage().instance().remove(&DataKey::PendingUpgrade);
        UpgradeCancelled { admin: config.admin, wasm_hash: pending.wasm_hash }.publish(&env);
    }

    pub fn get_pending_upgrade(env: Env) -> Option<PendingUpgrade> {
        env.storage().instance().get(&DataKey::PendingUpgrade)
    }

    pub fn admin_pause(env: Env) {
        let config = get_config(&env);
        config.admin.require_auth();

        env.storage().instance().set(&DataKey::IsPaused, &true);
        VaultPaused { by: config.admin, timestamp: env.ledger().timestamp() }.publish(&env);
    }

    pub fn set_pause_authority(env: Env, new_pause_authority: Address) {
        let mut config = get_config(&env);
        config.admin.require_auth();
        config.pause_authority = new_pause_authority;
        env.storage().instance().set(&DataKey::Config, &config);
    }

    /// Bind this vault to the policy its `owner` signed (audit P3).
    ///
    /// Requires the vault admin (who chooses which owner governs the vault)
    /// AND the policy owner, whose signature on `register_policy` covers the
    /// policy hash and every enforced term. The vault's own consent is given by
    /// it being the invoker. Registration is one-time in the policy contract;
    /// later amendments are signed by the owner directly on the policy contract.
    /// Designate the hot keeper key allowed to SUBMIT rebalances (admin).
    /// The keeper cannot move value beyond what the signed policy validates.
    pub fn set_keeper(env: Env, keeper: Address) {
        get_config(&env).admin.require_auth();
        env.storage().instance().set(&DataKey::Keeper, &keeper);
    }

    pub fn get_keeper(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Keeper)
    }

    /// Propose the executor contract for a trade route (admin). A router can
    /// move vault funds, so it takes effect only after 48 h (anyone activates).
    pub fn propose_router(env: Env, route: TradeRoute, router: Address) {
        get_config(&env).admin.require_auth();
        let effective_at = env.ledger().timestamp().saturating_add(ROUTER_CHANGE_DELAY_SECONDS);
        env.storage().instance().set(&DataKey::PendingRouter(route as u32), &PendingRouter { router: router.clone(), effective_at });
        RouterProposed { route: route as u32, router, effective_at }.publish(&env);
    }

    pub fn activate_router(env: Env, route: TradeRoute) {
        let pending: PendingRouter = env.storage().instance().get(&DataKey::PendingRouter(route as u32))
            .unwrap_or_else(|| panic_with_error!(&env, VaultError::RouterNotReady));
        if env.ledger().timestamp() < pending.effective_at {
            panic_with_error!(&env, VaultError::RouterNotReady);
        }
        env.storage().instance().remove(&DataKey::PendingRouter(route as u32));
        env.storage().instance().set(&DataKey::Router(route as u32), &pending.router);
    }

    pub fn get_router(env: Env, route: TradeRoute) -> Option<Address> {
        env.storage().instance().get(&DataKey::Router(route as u32))
    }

    /// Execute a rebalance proposal (audit W1). The keeper submits; the signed
    /// policy must validate the proposal; every swap is measured on the
    /// vault's own balances and must deliver at least `min_amount_out`;
    /// sleeve holdings are booked from those measured deltas only. Finally
    /// (audit W1b) the vault's ACTUAL holdings - not the claimed
    /// `weights_after` - must satisfy the signed bands, caps and floor.
    pub fn rebalance(env: Env, proposal: RebalanceProposal) {
        let keeper: Address = env.storage().instance().get(&DataKey::Keeper)
            .unwrap_or_else(|| panic_with_error!(&env, VaultError::Unauthorized));
        if proposal.proposer != keeper {
            panic_with_error!(&env, VaultError::Unauthorized);
        }
        keeper.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        let config = get_config(&env);
        if proposal.vault != env.current_contract_address() {
            panic_with_error!(&env, VaultError::Unauthorized);
        }
        let verdict = PolicyClient::new(&env, &config.policy_contract)
            .validate_rebalance(&env.current_contract_address(), &proposal);
        if !verdict.is_valid {
            panic_with_error!(&env, VaultError::RebalanceRejected);
        }

        let mut sleeves: Vec<SleeveState> = env.storage().instance().get(&DataKey::Sleeves).unwrap_or_else(|| Vec::new(&env));
        for op in proposal.operations.iter() {
            let has = |a: &Address| sleeves.iter().any(|s| s.contract_address == *a);
            if !has(&op.asset_in) || !has(&op.asset_out) {
                panic_with_error!(&env, VaultError::SleeveNotFound);
            }
            let i_in = sleeve_index(&sleeves, &op.asset_in);
            let i_out = sleeve_index(&sleeves, &op.asset_out);
            let mut s_in = sleeves.get_unchecked(i_in);
            if op.amount_in > sleeve_holdings(&env, &s_in) {
                panic_with_error!(&env, VaultError::InsufficientBalance);
            }
            let (spent, received) = match op.route {
                TradeRoute::Soroswap => {
                    // A Blend position is not a token: never route it through an AMM.
                    if blend_of(&env, &s_in).is_some() || blend_of(&env, &sleeves.get_unchecked(i_out)).is_some() {
                        panic_with_error!(&env, VaultError::RouteUnavailable);
                    }
                    let router: Address = env.storage().instance().get(&DataKey::Router(TradeRoute::Soroswap as u32))
                        .unwrap_or_else(|| panic_with_error!(&env, VaultError::RouteUnavailable));
                    swap_on_soroswap(&env, &router, &op, proposal.deadline)
                }
                TradeRoute::Blend => blend_leg(&env, &config, &op),
                _ => panic_with_error!(&env, VaultError::RouteUnavailable),
            };
            if spent > op.amount_in || received < op.min_amount_out {
                panic_with_error!(&env, VaultError::SlippageExceeded);
            }
            s_in.current_balance -= spent;
            sleeves.set(i_in, s_in);
            let mut s_out = sleeves.get_unchecked(i_out);
            s_out.current_balance += received;
            sleeves.set(i_out, s_out);
            RebalanceStep { proposer: keeper.clone(), asset_in: op.asset_in.clone(), asset_out: op.asset_out.clone(), spent, received }.publish(&env);
        }
        check_actual_allocation(&env, &config, &sleeves);
        env.storage().instance().set(&DataKey::Sleeves, &sleeves);
        RebalanceComplete { proposer: keeper, steps: proposal.operations.len() }.publish(&env);
    }

    pub fn bind_policy(
        env: Env,
        owner: Address,
        policy_hash: BytesN<32>,
        min_liquidity_sleeve_bps: u32,
        max_single_asset_bps: u32,
        max_issuer_bps: u32,
        max_slippage_bps: u32,
        rebalance_band_bps: u32,
        sleeves: Vec<PolicySleeveTargetRef>,
    ) {
        let config = get_config(&env);
        config.admin.require_auth();

        PolicyClient::new(&env, &config.policy_contract).register_policy(
            &env.current_contract_address(),
            &owner,
            &policy_hash,
            &config.registry_contract,
            &min_liquidity_sleeve_bps,
            &max_single_asset_bps,
            &max_issuer_bps,
            &max_slippage_bps,
            &rebalance_band_bps,
            &sleeves,
        );

        PolicyBound { owner, policy_hash }.publish(&env);
    }

    pub fn get_policy_hash(env: Env) -> Option<BytesN<32>> {
        let config = get_config(&env);
        let policy = PolicyClient::new(&env, &config.policy_contract);
        policy
            .get_policy_config(&env.current_contract_address())
            .map(|c| c.policy_hash)
    }
}

#[cfg(test)]
mod test;
