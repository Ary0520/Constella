#![no_std]
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    Address, BytesN,
    Env, FromVal, Symbol, SymbolStr, Vec,
};
use soroban_sdk::xdr::ToXdr;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RegistrySleeveType {
    Cash = 0,
    UsdGovFund = 1,
    EurSovereign = 2,
    YieldUsd = 3,
    Lending = 4,
    Equity = 5,
    Xlm = 6,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryAssetEntry {
    pub contract_address: Address,
    pub asset_id: Symbol,
    pub issuer: Symbol,
    pub sleeve_type: RegistrySleeveType,
    pub liquidity_tier: u32,
    pub permissionless: bool,
    pub oracle_sources: Vec<Symbol>,
    pub eligible_jurisdictions: Vec<Symbol>,
    pub blocked_jurisdictions: Vec<Symbol>,
    pub investor_classes_required: Vec<Symbol>,
    pub is_active: bool,
    pub added_at: u64,
}

/// Mirror of `constella_nav_oracle::PriceData` (decoupled to avoid WASM symbol clashes).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OraclePriceRef {
    pub price: i128,
    pub timestamp: u64,
    pub source: Symbol,
}

/// Mirror of `constella_nav_oracle::BlendPosition` (decision D-H).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendPositionRef {
    pub pool: Address,
    pub underlying: Address,
}

#[contractclient(name = "OraclePriceClient")]
pub trait OraclePriceInterface {
    fn asset_price(env: Env, asset: Address) -> Option<OraclePriceRef>;
    fn blend_position(env: Env, pool: Address) -> Option<BlendPositionRef>;
}

/// Decimals of an asset leg. A Blend position is not a token: its bTokens
/// use the underlying's decimals (blend-contracts-v2 storage.rs: b_supply is
/// "in the underlying token's decimals"), and the oracle names the underlying.
fn leg_decimals(env: &Env, oracle: &OraclePriceClient, asset: &Address) -> u32 {
    match oracle.blend_position(asset) {
        Some(pos) => token::Client::new(env, &pos.underlying).decimals(),
        None => token::Client::new(env, asset).decimals(),
    }
}

#[contractclient(name = "AssetRegistryClient")]
pub trait AssetRegistryInterface {
    fn is_registered(env: Env, contract_address: Address) -> bool;
    fn get_asset(env: Env, asset_id: Symbol) -> Option<RegistryAssetEntry>;
    fn get_asset_by_address(env: Env, contract_address: Address) -> Option<RegistryAssetEntry>;
}

// Events (audit W4.1b: typed #[contractevent]s, byte-identical to the former
// Events::publish encodings; pinned by golden/events.txt).

/// Topics [POLICY_REGISTERED, vault]; data (policy_hash, owner, terms_hash).
#[contractevent(topics = ["POLICY_REGISTERED"], data_format = "vec")]
pub struct PolicyRegistered {
    #[topic]
    pub vault: Address,
    pub policy_hash: BytesN<32>,
    pub owner: Address,
    pub terms_hash: BytesN<32>,
}

/// Topics [POLICY_UPDATE_PROPOSED, vault]; data (new_hash, effective_at, terms_hash).
#[contractevent(topics = ["POLICY_UPDATE_PROPOSED"], data_format = "vec")]
pub struct PolicyUpdateProposed {
    #[topic]
    pub vault: Address,
    pub new_hash: BytesN<32>,
    pub effective_at: u64,
    pub terms_hash: BytesN<32>,
}

/// Topics [POLICY_UPDATED, vault]; data (new_hash, version, terms_hash).
#[contractevent(topics = ["POLICY_UPDATED"], data_format = "vec")]
pub struct PolicyUpdated {
    #[topic]
    pub vault: Address,
    pub new_hash: BytesN<32>,
    pub version: u32,
    pub terms_hash: BytesN<32>,
}

/// Topics [POLICY_UPDATE_CANCELLED, vault]; data the cancelled hash.
#[contractevent(topics = ["POLICY_UPDATE_CANCELLED"], data_format = "single-value")]
pub struct PolicyUpdateCancelled {
    #[topic]
    pub vault: Address,
    pub new_hash: BytesN<32>,
}

/// Topics [POLICY_NOTICE_SET, vault]; data (previous_seconds, new_seconds).
#[contractevent(topics = ["POLICY_NOTICE_SET"], data_format = "vec")]
pub struct PolicyNoticeSet {
    #[topic]
    pub vault: Address,
    pub previous_seconds: u64,
    pub new_seconds: u64,
}

/// Topics [REBALANCE_VALIDATED, vault]; data (is_valid, code).
#[contractevent(topics = ["REBALANCE_VALIDATED"], data_format = "vec")]
pub struct RebalanceValidated {
    #[topic]
    pub vault: Address,
    pub is_valid: bool,
    pub code: u32,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PolicyError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    PolicyNotFound = 4,
    DeadlineExceeded = 5,
    WeightsDoNotSumTo10000 = 6,
    WeightBandExceeded = 7,
    IssuerCapExceeded = 8,
    SingleAssetCapExceeded = 9,
    LiquidityFloorBreached = 10,
    AssetNotRegistered = 11,
    SlippageLimitExceeded = 12,
    /// A vault's policy can be registered once; afterwards only the owner may amend it.
    PolicyAlreadyRegistered = 13,
    /// A traded asset has no valid oracle price, so slippage cannot be measured.
    PriceUnavailable = 14,
    /// A trade buys an asset that is not declared (with a non-zero band) in the signed policy.
    AssetNotInPolicy = 15,
    /// The proposal was built against a different policy hash than the vault's current one.
    PolicyHashMismatch = 16,
    /// Notice period below the protocol minimum or shorter than the current one.
    NoticeTooShort = 17,
    /// No amendment is pending for this vault.
    NoPendingUpdate = 18,
    /// The pending amendment's notice period has not elapsed yet.
    NoticeNotElapsed = 19,
    /// Holdings passed to `validate_holdings` are malformed: a negative or
    /// duplicate entry, a zero total, or a value too large to compare.
    InvalidHoldings = 20,
    /// Decision D-A: at most one "numeraire" sleeve; it must be an active
    /// registry CASH asset, and it cannot change through an amendment.
    NumeraireSleeveInvalid = 21,
}

/// Protocol minimum notice before a policy amendment takes effect (audit P4,
/// owner decision D-E). Matches the 48h depositor notice used for upgrades in
/// CONTRACTS-SPEC. Owners may lengthen it per vault, never shorten it.
pub const PROTOCOL_MIN_NOTICE_SECONDS: u64 = 172_800;

/// An owner-signed amendment waiting out its notice period.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingPolicyUpdate {
    pub new_hash: BytesN<32>,
    pub min_liquidity_sleeve_bps: u32,
    pub max_single_asset_bps: u32,
    pub max_issuer_bps: u32,
    pub max_slippage_bps: u32,
    pub rebalance_band_bps: u32,
    pub sleeves: Vec<SleeveTarget>,
    pub terms_hash: BytesN<32>,
    pub proposed_at: u64,
    pub effective_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SleeveTarget {
    pub asset_id: Symbol,
    pub target_weight_bps: u32,
    pub min_band_bps: u32,
    pub max_band_bps: u32,
    pub sleeve_type: Symbol, // e.g. "cash", "usd_gov_fund", "eur_sovereign"
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyConfig {
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
    /// The policy signer (user smart account or treasury multisig). Only this
    /// address can amend the policy (audit P3).
    pub owner: Address,
    /// SHA-256 of the XDR encoding of the stored terms
    /// (registry, caps, sleeves) - lets anyone verify what was signed.
    pub terms_hash: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum TradeRoute {
    Issuer = 0,
    Soroswap = 1,
    Aquarius = 2,
    Sdex = 3,
    /// Blend v2 supply (asset_in = underlying, asset_out = pool position) or
    /// withdraw (the reverse); amounts in underlying units / bTokens.
    Blend = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeOperation {
    pub asset_in: Address,
    pub asset_out: Address,
    pub amount_in: i128,
    pub min_amount_out: i128, // Enforces maximum slippage
    pub route: TradeRoute,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebalanceProposal {
    pub vault: Address,
    /// Hash of the policy the proposer built against; must equal the vault's
    /// current registered hash (CONTRACTS-SPEC check 1, audit M1).
    pub policy_hash: BytesN<32>,
    pub operations: Vec<TradeOperation>,
    pub weights_after: Vec<(Symbol, u32)>, // (asset_id, weight_bps)
    pub deadline: u64,
    pub proposer: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub code: u32, // 0 = Valid, 1..=12 = Specific failure reason
    pub message: Symbol,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Oracle,
    PolicyConfig(Address),
    SleeveTargets(Address),
    PendingUpdate(Address),
    NoticePeriod(Address),
}

#[cfg(test)]
mod test;

pub trait PolicyContractTrait {
    /// The NAV oracle used for slippage valuation.
    fn get_oracle(env: Env) -> Address;

    /// Register the signed policy for a vault. Requires authorization from BOTH
    /// the policy `owner` (who signs the hash and every term) and the `vault`
    /// itself. One-time: re-registration fails with `PolicyAlreadyRegistered`.
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
        sleeves: Vec<SleeveTarget>,
    );

    /// Propose an amendment. Requires a fresh signature from the stored policy
    /// owner (audit P3). The current terms stay in force until the notice
    /// period has elapsed and `activate_policy_update` is called (audit P4).
    /// A new proposal replaces any pending one and restarts the clock.
    fn update_policy(
        env: Env,
        vault: Address,
        new_hash: BytesN<32>,
        min_liquidity_sleeve_bps: u32,
        max_single_asset_bps: u32,
        max_issuer_bps: u32,
        max_slippage_bps: u32,
        rebalance_band_bps: u32,
        new_sleeves: Vec<SleeveTarget>,
    );

    /// Apply a pending amendment once its notice period has elapsed.
    /// Permissionless: anyone (keeper, UI, holder) may finalize it.
    fn activate_policy_update(env: Env, vault: Address);

    /// Withdraw a pending amendment. Policy owner only.
    fn cancel_policy_update(env: Env, vault: Address);

    /// The amendment waiting out its notice period, if any.
    fn get_pending_policy_update(env: Env, vault: Address) -> Option<PendingPolicyUpdate>;

    /// Lengthen this vault's amendment notice period. Owner only; can never be
    /// shortened and never below PROTOCOL_MIN_NOTICE_SECONDS.
    fn set_notice_period(env: Env, vault: Address, seconds: u64);

    /// Current amendment notice period for this vault.
    fn get_notice_period(env: Env, vault: Address) -> u64;

    /// Get registered policy hash for a vault
    fn get_policy_hash(env: Env, vault: Address) -> BytesN<32>;

    /// Get policy configuration parameters
    fn get_policy_config(env: Env, vault: Address) -> Option<PolicyConfig>;

    /// Get sleeve targets and bands for a vault
    fn get_sleeve_targets(env: Env, vault: Address) -> Vec<SleeveTarget>;

    /// Validate a proposed rebalance. Returns ValidationResult(is_valid, code, message).
    fn validate_rebalance(
        env: Env,
        vault: Address,
        proposal: RebalanceProposal,
    ) -> ValidationResult;

    /// Audit W1b: check what a vault ACTUALLY holds against its signed policy.
    /// `holdings` is (registry asset_id, value in numéraire) per sleeve. The
    /// weights are compared exactly (value x 10000 against band x total, no
    /// rounding) with every declared band, the single-asset cap, the issuer
    /// caps (issuer from the registry) and the cash liquidity floor. A held
    /// asset that the policy does not declare is rejected. Read-only.
    fn validate_holdings(env: Env, vault: Address, holdings: Vec<(Symbol, i128)>) -> ValidationResult;

    /// Get admin address
    fn get_admin(env: Env) -> Address;
}


/// A1 (storage rent): 30 days, renewed whenever an entry is used in a
/// transaction (reads in simulations do not persist and renew nothing; the
/// keeper's extend job covers idle entries).
const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 29 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 30 * DAY_LEDGERS;

/// Renew the instance and this vault's records. Per-vault records are their
/// own persistent entries: the instance is loaded on every call and has a
/// size cap, so it must not grow with each factory vault.
fn renew(env: &Env, vault: &Address) {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
    for key in [DataKey::PolicyConfig(vault.clone()), DataKey::SleeveTargets(vault.clone()),
                DataKey::PendingUpdate(vault.clone()), DataKey::NoticePeriod(vault.clone())] {
        if env.storage().persistent().has(&key) {
            env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
        }
    }
}

/// Decision D-L: true only if every term is at least as safe as today AND
/// something actually changes toward safety. Conservative by construction:
/// anything not listed as safer (band changes, new assets, changed types)
/// takes the full notice.
#[allow(clippy::too_many_arguments)]
fn is_risk_reducing(env: &Env, cur: &PolicyConfig, cur_sleeves: &Vec<SleeveTarget>, floor: u32, single: u32, issuer: u32,
                    slip: u32, band: u32, new_sleeves: &Vec<SleeveTarget>) -> bool {
    if floor < cur.min_liquidity_sleeve_bps || single > cur.max_single_asset_bps || issuer > cur.max_issuer_bps
        || slip > cur.max_slippage_bps || band != cur.rebalance_band_bps {
        return false;
    }
    let mut changed = floor != cur.min_liquidity_sleeve_bps || single != cur.max_single_asset_bps
        || issuer != cur.max_issuer_bps || slip != cur.max_slippage_bps;
    for n in new_sleeves.iter() {
        let Some(o) = cur_sleeves.iter().find(|o| o.asset_id == n.asset_id) else { return false }; // new asset
        if o.sleeve_type != n.sleeve_type {
            return false;
        }
        let ok = if is_numeraire_sleeve(env, &n.sleeve_type) {
            // More vault currency is safer: nothing may go down.
            n.target_weight_bps >= o.target_weight_bps && n.min_band_bps >= o.min_band_bps && n.max_band_bps >= o.max_band_bps
        } else {
            // Less of a risk asset is safer: nothing may go up.
            n.target_weight_bps <= o.target_weight_bps && n.min_band_bps <= o.min_band_bps && n.max_band_bps <= o.max_band_bps
        };
        if !ok {
            return false;
        }
        changed |= n != o;
    }
    // Removing a (non-numeraire) asset reduces risk; the numeraire cannot be
    // removed (checked before this). Remaining risk-asset removals count as change.
    changed |= cur_sleeves.len() != new_sleeves.len();
    changed
}

#[contract]
pub struct PolicyContract;

fn is_cash_sleeve(env: &Env, st: &Symbol) -> bool {
    let sym_str: SymbolStr = SymbolStr::from_val(env, &st.to_symbol_val());
    let bytes: &[u8] = sym_str.as_ref();
    bytes.eq_ignore_ascii_case(b"cash")
        || bytes.eq_ignore_ascii_case(b"cash_stable")
        || bytes.eq_ignore_ascii_case(b"cash_sleeve")
        || is_numeraire_sleeve(env, st)
}

/// Decision D-A: the vault's own currency is declared in the signed sleeves
/// with sleeve_type "numeraire". It counts as cash for the liquidity floor
/// and is EXEMPT from the single-asset and issuer caps (a USDC vault is
/// inherently USDC-exposed; its cash is governed by the floor instead).
///
/// Exact, case-sensitive match on purpose: the vault's binding check
/// compares the same exact symbol, so no other spelling can carry the
/// exemption past it.
fn is_numeraire_sleeve(env: &Env, st: &Symbol) -> bool {
    *st == Symbol::new(env, "numeraire")
}

/// The asset_id of the (single) numéraire sleeve, if declared.
fn numeraire_of(env: &Env, sleeves: &Vec<SleeveTarget>) -> Option<Symbol> {
    sleeves.iter().find(|t| is_numeraire_sleeve(env, &t.sleeve_type)).map(|t| t.asset_id)
}

fn verdict(env: &Env, err: Option<(PolicyError, &str)>) -> ValidationResult {
    match err {
        None => ValidationResult { is_valid: true, code: 0, message: Symbol::new(env, "VALID") },
        Some((e, msg)) => ValidationResult { is_valid: false, code: e as u32, message: Symbol::new(env, msg) },
    }
}

/// `value / total` compared with `bps / 10000` exactly: returns
/// value*10000 - bps*total sign, or None on overflow.
fn cmp_weight(value: i128, total: i128, bps: u32) -> Option<core::cmp::Ordering> {
    let lhs = value.checked_mul(10_000)?;
    let rhs = total.checked_mul(bps as i128)?;
    Some(lhs.cmp(&rhs))
}

/// Audit W1b: the checks behind `validate_holdings`.
fn check_holdings(env: &Env, vault: &Address, holdings: &Vec<(Symbol, i128)>) -> Option<(PolicyError, &'static str)> {
    use core::cmp::Ordering::{Greater, Less};
    let config: PolicyConfig = match env.storage().persistent().get(&DataKey::PolicyConfig(vault.clone())) {
        Some(c) => c,
        None => return Some((PolicyError::PolicyNotFound, "POLICY_NOT_FOUND")),
    };
    if !config.is_active {
        return Some((PolicyError::PolicyNotFound, "POLICY_INACTIVE"));
    }
    let targets: Vec<SleeveTarget> = env
        .storage()
        .persistent()
        .get(&DataKey::SleeveTargets(vault.clone()))
        .unwrap_or_else(|| Vec::new(env));

    let mut total = 0i128;
    for i in 0..holdings.len() {
        let (id, value) = holdings.get_unchecked(i);
        if value < 0 {
            return Some((PolicyError::InvalidHoldings, "NEGATIVE_HOLDING"));
        }
        for j in (i + 1)..holdings.len() {
            if holdings.get_unchecked(j).0 == id {
                return Some((PolicyError::InvalidHoldings, "DUPLICATE_HOLDING"));
            }
        }
        total = match total.checked_add(value) {
            Some(t) => t,
            None => return Some((PolicyError::InvalidHoldings, "HOLDINGS_OVERFLOW")),
        };
    }
    if total == 0 {
        return Some((PolicyError::InvalidHoldings, "EMPTY_HOLDINGS"));
    }
    let value_of = |id: &Symbol| -> i128 {
        for (h, v) in holdings.iter() {
            if h == *id {
                return v;
            }
        }
        0
    };

    // Anything held must be declared by the signed policy.
    for (id, value) in holdings.iter() {
        if value > 0 && !targets.iter().any(|t| t.asset_id == id) {
            return Some((PolicyError::AssetNotInPolicy, "UNDECLARED_HOLDING"));
        }
    }
    // Every declared band, on actual values.
    let mut cash = 0i128;
    for t in targets.iter() {
        let v = value_of(&t.asset_id);
        match (cmp_weight(v, total, t.min_band_bps), cmp_weight(v, total, t.max_band_bps)) {
            (Some(lo), Some(hi)) => {
                if lo == Less || hi == Greater {
                    return Some((PolicyError::WeightBandExceeded, "ACTUAL_OUT_OF_BAND"));
                }
            }
            _ => return Some((PolicyError::InvalidHoldings, "HOLDINGS_OVERFLOW")),
        }
        if is_cash_sleeve(env, &t.sleeve_type) {
            cash = cash.saturating_add(v);
        }
    }
    // Single-asset cap and issuer caps (issuer recorded in the registry);
    // the numéraire is exempt from both (D-A) but must still be registered.
    let registry = AssetRegistryClient::new(env, &config.registry_address);
    let numeraire = numeraire_of(env, &targets);
    let mut issuers: Vec<(Symbol, i128)> = Vec::new(env);
    for (id, value) in holdings.iter() {
        if value == 0 {
            continue;
        }
        if numeraire.as_ref() == Some(&id) {
            match registry.get_asset(&id) {
                Some(e) if e.is_active => continue,
                _ => return Some((PolicyError::AssetNotRegistered, "ASSET_NOT_REGISTERED")),
            }
        }
        match cmp_weight(value, total, config.max_single_asset_bps) {
            Some(Greater) => return Some((PolicyError::SingleAssetCapExceeded, "ACTUAL_SINGLE_ASSET_CAP")),
            None => return Some((PolicyError::InvalidHoldings, "HOLDINGS_OVERFLOW")),
            _ => {}
        }
        let entry = match registry.get_asset(&id) {
            Some(e) if e.is_active => e,
            _ => return Some((PolicyError::AssetNotRegistered, "ASSET_NOT_REGISTERED")),
        };
        let mut found = false;
        for k in 0..issuers.len() {
            let (issuer, sum) = issuers.get_unchecked(k);
            if issuer == entry.issuer {
                issuers.set(k, (issuer, sum.saturating_add(value)));
                found = true;
                break;
            }
        }
        if !found {
            issuers.push_back((entry.issuer, value));
        }
    }
    for (_, sum) in issuers.iter() {
        match cmp_weight(sum, total, config.max_issuer_bps) {
            Some(Greater) => return Some((PolicyError::IssuerCapExceeded, "ACTUAL_ISSUER_CAP")),
            None => return Some((PolicyError::InvalidHoldings, "HOLDINGS_OVERFLOW")),
            _ => {}
        }
    }
    // Cash liquidity floor.
    match cmp_weight(cash, total, config.min_liquidity_sleeve_bps) {
        Some(Less) => Some((PolicyError::LiquidityFloorBreached, "ACTUAL_LIQUIDITY_FLOOR")),
        None => Some((PolicyError::InvalidHoldings, "HOLDINGS_OVERFLOW")),
        _ => None,
    }
}

fn validate_terms(
    env: &Env,
    registry: &Address,
    min_liquidity_sleeve_bps: u32,
    max_single_asset_bps: u32,
    max_issuer_bps: u32,
    max_slippage_bps: u32,
    sleeves: &Vec<SleeveTarget>,
) {
    if max_slippage_bps > 10000 {
        panic_with_error!(env, PolicyError::SlippageLimitExceeded);
    }
    if min_liquidity_sleeve_bps > 10000 {
        panic_with_error!(env, PolicyError::LiquidityFloorBreached);
    }
    if max_single_asset_bps > 10000 {
        panic_with_error!(env, PolicyError::SingleAssetCapExceeded);
    }
    if max_issuer_bps > 10000 {
        panic_with_error!(env, PolicyError::IssuerCapExceeded);
    }
    for i in 0..sleeves.len() {
        let sleeve_i = sleeves.get(i).unwrap();
        if sleeve_i.min_band_bps > sleeve_i.max_band_bps {
            panic_with_error!(env, PolicyError::WeightBandExceeded);
        }
        for j in (i + 1)..sleeves.len() {
            if sleeve_i.asset_id == sleeves.get(j).unwrap().asset_id {
                panic_with_error!(env, PolicyError::WeightBandExceeded);
            }
        }
    }
    // D-A: at most one numéraire sleeve, an active registry CASH asset.
    let declared = sleeves.iter().filter(|t| is_numeraire_sleeve(env, &t.sleeve_type)).count();
    if declared > 1 {
        panic_with_error!(env, PolicyError::NumeraireSleeveInvalid);
    }
    if let Some(id) = numeraire_of(env, sleeves) {
        match AssetRegistryClient::new(env, registry).get_asset(&id) {
            Some(e) if e.is_active && e.sleeve_type == RegistrySleeveType::Cash => {}
            _ => panic_with_error!(env, PolicyError::NumeraireSleeveInvalid),
        }
    }
}

/// SHA-256 over the XDR of every enforced term, in a fixed order.
pub fn compute_terms_hash(
    env: &Env,
    registry_address: &Address,
    min_liquidity_sleeve_bps: u32,
    max_single_asset_bps: u32,
    max_issuer_bps: u32,
    max_slippage_bps: u32,
    rebalance_band_bps: u32,
    sleeves: &Vec<SleeveTarget>,
) -> BytesN<32> {
    let terms = (
        registry_address.clone(),
        min_liquidity_sleeve_bps,
        max_single_asset_bps,
        max_issuer_bps,
        max_slippage_bps,
        rebalance_band_bps,
        sleeves.clone(),
    );
    env.crypto().sha256(&terms.to_xdr(env)).to_bytes()
}

#[contractimpl]
impl PolicyContract {
    /// Set the governance admin and the NAV oracle used to value both legs of
    /// every trade (audit P1). Runs atomically as part of deployment, so no one
    /// can claim an uninitialized policy contract (audit M3). The oracle is
    /// fixed for the life of this contract: there is deliberately no setter.
    pub fn __constructor(env: Env, admin: Address, oracle: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
    }
}

#[contractimpl]
impl PolicyContractTrait for PolicyContract {
    fn get_oracle(env: Env) -> Address {
        match env.storage().instance().get(&DataKey::Oracle) {
            Some(o) => o,
            None => panic_with_error!(&env, PolicyError::NotInitialized),
        }
    }

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
        sleeves: Vec<SleeveTarget>,
    ) {
        // Fail fast if the contract was never initialized.
        let _ = Self::get_admin(env.clone());
        // The vault must consent to being governed by this policy, and the
        // owner signs the hash together with every enforced term (audit P3).
        vault.require_auth();
        owner.require_auth();

        if env
            .storage()
            .persistent()
            .has(&DataKey::PolicyConfig(vault.clone()))
        {
            panic_with_error!(&env, PolicyError::PolicyAlreadyRegistered);
        }

        validate_terms(
            &env,
            &registry_address,
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            &sleeves,
        );

        let terms_hash = compute_terms_hash(
            &env,
            &registry_address,
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            rebalance_band_bps,
            &sleeves,
        );

        let config = PolicyConfig {
            vault: vault.clone(),
            policy_hash: policy_hash.clone(),
            version: 1,
            registry_address,
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            rebalance_band_bps,
            is_active: true,
            owner: owner.clone(),
            terms_hash: terms_hash.clone(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::PolicyConfig(vault.clone()), &config);
        env.storage()
            .persistent()
            .set(&DataKey::SleeveTargets(vault.clone()), &sleeves);
        renew(&env, &vault);

        PolicyRegistered { vault, policy_hash, owner, terms_hash }.publish(&env);
    }

    fn update_policy(
        env: Env,
        vault: Address,
        new_hash: BytesN<32>,
        min_liquidity_sleeve_bps: u32,
        max_single_asset_bps: u32,
        max_issuer_bps: u32,
        max_slippage_bps: u32,
        rebalance_band_bps: u32,
        new_sleeves: Vec<SleeveTarget>,
    ) {
        renew(&env, &vault);
        let config: PolicyConfig = match env
            .storage()
            .persistent()
            .get(&DataKey::PolicyConfig(vault.clone()))
        {
            Some(cfg) => cfg,
            None => panic_with_error!(&env, PolicyError::PolicyNotFound),
        };
        // Only the policy owner can amend — never the protocol admin (audit P3).
        config.owner.require_auth();

        validate_terms(
            &env,
            &config.registry_address,
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            &new_sleeves,
        );
        // D-A: the numéraire is fixed at binding (checked by the vault, which
        // is not consulted on amendments), so an amendment cannot add,
        // remove or change it.
        let current: Vec<SleeveTarget> = env.storage().persistent().get(&DataKey::SleeveTargets(vault.clone())).unwrap_or_else(|| Vec::new(&env));
        if numeraire_of(&env, &new_sleeves) != numeraire_of(&env, &current) {
            panic_with_error!(&env, PolicyError::NumeraireSleeveInvalid);
        }
        let terms_hash = compute_terms_hash(
            &env,
            &config.registry_address,
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            rebalance_band_bps,
            &new_sleeves,
        );

        // Decision D-L: a strictly risk-reducing amendment applies at once (a
        // crisis needs no shortcut around the notice) and cancels any pending
        // amendment, so an older, looser one can never land on top of it.
        if is_risk_reducing(&env, &config, &current, min_liquidity_sleeve_bps, max_single_asset_bps, max_issuer_bps,
                            max_slippage_bps, rebalance_band_bps, &new_sleeves) {
            env.storage().persistent().remove(&DataKey::PendingUpdate(vault.clone()));
            let mut config = config;
            config.policy_hash = new_hash.clone();
            config.version += 1;
            config.min_liquidity_sleeve_bps = min_liquidity_sleeve_bps;
            config.max_single_asset_bps = max_single_asset_bps;
            config.max_issuer_bps = max_issuer_bps;
            config.max_slippage_bps = max_slippage_bps;
            config.terms_hash = terms_hash.clone();
            env.storage().persistent().set(&DataKey::PolicyConfig(vault.clone()), &config);
            env.storage().persistent().set(&DataKey::SleeveTargets(vault.clone()), &new_sleeves);
            renew(&env, &vault);
            PolicyUpdated { vault, new_hash, version: config.version, terms_hash }.publish(&env);
            return;
        }

        // Audit P4: schedule, do not apply. Holders keep the current terms for
        // the full notice period and can exit before the change lands.
        let now = env.ledger().timestamp();
        let effective_at = match now.checked_add(Self::get_notice_period(env.clone(), vault.clone())) {
            Some(t) => t,
            None => panic_with_error!(&env, PolicyError::NoticeTooShort),
        };
        let pending = PendingPolicyUpdate {
            new_hash: new_hash.clone(),
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            rebalance_band_bps,
            sleeves: new_sleeves,
            terms_hash: terms_hash.clone(),
            proposed_at: now,
            effective_at,
        };
        env.storage()
            .persistent()
            .set(&DataKey::PendingUpdate(vault.clone()), &pending);

        PolicyUpdateProposed { vault, new_hash, effective_at, terms_hash }.publish(&env);
    }

    fn activate_policy_update(env: Env, vault: Address) {
        renew(&env, &vault);
        let pending: PendingPolicyUpdate = match env
            .storage()
            .persistent()
            .get(&DataKey::PendingUpdate(vault.clone()))
        {
            Some(p) => p,
            None => panic_with_error!(&env, PolicyError::NoPendingUpdate),
        };
        if env.ledger().timestamp() < pending.effective_at {
            panic_with_error!(&env, PolicyError::NoticeNotElapsed);
        }
        let mut config: PolicyConfig = match env
            .storage()
            .persistent()
            .get(&DataKey::PolicyConfig(vault.clone()))
        {
            Some(cfg) => cfg,
            None => panic_with_error!(&env, PolicyError::PolicyNotFound),
        };

        config.policy_hash = pending.new_hash.clone();
        config.version += 1;
        config.min_liquidity_sleeve_bps = pending.min_liquidity_sleeve_bps;
        config.max_single_asset_bps = pending.max_single_asset_bps;
        config.max_issuer_bps = pending.max_issuer_bps;
        config.max_slippage_bps = pending.max_slippage_bps;
        config.rebalance_band_bps = pending.rebalance_band_bps;
        config.terms_hash = pending.terms_hash.clone();

        env.storage()
            .persistent()
            .set(&DataKey::PolicyConfig(vault.clone()), &config);
        env.storage()
            .persistent()
            .set(&DataKey::SleeveTargets(vault.clone()), &pending.sleeves);
        env.storage()
            .persistent()
            .remove(&DataKey::PendingUpdate(vault.clone()));

        PolicyUpdated {
            vault,
            new_hash: pending.new_hash,
            version: config.version,
            terms_hash: config.terms_hash,
        }
        .publish(&env);
    }

    fn cancel_policy_update(env: Env, vault: Address) {
        renew(&env, &vault);
        let config: PolicyConfig = match env
            .storage()
            .persistent()
            .get(&DataKey::PolicyConfig(vault.clone()))
        {
            Some(cfg) => cfg,
            None => panic_with_error!(&env, PolicyError::PolicyNotFound),
        };
        config.owner.require_auth();
        let pending: PendingPolicyUpdate = match env
            .storage()
            .persistent()
            .get(&DataKey::PendingUpdate(vault.clone()))
        {
            Some(p) => p,
            None => panic_with_error!(&env, PolicyError::NoPendingUpdate),
        };
        env.storage()
            .persistent()
            .remove(&DataKey::PendingUpdate(vault.clone()));
        PolicyUpdateCancelled { vault, new_hash: pending.new_hash }.publish(&env);
    }

    fn get_pending_policy_update(env: Env, vault: Address) -> Option<PendingPolicyUpdate> {
        renew(&env, &vault);
        env.storage().persistent().get(&DataKey::PendingUpdate(vault))
    }

    fn set_notice_period(env: Env, vault: Address, seconds: u64) {
        renew(&env, &vault);
        let config: PolicyConfig = match env
            .storage()
            .persistent()
            .get(&DataKey::PolicyConfig(vault.clone()))
        {
            Some(cfg) => cfg,
            None => panic_with_error!(&env, PolicyError::PolicyNotFound),
        };
        config.owner.require_auth();
        let current = Self::get_notice_period(env.clone(), vault.clone());
        if seconds < current {
            panic_with_error!(&env, PolicyError::NoticeTooShort);
        }
        env.storage()
            .persistent()
            .set(&DataKey::NoticePeriod(vault.clone()), &seconds);
        PolicyNoticeSet { vault, previous_seconds: current, new_seconds: seconds }.publish(&env);
    }

    fn get_notice_period(env: Env, vault: Address) -> u64 {
        renew(&env, &vault);
        let stored: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::NoticePeriod(vault))
            .unwrap_or(PROTOCOL_MIN_NOTICE_SECONDS);
        if stored < PROTOCOL_MIN_NOTICE_SECONDS {
            PROTOCOL_MIN_NOTICE_SECONDS
        } else {
            stored
        }
    }

    fn get_policy_hash(env: Env, vault: Address) -> BytesN<32> {
        renew(&env, &vault);
        match env
            .storage()
            .persistent()
            .get::<_, PolicyConfig>(&DataKey::PolicyConfig(vault))
        {
            Some(config) => config.policy_hash,
            None => panic_with_error!(&env, PolicyError::PolicyNotFound),
        }
    }

    fn get_policy_config(env: Env, vault: Address) -> Option<PolicyConfig> {
        renew(&env, &vault);
        env.storage().persistent().get(&DataKey::PolicyConfig(vault))
    }

    fn get_sleeve_targets(env: Env, vault: Address) -> Vec<SleeveTarget> {
        renew(&env, &vault);
        env.storage()
            .persistent()
            .get(&DataKey::SleeveTargets(vault))
            .unwrap_or_else(|| Vec::new(&env))
    }

    fn validate_rebalance(
        env: Env,
        vault: Address,
        proposal: RebalanceProposal,
    ) -> ValidationResult {
        renew(&env, &vault);
        let config: PolicyConfig = match env
            .storage()
            .persistent()
            .get(&DataKey::PolicyConfig(vault.clone()))
        {
            Some(c) => c,
            None => {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::PolicyNotFound as u32,
                    message: Symbol::new(&env, "POLICY_NOT_FOUND"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
        };

        if !config.is_active {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::PolicyNotFound as u32,
                message: Symbol::new(&env, "POLICY_INACTIVE"),
            };
            RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
            return res;
        }

        // Verify proposal is destined for the queried vault
        if proposal.vault != vault {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::Unauthorized as u32,
                message: Symbol::new(&env, "VAULT_MISMATCH"),
            };
            RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
            return res;
        }

        // 0. The proposal must target the vault's CURRENT signed policy (audit M1).
        // After the owner amends, proposals built for the old policy are void.
        if proposal.policy_hash != config.policy_hash {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::PolicyHashMismatch as u32,
                message: Symbol::new(&env, "POLICY_HASH_MISMATCH"),
            };
            RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
            return res;
        }

        // 1. Verify proposal deadline >= env.ledger().timestamp()
        if proposal.deadline < env.ledger().timestamp() {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::DeadlineExceeded as u32,
                message: Symbol::new(&env, "DEADLINE_EXCEEDED"),
            };
            RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
            return res;
        }

        // 2. Verify sum of proposal.weights_after == 10000 bps
        let mut sum_weights: u32 = 0;
        for i in 0..proposal.weights_after.len() {
            let (_, weight) = proposal.weights_after.get(i).unwrap();
            sum_weights = sum_weights.saturating_add(weight);
        }
        if sum_weights != 10000 {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::WeightsDoNotSumTo10000 as u32,
                message: Symbol::new(&env, "WEIGHTS_NOT_10000"),
            };
            RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
            return res;
        }

        // 2b. Reject duplicate assets in weights_after to prevent single-asset and band bypass
        for i in 0..proposal.weights_after.len() {
            let (asset_i, _) = proposal.weights_after.get(i).unwrap();
            for j in (i + 1)..proposal.weights_after.len() {
                let (asset_j, _) = proposal.weights_after.get(j).unwrap();
                if asset_i == asset_j {
                    let res = ValidationResult {
                        is_valid: false,
                        code: PolicyError::WeightBandExceeded as u32,
                        message: Symbol::new(&env, "DUPLICATE_ASSET"),
                    };
                    RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                    return res;
                }
            }
        }

        // 3. Verify each proposed weight is within [min_band_bps, max_band_bps] for declared sleeves
        let sleeves: Vec<SleeveTarget> = env
            .storage()
            .persistent()
            .get(&DataKey::SleeveTargets(vault.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        for i in 0..sleeves.len() {
            let sleeve = sleeves.get(i).unwrap();
            let mut proposed_weight = 0u32;
            for j in 0..proposal.weights_after.len() {
                let (asset_id, weight) = proposal.weights_after.get(j).unwrap();
                if asset_id == sleeve.asset_id {
                    proposed_weight = weight;
                    break;
                }
            }
            if proposed_weight < sleeve.min_band_bps || proposed_weight > sleeve.max_band_bps {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::WeightBandExceeded as u32,
                    message: Symbol::new(&env, "WEIGHT_BAND_EXCEEDED"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
        }

        for j in 0..proposal.weights_after.len() {
            let (asset_id, _) = proposal.weights_after.get(j).unwrap();
            let mut declared = false;
            for i in 0..sleeves.len() {
                let sleeve = sleeves.get(i).unwrap();
                if sleeve.asset_id == asset_id {
                    declared = true;
                    break;
                }
            }
            if !declared {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::WeightBandExceeded as u32,
                    message: Symbol::new(&env, "UNDECLARED_SLEEVE"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
        }

        // 4. Verify no single asset weight > max_single_asset_bps (the
        // numéraire is exempt, decision D-A)
        let numeraire = numeraire_of(&env, &sleeves);
        for j in 0..proposal.weights_after.len() {
            let (asset_id, weight) = proposal.weights_after.get(j).unwrap();
            if numeraire.as_ref() == Some(&asset_id) {
                continue;
            }
            if weight > config.max_single_asset_bps {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::SingleAssetCapExceeded as u32,
                    message: Symbol::new(&env, "SINGLE_ASSET_CAP"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
        }

        // 5. Closed universe + issuer caps (audit P2). Every asset carrying
        // weight must be an active registry entry, and issuer concentration is
        // summed by the issuer recorded in the registry (e.g. USDC and EURC
        // both count toward CIRCLE) - never inferred from the asset_id text.
        let registry_client = AssetRegistryClient::new(&env, &config.registry_address);
        let mut issuer_weights: Vec<(Symbol, u32)> = Vec::new(&env);
        for j in 0..proposal.weights_after.len() {
            let (asset_id, weight) = proposal.weights_after.get(j).unwrap();
            if weight == 0 {
                continue;
            }
            let entry = match registry_client.get_asset(&asset_id) {
                Some(entry) if entry.is_active => entry,
                _ => {
                    let res = ValidationResult {
                        is_valid: false,
                        code: PolicyError::AssetNotRegistered as u32,
                        message: Symbol::new(&env, "ASSET_NOT_REGISTERED"),
                    };
                    RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                    return res;
                }
            };
            if numeraire.as_ref() == Some(&asset_id) {
                continue; // registered and active (checked above); exempt from the issuer cap (D-A)
            }
            let mut found = false;
            for k in 0..issuer_weights.len() {
                let (issuer, total) = issuer_weights.get(k).unwrap();
                if issuer == entry.issuer {
                    issuer_weights.set(k, (issuer, total.saturating_add(weight)));
                    found = true;
                    break;
                }
            }
            if !found {
                issuer_weights.push_back((entry.issuer, weight));
            }
        }
        for (_, total) in issuer_weights.iter() {
            if total > config.max_issuer_bps {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::IssuerCapExceeded as u32,
                    message: Symbol::new(&env, "ISSUER_CAP_EXCEEDED"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
        }

        // 6. Verify cash liquidity sleeve >= min_liquidity_sleeve_bps
        let mut total_cash_weight = 0u32;
        for i in 0..sleeves.len() {
            let sleeve = sleeves.get(i).unwrap();
            if is_cash_sleeve(&env, &sleeve.sleeve_type) {
                for j in 0..proposal.weights_after.len() {
                    let (asset_id, weight) = proposal.weights_after.get(j).unwrap();
                    if asset_id == sleeve.asset_id {
                        total_cash_weight = total_cash_weight.saturating_add(weight);
                        break;
                    }
                }
            }
        }
        if total_cash_weight < config.min_liquidity_sleeve_bps {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::LiquidityFloorBreached as u32,
                message: Symbol::new(&env, "LIQUIDITY_FLOOR_BREACHED"),
            };
            RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
            return res;
        }

        // 7. Structural checks on every trade operation
        for i in 0..proposal.operations.len() {
            let op = proposal.operations.get(i).unwrap();
            if op.amount_in <= 0 || op.min_amount_out <= 0 {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::SlippageLimitExceeded as u32,
                    message: Symbol::new(&env, "INVALID_TRADE_AMOUNT"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
            if op.asset_in == op.asset_out {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::SlippageLimitExceeded as u32,
                    message: Symbol::new(&env, "IDENTICAL_ASSETS"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }

        }

        // 8. (weighted assets were verified against the registry in step 5)
        // 8b. Verify all traded assets in operations are registered and active
        for i in 0..proposal.operations.len() {
            let op = proposal.operations.get(i).unwrap();
            if !registry_client.is_registered(&op.asset_in)
                || !registry_client.is_registered(&op.asset_out)
            {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::AssetNotRegistered as u32,
                    message: Symbol::new(&env, "ASSET_NOT_REGISTERED"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
        }

        // 8c. Exclusions (audit M1, EXECUTION.md "no excluded asset is bought"):
        // the signed sleeve list is the allow-list. Every asset bought must be
        // declared there with a non-zero upper band. Selling is not restricted.
        for i in 0..proposal.operations.len() {
            let op = proposal.operations.get(i).unwrap();
            let bought = registry_client.get_asset_by_address(&op.asset_out);
            let allowed = match bought {
                Some(entry) => {
                    let mut ok = false;
                    for k in 0..sleeves.len() {
                        let sleeve = sleeves.get(k).unwrap();
                        if sleeve.asset_id == entry.asset_id && sleeve.max_band_bps > 0 {
                            ok = true;
                            break;
                        }
                    }
                    ok
                }
                None => false,
            };
            if !allowed {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::AssetNotInPolicy as u32,
                    message: Symbol::new(&env, "ASSET_NOT_IN_POLICY"),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
        }

        // 9. Slippage measured in VALUE (audit P1). Both legs are priced by the
        // NAV oracle (median of each asset's valid sources, numéraire = 1.0) and
        // normalised by each token's decimals:
        //   value = amount * price / 10^token_decimals
        //   require  min_value_out >= value_in * (1 - max_slippage)
        let oracle = OraclePriceClient::new(&env, &Self::get_oracle(env.clone()));
        let keep_bps = 10000i128 - config.max_slippage_bps as i128;
        for i in 0..proposal.operations.len() {
            let op = proposal.operations.get(i).unwrap();
            let (p_in, p_out) = match (oracle.asset_price(&op.asset_in), oracle.asset_price(&op.asset_out)) {
                (Some(a), Some(b)) if a.price > 0 && b.price > 0 => (a.price, b.price),
                _ => {
                    let res = ValidationResult {
                        is_valid: false,
                        code: PolicyError::PriceUnavailable as u32,
                        message: Symbol::new(&env, "PRICE_UNAVAILABLE"),
                    };
                    RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                    return res;
                }
            };
            let dec_in = leg_decimals(&env, &oracle, &op.asset_in);
            let dec_out = leg_decimals(&env, &oracle, &op.asset_out);
            let checked = (|| -> Option<bool> {
                let value_in = op.amount_in.checked_mul(p_in)? / 10i128.checked_pow(dec_in)?;
                let min_value_out = op.min_amount_out.checked_mul(p_out)? / 10i128.checked_pow(dec_out)?;
                Some(min_value_out.checked_mul(10000)? >= value_in.checked_mul(keep_bps)?)
            })();
            let (ok, msg) = match checked {
                Some(true) => (true, ""),
                Some(false) => (false, "SLIPPAGE_EXCEEDED"),
                None => (false, "SLIPPAGE_OVERFLOW"),
            };
            if !ok {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::SlippageLimitExceeded as u32,
                    message: Symbol::new(&env, msg),
                };
                RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
                return res;
            }
        }

        // All checks passed!
        let res = ValidationResult {
            is_valid: true,
            code: 0,
            message: Symbol::new(&env, "VALID"),
        };
        RebalanceValidated { vault, is_valid: res.is_valid, code: res.code }.publish(&env);
        res
    }

    fn validate_holdings(env: Env, vault: Address, holdings: Vec<(Symbol, i128)>) -> ValidationResult {
        renew(&env, &vault);
        verdict(&env, check_holdings(&env, &vault, &holdings))
    }

    fn get_admin(env: Env) -> Address {
        match env.storage().instance().get(&DataKey::Admin) {
            Some(admin) => admin,
            None => panic_with_error!(&env, PolicyError::NotInitialized),
        }
    }
}
