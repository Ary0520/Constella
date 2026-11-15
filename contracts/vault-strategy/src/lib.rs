#![no_std]
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    token, Address, BytesN, Env, String, Symbol, Vec,
};

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
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct AllowanceDataKey {
    pub from: Address,
    pub spender: Address,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct AllowanceValue {
    pub amount: i128,
    pub expiration_ledger: u32,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum DataKey {
    Config,
    Name,
    Symbol,
    Decimals,
    TotalShares,
    Balance(Address),
    Allowance(AllowanceDataKey),
    Sleeves,
    IsPaused,
    LastFeeAccrual,
}

// Cross-Contract Client Traits for Decoupled WASM Compilation
#[contractclient(name = "NavOracleClient")]
pub trait NavOracleInterface {
    fn get_collateral_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>;
    fn get_deposit_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>;
    fn price(env: Env, asset: Address) -> Option<PriceData>;
    fn get_source(env: Env, source_id: Symbol) -> Option<OracleSource>;
    fn get_sleeve_sources(env: Env, sleeve_id: Symbol) -> Vec<Symbol>;
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryAssetEntryRef {
    pub contract_address: Address,
    pub asset_id: Symbol,
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
        Some(c) => c,
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

fn accrue_fees_internal(env: &Env, config: &VaultConfig) -> i128 {
    let last_accrual: u64 = env
        .storage()
        .instance()
        .get(&DataKey::LastFeeAccrual)
        .unwrap_or(0);
    let now = env.ledger().timestamp();
    if now <= last_accrual || config.management_fee_bps == 0 {
        return 0;
    }

    let elapsed = now - last_accrual;
    let total_shares: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalShares)
        .unwrap_or(0);

    if total_shares == 0 {
        env.storage().instance().set(&DataKey::LastFeeAccrual, &now);
        return 0;
    }

    let numerator = total_shares
        .checked_mul(config.management_fee_bps as i128)
        .and_then(|v| v.checked_mul(elapsed as i128));
    let denominator = (10_000i128).checked_mul(SECONDS_PER_YEAR as i128);

    let d_shares = match (numerator, denominator) {
        (Some(num), Some(den)) if den > 0 => num / den,
        _ => 0,
    };

    if d_shares > 0 {
        let treasury_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(config.treasury.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::Balance(config.treasury.clone()), &(treasury_bal + d_shares));
        let new_total_shares = total_shares + d_shares;
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &new_total_shares);

        if let (Some(den), Some(fee_rate)) = (
            denominator,
            total_shares.checked_mul(config.management_fee_bps as i128),
        ) {
            if fee_rate > 0 {
                let accrued_time = (d_shares * den) / fee_rate;
                let new_last = last_accrual + accrued_time as u64;
                env.storage()
                    .instance()
                    .set(&DataKey::LastFeeAccrual, &new_last);
            } else {
                env.storage().instance().set(&DataKey::LastFeeAccrual, &now);
            }
        } else {
            env.storage().instance().set(&DataKey::LastFeeAccrual, &now);
        }

        env.events().publish(
            (Symbol::new(env, "fee_accrued"), config.treasury.clone()),
            d_shares,
        );
        d_shares
    } else {
        0
    }
}

fn compute_deposit_nav(env: &Env, config: &VaultConfig) -> Option<PriceData> {
    let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
    let scale = 10i128.pow(decimals);
    let total_shares: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalShares)
        .unwrap_or(0);
    let sleeves: Vec<SleeveState> = env
        .storage()
        .instance()
        .get(&DataKey::Sleeves)
        .unwrap_or_else(|| Vec::new(env));

    let oracle = NavOracleClient::new(env, &config.oracle_contract);
    let vault_sym = Symbol::new(env, "VAULT");
    if let Some(pd) = oracle.get_deposit_nav(&vault_sym) {
        return Some(pd);
    }

    let mut total_value = 0i128;
    let mut latest_ts = env.ledger().timestamp();
    let mut has_sleeves = false;

    for sleeve in sleeves.iter() {
        has_sleeves = true;
        if sleeve.sleeve_type == SleeveType::CashStable {
            total_value = total_value.checked_add(sleeve_holdings(env, &sleeve))?;
        } else {
            // Audit V3: a sleeve that holds nothing contributes exactly 0, so an
            // unpriced (paused / stale / unconfigured) empty sleeve must not
            // block pricing. A FUNDED sleeve with no valid price still makes
            // this return None (hard stop) until queued in-kind claims exist (V6).
            let held = sleeve_holdings(env, &sleeve);
            if held == 0 {
                continue;
            }
            let sym = compute_sleeve_symbol(env, sleeve.sleeve_type);
            let pd = oracle.get_deposit_nav(&sym)?;
            if pd.timestamp > latest_ts {
                latest_ts = pd.timestamp;
            }
            let val = held.checked_mul(pd.price)? / scale;
            total_value = total_value.checked_add(val)?;
        }
    }

    if !has_sleeves || total_shares == 0 {
        Some(PriceData {
            price: scale,
            timestamp: latest_ts,
            source: Symbol::new(env, "MEDIAN_NAV"),
        })
    } else {
        let nav_per_share = (total_value.checked_mul(scale)?) / total_shares;
        Some(PriceData {
            price: nav_per_share,
            timestamp: latest_ts,
            source: Symbol::new(env, "MEDIAN_NAV"),
        })
    }
}

fn compute_conservative_nav(env: &Env, config: &VaultConfig) -> Option<PriceData> {
    let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
    let scale = 10i128.pow(decimals);
    let total_shares: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalShares)
        .unwrap_or(0);
    let sleeves: Vec<SleeveState> = env
        .storage()
        .instance()
        .get(&DataKey::Sleeves)
        .unwrap_or_else(|| Vec::new(env));

    let oracle = NavOracleClient::new(env, &config.oracle_contract);
    let vault_sym = Symbol::new(env, "VAULT");
    if let Some(pd) = oracle.get_collateral_nav(&vault_sym) {
        return Some(pd);
    }

    let mut total_value = 0i128;
    let mut latest_ts = env.ledger().timestamp();
    let mut has_sleeves = false;

    for sleeve in sleeves.iter() {
        has_sleeves = true;
        if sleeve.sleeve_type == SleeveType::CashStable {
            total_value = total_value.checked_add(sleeve_holdings(env, &sleeve))?;
        } else {
            // Audit V3: a sleeve that holds nothing contributes exactly 0, so an
            // unpriced (paused / stale / unconfigured) empty sleeve must not
            // block pricing. A FUNDED sleeve with no valid price still makes
            // this return None (hard stop) until queued in-kind claims exist (V6).
            let held = sleeve_holdings(env, &sleeve);
            if held == 0 {
                continue;
            }
            let sym = compute_sleeve_symbol(env, sleeve.sleeve_type);
            let pd = oracle.get_collateral_nav(&sym)?;
            if pd.timestamp > latest_ts {
                latest_ts = pd.timestamp;
            }
            let val = held.checked_mul(pd.price)? / scale;
            total_value = total_value.checked_add(val)?;
        }
    }

    if !has_sleeves || total_shares == 0 {
        Some(PriceData {
            price: scale,
            timestamp: latest_ts,
            source: Symbol::new(env, "CONSERVATIVE_NAV"),
        })
    } else {
        let nav_per_share = (total_value.checked_mul(scale)?) / total_shares;
        Some(PriceData {
            price: nav_per_share,
            timestamp: latest_ts,
            source: Symbol::new(env, "CONSERVATIVE_NAV"),
        })
    }
}

/// Tokens of `asset` actually held by this vault right now.
fn real_balance(env: &Env, asset: &Address) -> i128 {
    token::Client::new(env, asset).balance(&env.current_contract_address())
}

/// Holdings used for every valuation: min(recorded, actually held).
///
/// Audit V2: recorded balances change only through value-moving flows
/// (deposit / mint / redeem / withdraw, and future policy-validated
/// rebalances). Capping by the real token balance means an issuer clawback or
/// freeze-and-burn lowers NAV immediately, while unsolicited transfers into the
/// vault never raise it (no donation-based share-price manipulation).
fn sleeve_holdings(env: &Env, sleeve: &SleeveState) -> i128 {
    let held = real_balance(env, &sleeve.contract_address);
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
    let total_shares: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalShares)
        .unwrap_or(0);
    let bal: i128 = env
        .storage()
        .instance()
        .get(&DataKey::Balance(receiver.clone()))
        .unwrap_or(0);

    env.storage()
        .instance()
        .set(&DataKey::TotalShares, &(total_shares + shares));
    env.storage()
        .instance()
        .set(&DataKey::Balance(receiver.clone()), &(bal + shares));

    env.events().publish(
        (Symbol::new(env, "mint"), receiver.clone()),
        shares,
    );
}

/// Moves `amount` shares from `from` to `to` (SEP-41 transfer semantics).
///
/// Audit V1: the previous inline implementation read `to`'s balance before
/// debiting `from`, so `from == to` credited a stale balance and minted
/// `amount` new shares. Here the sender is always debited first and a
/// self-transfer is a balance no-op (the caller still had to authorize it and
/// must own at least `amount`).
fn move_shares(env: &Env, from: &Address, to: &Address, amount: i128) {
    let from_bal: i128 = env
        .storage()
        .instance()
        .get(&DataKey::Balance(from.clone()))
        .unwrap_or(0);
    if from_bal < amount {
        panic_with_error!(env, VaultError::InsufficientBalance);
    }
    if from == to {
        return;
    }

    let new_from_bal = from_bal - amount;
    if new_from_bal == 0 {
        env.storage().instance().remove(&DataKey::Balance(from.clone()));
    } else {
        env.storage()
            .instance()
            .set(&DataKey::Balance(from.clone()), &new_from_bal);
    }

    let to_bal: i128 = env
        .storage()
        .instance()
        .get(&DataKey::Balance(to.clone()))
        .unwrap_or(0);
    let new_to_bal = match to_bal.checked_add(amount) {
        Some(v) => v,
        None => panic_with_error!(env, VaultError::MathOverflow),
    };
    env.storage()
        .instance()
        .set(&DataKey::Balance(to.clone()), &new_to_bal);
}

fn burn_shares_internal(env: &Env, owner: &Address, shares: i128) {
    let total_shares: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalShares)
        .unwrap_or(0);
    let bal: i128 = env
        .storage()
        .instance()
        .get(&DataKey::Balance(owner.clone()))
        .unwrap_or(0);

    let new_total = total_shares - shares;
    let new_bal = bal - shares;

    env.storage().instance().set(&DataKey::TotalShares, &new_total);
    if new_bal == 0 {
        env.storage().instance().remove(&DataKey::Balance(owner.clone()));
    } else {
        env.storage().instance().set(&DataKey::Balance(owner.clone()), &new_bal);
    }

    env.events().publish(
        (Symbol::new(env, "burn"), owner.clone()),
        shares,
    );
}

#[contract]
pub struct StrategyVault;

#[contractimpl]
impl StrategyVault {
    pub fn initialize(
        env: Env,
        config: VaultConfig,
        name: String,
        symbol: String,
        decimals: u32,
    ) {
        if env.storage().instance().has(&DataKey::Config) {
            panic_with_error!(&env, VaultError::AlreadyInitialized);
        }
        if config.management_fee_bps > 10000 || decimals > 18 {
            panic_with_error!(&env, VaultError::InvalidBps);
        }

        let admin = config.admin.clone();
        let numeraire = config.numeraire.clone();

        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::TotalShares, &0i128);
        env.storage().instance().set(&DataKey::IsPaused, &false);
        env.storage()
            .instance()
            .set(&DataKey::LastFeeAccrual, &env.ledger().timestamp());

        let mut sleeves = Vec::new(&env);
        sleeves.push_back(SleeveState {
            asset_id: String::from_str(&env, "USDC"),
            contract_address: numeraire,
            target_weight_bps: 10000,
            band_bps: 0,
            current_balance: 0,
            sleeve_type: SleeveType::CashStable,
            liquidity_tier: 0,
        });
        env.storage().instance().set(&DataKey::Sleeves, &sleeves);

        env.events().publish(
            (Symbol::new(&env, "vault_initialized"), admin),
            env.ledger().timestamp(),
        );
    }

    // SEP-56 Vault Operations
    pub fn deposit(env: Env, assets: i128, receiver: Address) -> i128 {
        receiver.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if assets <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let deposit_nav = match compute_deposit_nav(&env, &config) {
            Some(pd) => pd.price,
            None => panic_with_error!(&env, VaultError::OracleUnavailable),
        };
        if deposit_nav <= 0 {
            panic_with_error!(&env, VaultError::OracleUnavailable);
        }

        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);

        let shares = (assets.checked_mul(scale).unwrap()) / deposit_nav;
        if shares <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }

        token::Client::new(&env, &config.numeraire).transfer(
            &receiver,
            &env.current_contract_address(),
            &assets,
        );

        update_cash_sleeve_balance(&env, assets, true);
        mint_shares_internal(&env, &receiver, shares);

        env.events().publish(
            (Symbol::new(&env, "deposit"), receiver),
            (assets, shares),
        );

        shares
    }

    pub fn mint(env: Env, shares: i128, receiver: Address) -> i128 {
        receiver.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if shares <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let deposit_nav = match compute_deposit_nav(&env, &config) {
            Some(pd) => pd.price,
            None => panic_with_error!(&env, VaultError::OracleUnavailable),
        };
        if deposit_nav <= 0 {
            panic_with_error!(&env, VaultError::OracleUnavailable);
        }

        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);

        let assets = (shares.checked_mul(deposit_nav).unwrap() + scale - 1) / scale;
        if assets <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }

        token::Client::new(&env, &config.numeraire).transfer(
            &receiver,
            &env.current_contract_address(),
            &assets,
        );

        update_cash_sleeve_balance(&env, assets, true);
        mint_shares_internal(&env, &receiver, shares);

        env.events().publish(
            (Symbol::new(&env, "deposit"), receiver),
            (assets, shares),
        );

        assets
    }

    pub fn redeem(env: Env, shares: i128, receiver: Address, owner: Address) -> i128 {
        owner.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if shares <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }

        let owner_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(owner.clone()))
            .unwrap_or(0);
        if owner_bal < shares {
            panic_with_error!(&env, VaultError::InsufficientBalance);
        }

        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let conservative_nav = match compute_conservative_nav(&env, &config) {
            Some(pd) => pd.price,
            None => panic_with_error!(&env, VaultError::OracleUnavailable),
        };
        if conservative_nav <= 0 {
            panic_with_error!(&env, VaultError::OracleUnavailable);
        }

        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);

        let assets = (shares.checked_mul(conservative_nav).unwrap()) / scale;
        if assets <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }

        let cash_balance = get_cash_sleeve_balance(&env);
        if assets > cash_balance {
            panic_with_error!(&env, VaultError::ExceedsRedemptionCapacity);
        }

        let total_a = Self::total_assets(env.clone());
        let floor_bps = get_policy_floor_bps(&env, &config);
        if total_a > assets && floor_bps > 0 {
            let remaining_assets = total_a - assets;
            let remaining_cash = cash_balance - assets;
            if remaining_cash.checked_mul(10000).unwrap()
                < remaining_assets.checked_mul(floor_bps as i128).unwrap()
            {
                panic_with_error!(&env, VaultError::LiquidityFloorBreached);
            }
        }

        burn_shares_internal(&env, &owner, shares);
        update_cash_sleeve_balance(&env, assets, false);

        token::Client::new(&env, &config.numeraire).transfer(
            &env.current_contract_address(),
            &receiver,
            &assets,
        );

        env.events().publish(
            (Symbol::new(&env, "withdraw"), receiver, owner),
            (assets, shares),
        );

        assets
    }

    pub fn withdraw(env: Env, assets: i128, receiver: Address, owner: Address) -> i128 {
        owner.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if assets <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }

        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        let conservative_nav = match compute_conservative_nav(&env, &config) {
            Some(pd) => pd.price,
            None => panic_with_error!(&env, VaultError::OracleUnavailable),
        };
        if conservative_nav <= 0 {
            panic_with_error!(&env, VaultError::OracleUnavailable);
        }

        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);

        let shares = (assets.checked_mul(scale).unwrap() + (conservative_nav - 1)) / conservative_nav;
        let owner_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(owner.clone()))
            .unwrap_or(0);
        if owner_bal < shares {
            panic_with_error!(&env, VaultError::InsufficientBalance);
        }

        let cash_balance = get_cash_sleeve_balance(&env);
        if assets > cash_balance {
            panic_with_error!(&env, VaultError::ExceedsRedemptionCapacity);
        }

        let total_a = Self::total_assets(env.clone());
        let floor_bps = get_policy_floor_bps(&env, &config);
        if total_a > assets && floor_bps > 0 {
            let remaining_assets = total_a - assets;
            let remaining_cash = cash_balance - assets;
            if remaining_cash.checked_mul(10000).unwrap()
                < remaining_assets.checked_mul(floor_bps as i128).unwrap()
            {
                panic_with_error!(&env, VaultError::LiquidityFloorBreached);
            }
        }

        burn_shares_internal(&env, &owner, shares);
        update_cash_sleeve_balance(&env, assets, false);

        token::Client::new(&env, &config.numeraire).transfer(
            &env.current_contract_address(),
            &receiver,
            &assets,
        );

        env.events().publish(
            (Symbol::new(&env, "withdraw"), receiver, owner),
            (assets, shares),
        );

        shares
    }

    pub fn total_assets(env: Env) -> i128 {
        let config = get_config(&env);
        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);
        let sleeves: Vec<SleeveState> = env
            .storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env));

        let mut total = 0i128;
        let oracle = NavOracleClient::new(&env, &config.oracle_contract);

        for sleeve in sleeves.iter() {
            if sleeve.sleeve_type == SleeveType::CashStable {
                total = match total.checked_add(sleeve_holdings(&env, &sleeve)) {
                    Some(v) => v,
                    None => panic_with_error!(&env, VaultError::MathOverflow),
                };
            } else {
                let sym = compute_sleeve_symbol(&env, sleeve.sleeve_type);
                let price = if let Some(pd) = oracle.get_collateral_nav(&sym) {
                    pd.price
                } else {
                    scale
                };
                let val = match sleeve_holdings(&env, &sleeve).checked_mul(price) {
                    Some(product) => product / scale,
                    None => panic_with_error!(&env, VaultError::MathOverflow),
                };
                total = match total.checked_add(val) {
                    Some(v) => v,
                    None => panic_with_error!(&env, VaultError::MathOverflow),
                };
            }
        }
        total
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalShares)
            .unwrap_or(0)
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

    pub fn asset(env: Env) -> Address {
        get_config(&env).numeraire
    }

    pub fn max_deposit(env: Env, _receiver: Address) -> i128 {
        if is_vault_paused(&env) {
            0
        } else {
            i128::MAX
        }
    }

    pub fn max_mint(env: Env, _receiver: Address) -> i128 {
        if is_vault_paused(&env) {
            0
        } else {
            i128::MAX
        }
    }

    pub fn max_withdraw(env: Env, owner: Address) -> i128 {
        if is_vault_paused(&env) {
            return 0;
        }
        let owner_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(owner))
            .unwrap_or(0);
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
        let owner_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(owner))
            .unwrap_or(0);
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
        env.storage()
            .instance()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if amount <= 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }
        let config = get_config(&env);
        accrue_fees_internal(&env, &config);

        move_shares(&env, &from, &to, amount);

        env.events().publish(
            (Symbol::new(&env, "transfer"), from, to),
            amount,
        );
    }

    pub fn approve(env: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32) {
        from.require_auth();
        if is_vault_paused(&env) {
            panic_with_error!(&env, VaultError::VaultPaused);
        }
        if amount < 0 {
            panic_with_error!(&env, VaultError::ZeroAmount);
        }

        let key = DataKey::Allowance(AllowanceDataKey {
            from: from.clone(),
            spender: spender.clone(),
        });
        if amount == 0 {
            env.storage().instance().remove(&key);
        } else {
            let val = AllowanceValue {
                amount,
                expiration_ledger,
            };
            env.storage().instance().set(&key, &val);
        }

        env.events().publish(
            (Symbol::new(&env, "approve"), from, spender),
            amount,
        );
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        let key = DataKey::Allowance(AllowanceDataKey { from, spender });
        if let Some(val) = env.storage().instance().get::<_, AllowanceValue>(&key) {
            if env.ledger().sequence() <= val.expiration_ledger {
                val.amount
            } else {
                0
            }
        } else {
            0
        }
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

        let key = DataKey::Allowance(AllowanceDataKey {
            from: from.clone(),
            spender: spender.clone(),
        });
        let mut val: AllowanceValue = match env.storage().instance().get(&key) {
            Some(v) => v,
            None => panic_with_error!(&env, VaultError::InsufficientAllowance),
        };
        if env.ledger().sequence() > val.expiration_ledger || val.amount < amount {
            panic_with_error!(&env, VaultError::InsufficientAllowance);
        }
        val.amount -= amount;
        if val.amount == 0 {
            env.storage().instance().remove(&key);
        } else {
            env.storage().instance().set(&key, &val);
        }

        move_shares(&env, &from, &to, amount);

        env.events().publish(
            (Symbol::new(&env, "transfer"), from, to),
            amount,
        );
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
        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);
        let oracle = NavOracleClient::new(&env, &config.oracle_contract);

        let mut cash_balance = 0i128;
        let mut in_24h = 0i128;
        let mut in_1w = 0i128;
        let mut longer = 0i128;

        for sleeve in sleeves.iter() {
            let sym = compute_sleeve_symbol(&env, sleeve.sleeve_type);
            let price = if sleeve.sleeve_type == SleeveType::CashStable {
                scale
            } else if let Some(pd) = oracle.get_collateral_nav(&sym) {
                pd.price
            } else {
                scale
            };
            let held = sleeve_holdings(&env, &sleeve);
            let value = (held * price) / scale;

            if sleeve.sleeve_type == SleeveType::CashStable {
                cash_balance += held;
            } else {
                match sleeve.liquidity_tier {
                    0 | 1 => in_24h += value,
                    2 => in_1w += value,
                    _ => longer += value,
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

    pub fn get_risk_sheet(env: Env) -> RiskSheet {
        let config = get_config(&env);
        let sleeves: Vec<SleeveState> = env
            .storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env));
        let total_a = Self::total_assets(env.clone());
        let decimals: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
        let scale = 10i128.pow(decimals);
        let oracle = NavOracleClient::new(&env, &config.oracle_contract);

        let mut weighted_hours = 0u64;
        if total_a > 0 {
            let mut total_val_hours = 0i128;
            for sleeve in sleeves.iter() {
                let tier_hours: i128 = match sleeve.liquidity_tier {
                    0 => 0,
                    1 => 24,
                    2 => 168,
                    _ => 720,
                };
                let price = if sleeve.sleeve_type == SleeveType::CashStable {
                    scale
                } else {
                    let sym = compute_sleeve_symbol(&env, sleeve.sleeve_type);
                    if let Some(pd) = oracle.get_collateral_nav(&sym) {
                        pd.price
                    } else {
                        scale
                    }
                };
                let val = (sleeve_holdings(&env, &sleeve) * price) / scale;
                total_val_hours += val * tier_hours;
            }
            weighted_hours = (total_val_hours / total_a) as u64;
        }

        let haircut = 200u32;
        let recommended_ltv = 10000u32 - haircut;

        RiskSheet {
            recommended_ltv_bps: recommended_ltv,
            recommended_haircut_bps: haircut,
            weighted_avg_redemption_hours: weighted_hours as u32,
            computed_at: env.ledger().timestamp(),
        }
    }

    pub fn is_eligible_liquidator(_env: Env, _liquidator: Address) -> bool {
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
            liquidation_threshold_bps: 9900,
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

        let vault_sym = Symbol::new(&env, "VAULT");
        let vault_src_ids = oracle.get_sleeve_sources(&vault_sym);
        for src_id in vault_src_ids.iter() {
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
        if let Some(src) = oracle.get_source(&vault_sym) {
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

        sources
    }

    // Sleeve Management & Info
    pub fn get_sleeves(env: Env) -> Vec<SleeveState> {
        env.storage()
            .instance()
            .get(&DataKey::Sleeves)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Replace the sleeve layout (targets, bands, tiers). Admin only.
    ///
    /// Audit V2: `current_balance` is NOT settable. Each entry must carry the
    /// balance the vault has already recorded for that asset (0 for a new
    /// sleeve); any other value is rejected with `BalanceNotSettable`. A sleeve
    /// whose asset the vault still holds (recorded or real) cannot be removed.
    pub fn set_sleeves(env: Env, sleeves: Vec<SleeveState>) {
        let config = get_config(&env);
        config.admin.require_auth();

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
            } else if !registry.is_registered(&s.contract_address) {
                panic_with_error!(&env, VaultError::AssetNotRegistered);
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
            if !kept && (e.current_balance != 0 || real_balance(&env, &e.contract_address) != 0) {
                panic_with_error!(&env, VaultError::SleeveHasHoldings);
            }
        }

        env.storage().instance().set(&DataKey::Sleeves, &sleeves);
        env.events().publish(
            (Symbol::new(&env, "sleeves_updated"), config.admin),
            sleeves.len(),
        );
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
        env.events().publish(
            (Symbol::new(&env, "vault_paused"), config.pause_authority),
            env.ledger().timestamp(),
        );
    }

    pub fn unpause(env: Env) {
        let config = get_config(&env);
        config.admin.require_auth();

        env.storage().instance().set(&DataKey::IsPaused, &false);
        env.events().publish(
            (Symbol::new(&env, "vault_unpaused"), config.admin),
            env.ledger().timestamp(),
        );
    }

    pub fn is_paused(env: Env) -> bool {
        is_vault_paused(&env)
    }

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let config = get_config(&env);
        config.admin.require_auth();

        env.deployer().update_current_contract_wasm(new_wasm_hash);
        env.events().publish(
            (Symbol::new(&env, "vault_upgraded"), config.admin),
            env.ledger().timestamp(),
        );
    }

    pub fn admin_pause(env: Env) {
        let config = get_config(&env);
        config.admin.require_auth();

        env.storage().instance().set(&DataKey::IsPaused, &true);
        env.events().publish(
            (Symbol::new(&env, "vault_paused"), config.admin),
            env.ledger().timestamp(),
        );
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

        env.events().publish(
            (Symbol::new(&env, "POLICY_BOUND"), owner),
            policy_hash,
        );
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
