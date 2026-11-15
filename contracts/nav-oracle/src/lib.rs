#![no_std]
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address, Env, Symbol,
    Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    AssetNotFound = 4,
    SleeveNotFound = 5,
    SleevePaused = 6,
    InsufficientSources = 7,
    DeviationExceeded = 8,
    PriceStale = 9,
    InsufficientLiquidityDepth = 10,
    InvalidThreshold = 11,
    /// Price timestamp is later than the current ledger time.
    FutureTimestamp = 12,
    /// Update is older than the price already stored for this source.
    OutOfOrderUpdate = 13,
    /// An asset's sources must all belong to one sleeve, and a source id
    /// cannot be re-registered for another asset or sleeve (audit O6).
    AssetSleeveMismatch = 14,
    /// No source change is pending for this id.
    NoPendingSourceChange = 15,
    /// The 48-hour source-change delay has not elapsed.
    SourceChangeNotReady = 16,
}

/// SEP-40 asset identifier (identical to script3/sep-40-oracle and Reflector).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Asset {
    /// Stellar asset contract (SAC) or Soroban token address.
    Stellar(Address),
    /// Any other asset; this oracle quotes none (always `None`).
    Other(Symbol),
}

/// SEP-40 price record: exactly `{price, timestamp}` (audit W5). Older SDKs
/// decode contracttype structs strictly, so no field may ever be added.
/// `price` has `decimals()` decimals; `timestamp` is in seconds, on the
/// `resolution()` grid.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}

/// Constella's detailed price: the SEP-40 fields plus the source it came from.
/// Used by the vault / policy views (same wire layout as before W5).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcedPrice {
    pub price: i128,
    pub timestamp: u64,
    pub source: Symbol,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
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
    /// The only key allowed to push this source's price (audit O7a).
    pub feeder: Address,
}

/// A source change waiting out `SOURCE_CHANGE_DELAY_SECONDS` (audit O7a).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingSource {
    pub source: OracleSource,
    pub effective_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SleeveOracleConfig {
    pub sleeve_id: Symbol,
    pub deviation_threshold_bps: u32,
    pub min_valid_sources: u32,
    pub is_paused: bool,
    pub pause_reason: Symbol,
}

// Events (audit W4.1b: typed #[contractevent]s, byte-identical to the former
// Events::publish encodings; pinned by golden/events.txt). Map-format events
// use sparse = false so every field is always written, exactly like the
// contracttype data structs below (which remain for decoding event data).

/// Topics: [NAV_PAUSE, sleeve_id]. Data: map shaped like `NavPauseData`.
#[contractevent(topics = ["NAV_PAUSE"], data_format = "map", sparse = false)]
pub struct NavPause {
    #[topic]
    pub sleeve_id: Symbol,
    pub reason: Symbol,
    pub sleeve: Symbol,
    pub sources_valid: u32,
    pub sources_total: u32,
    pub deviation_bps: u32,
    pub timestamp: u64,
}

/// Topics: [NAV_RESUME, sleeve_id]. Data: map shaped like `NavResumeData`.
#[contractevent(topics = ["NAV_RESUME"], data_format = "map", sparse = false)]
pub struct NavResume {
    #[topic]
    pub sleeve_id: Symbol,
    pub sleeve: Symbol,
    pub sources_valid: u32,
    pub timestamp: u64,
}

/// Topics: [PRICE_UPDATED, source_id]. Data: map shaped like `PriceUpdatedData`.
#[contractevent(topics = ["PRICE_UPDATED"], data_format = "map", sparse = false)]
pub struct PriceUpdated {
    #[topic]
    pub id: Symbol,
    pub source_id: Symbol,
    pub price: i128,
    pub liquidity_depth_usd: i128,
    pub timestamp: u64,
}

/// Topics: [PAUSE_AUTHORITY_SET, old_authority]. Data: new authority.
#[contractevent(topics = ["PAUSE_AUTHORITY_SET"], data_format = "single-value")]
pub struct PauseAuthoritySet {
    #[topic]
    pub old_authority: Address,
    pub new_authority: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavPauseData {
    pub reason: Symbol,
    pub sleeve: Symbol,
    pub sources_valid: u32,
    pub sources_total: u32,
    pub deviation_bps: u32,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavResumeData {
    pub sleeve: Symbol,
    pub sources_valid: u32,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceUpdatedData {
    pub source_id: Symbol,
    pub price: i128,
    pub liquidity_depth_usd: i128,
    pub timestamp: u64,
}

/// Topics: [SOURCE_CHANGE_PROPOSED, source_id]. Data: effective_at (audit O7a).
#[contractevent(topics = ["SOURCE_CHANGE_PROPOSED"], data_format = "single-value")]
pub struct SourceChangeProposed {
    #[topic]
    pub source_id: Symbol,
    pub effective_at: u64,
}

/// Topics: [SOURCE_ACTIVATED, source_id]. Data: feeder.
#[contractevent(topics = ["SOURCE_ACTIVATED"], data_format = "single-value")]
pub struct SourceActivated {
    #[topic]
    pub source_id: Symbol,
    pub feeder: Address,
}

/// Topics: [SOURCE_CHANGE_CANCELLED, source_id]. Data: effective_at.
#[contractevent(topics = ["SOURCE_CHANGE_CANCELLED"], data_format = "single-value")]
pub struct SourceChangeCancelled {
    #[topic]
    pub source_id: Symbol,
    pub effective_at: u64,
}

/// Topics: [SOURCE_DEACTIVATED, source_id]. Data: ledger timestamp.
#[contractevent(topics = ["SOURCE_DEACTIVATED"], data_format = "single-value")]
pub struct SourceDeactivated {
    #[topic]
    pub source_id: Symbol,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    BaseAsset,
    Decimals,
    Resolution,
    Source(Symbol),
    SleeveConfig(Symbol),
    SleeveSources(Symbol),
    AssetSources(Address),
    AssetSleeve(Address),
    AllAssets,
    /// SEP-40 history: conservative price snapshots, newest first (W5).
    History(Address),
    /// Source change waiting for its delay (audit O7a).
    PendingSource(Symbol),
    PauseAuthority,
}

pub trait NavOracleTrait {
    /// Configure sleeve deviation threshold (bps) and min valid sources
    fn configure_sleeve(
        env: Env,
        sleeve_id: Symbol,
        deviation_threshold_bps: u32,
        min_valid_sources: u32,
    );

    /// Register a new oracle price feed source (admin only)
    fn register_source(env: Env, source: OracleSource);

    /// Update a price feed with current price, depth, and timestamp
    fn update_price(
        env: Env,
        source_id: Symbol,
        price: i128,
        liquidity_depth_usd: i128,
        timestamp: u64,
    );

    /// Rule 3: Conservative NAV (minimum of valid sources) for collateral sizing
    fn get_collateral_nav(env: Env, sleeve_id: Symbol) -> Option<SourcedPrice>;

    /// Rule 3: Deposit NAV (median of valid sources) for share pricing
    fn get_deposit_nav(env: Env, sleeve_id: Symbol) -> Option<SourcedPrice>;

    /// Rule 3 collateral price of one asset: minimum of its valid sources.
    fn collateral_price(env: Env, asset: Address) -> Option<SourcedPrice>;

    /// SEP-40: price at `timestamp` (seconds), floored to the resolution grid;
    /// `None` for unknown assets, the future, or periods with no valid price.
    fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData>;

    /// SEP-40: the current conservative (collateral) price, evaluated live:
    /// `None` whenever Constella's rules give no valid price (stale per the
    /// source's own limit, fewer than 2 valid sources, or a paused sleeve).
    fn lastprice(env: Env, asset: Asset) -> Option<PriceData>;

    /// Fair price of one asset in the numéraire: median of the asset's own
    /// valid sources (same validity rules as every other read). The base
    /// asset is exactly 1.0. Used to value both legs of a trade (audit P1).
    fn asset_price(env: Env, asset: Address) -> Option<SourcedPrice>;

    /// SEP-40: the last `records` (1..=20) price records, newest first.
    fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>>;

    /// SEP-40 metadata queries
    fn base(env: Env) -> Asset;
    fn decimals(env: Env) -> u32;
    fn resolution(env: Env) -> u32;
    fn assets(env: Env) -> Vec<Asset>;

    /// Circuit breaker status and management
    fn is_sleeve_paused(env: Env, sleeve_id: Symbol) -> bool;
    /// Manually pause a sleeve. `caller` must sign and be the pause
    /// authority or the admin (audit O1). Fast, single-key by design.
    fn trigger_circuit_breaker(env: Env, caller: Address, sleeve_id: Symbol, reason: Symbol);
    /// Resume a sleeve. Admin (multisig) only.
    fn clear_circuit_breaker(env: Env, sleeve_id: Symbol);
    /// Rotate the pause authority. Admin only.
    fn set_pause_authority(env: Env, new_authority: Address);
    /// Current pause authority (defaults to the admin).
    fn get_pause_authority(env: Env) -> Address;

    /// Admin query
    fn get_admin(env: Env) -> Address;

    /// Inspect source
    fn get_source(env: Env, source_id: Symbol) -> Option<OracleSource>;

    /// Inspect sleeve config
    fn get_sleeve_config(env: Env, sleeve_id: Symbol) -> Option<SleeveOracleConfig>;

    /// Inspect all source IDs for a sleeve
    fn get_sleeve_sources(env: Env, sleeve_id: Symbol) -> Vec<Symbol>;

    /// Inspect all sources across all registered assets
    fn get_sources(env: Env) -> Vec<OracleSource>;

    /// Activate a pending source change once its 48h delay has passed (anyone).
    fn activate_source(env: Env, source_id: Symbol);
    /// Cancel a pending source change (admin).
    fn cancel_source_change(env: Env, source_id: Symbol);
    /// Deactivate a source immediately (admin). Fails safe: can only halt.
    fn deactivate_source(env: Env, source_id: Symbol);
    /// Pending change for a source id, if any.
    fn get_pending_source(env: Env, source_id: Symbol) -> Option<PendingSource>;
}

/// A source is fresh only if its timestamp is not in the future and is within
/// its staleness limit (audit O2: future-dated data must never count as fresh).
/// NAV-ORACLE Rule 5 / AGENT-RULES Rule 3: fewer than 2 valid sources is a
/// hard stop, so no sleeve may be configured to accept fewer (audit O3).
pub const MIN_VALID_SOURCES_FLOOR: u32 = 2;

fn is_fresh(now: u64, source: &OracleSource) -> bool {
    source.timestamp <= now && now - source.timestamp <= source.staleness_limit_seconds
}

fn sort_sources_by_price(mut sources: Vec<OracleSource>) -> Vec<OracleSource> {
    let len = sources.len();
    if len <= 1 {
        return sources;
    }
    for i in 1..len {
        let mut j = i;
        while j > 0 && sources.get(j).unwrap().price < sources.get(j - 1).unwrap().price {
            let sj = sources.get(j).unwrap();
            let sj_prev = sources.get(j - 1).unwrap();
            sources.set(j, sj_prev);
            sources.set(j - 1, sj);
            j -= 1;
        }
    }
    sources
}

/// Median of an already-validated, non-empty source list.
fn median_price(sources: Vec<OracleSource>) -> SourcedPrice {
    let sorted = sort_sources_by_price(sources);
    let len = sorted.len();
    if len % 2 == 1 {
        let m = sorted.get(len / 2).unwrap();
        SourcedPrice {
            price: m.price,
            timestamp: m.timestamp,
            source: m.source_id.clone(),
        }
    } else {
        let s1 = sorted.get(len / 2 - 1).unwrap();
        let s2 = sorted.get(len / 2).unwrap();
        SourcedPrice {
            price: s1.price + (s2.price - s1.price) / 2,
            timestamp: s1.timestamp.min(s2.timestamp),
            source: s2.source_id.clone(),
        }
    }
}

fn evaluate_sources(
    env: &Env,
    source_ids: &Vec<Symbol>,
    sleeve_id: &Symbol,
    config: &mut SleeveOracleConfig,
    persist_pause: bool,
) -> Result<Vec<OracleSource>, OracleError> {
    if config.is_paused {
        return Err(OracleError::SleevePaused);
    }

    let mut valid_sources = Vec::new(env);
    let now = env.ledger().timestamp();

    for source_id in source_ids.iter() {
        if let Some(source) = env
            .storage()
            .instance()
            .get::<_, OracleSource>(&DataKey::Source(source_id))
        {
            // Verify sleeve matches
            if source.sleeve != *sleeve_id {
                continue;
            }
            // Filter 1: Active check
            if !source.is_active {
                continue;
            }
            // Filter 2: Liquidity depth check (Rule 2)
            if source.current_liquidity_depth_usd < source.min_liquidity_depth_usd {
                continue;
            }
            // Filter 3: Staleness check (Rule 5) — future-dated data is invalid too.
            if !is_fresh(now, &source) {
                continue;
            }
            // Positive price check
            if source.price <= 0 {
                continue;
            }
            valid_sources.push_back(source);
        }
    }

    // Rule 5: Hard stop if fewer than min_valid_sources
    if valid_sources.len() < config.min_valid_sources {
        return Err(OracleError::InsufficientSources);
    }

    // Rule 4: Deviation circuit breaker check
    let mut min_price = i128::MAX;
    let mut max_price = i128::MIN;
    for s in valid_sources.iter() {
        if s.price < min_price {
            min_price = s.price;
        }
        if s.price > max_price {
            max_price = s.price;
        }
    }

    let diff = max_price - min_price;
    let dev_bps = match diff.checked_mul(10_000) {
        Some(product) => product / min_price,
        None => i128::MAX,
    };

    if dev_bps > config.deviation_threshold_bps as i128 {
        if !persist_pause {
            return Err(OracleError::DeviationExceeded);
        }
        config.is_paused = true;
        config.pause_reason = Symbol::new(env, "DEVIATION");
        env.storage()
            .instance()
            .set(&DataKey::SleeveConfig(sleeve_id.clone()), config);

        let dev_bps_clamped = if dev_bps > u32::MAX as i128 {
            u32::MAX
        } else {
            dev_bps as u32
        };

        NavPause {
            sleeve_id: sleeve_id.clone(),
            reason: Symbol::new(env, "DEVIATION"),
            sleeve: sleeve_id.clone(),
            sources_valid: valid_sources.len(),
            sources_total: source_ids.len(),
            deviation_bps: dev_bps_clamped,
            timestamp: now,
        }
        .publish(env);
        return Err(OracleError::DeviationExceeded);
    }

    Ok(valid_sources)
}

fn sleeve_config(env: &Env, sleeve_id: &Symbol) -> SleeveOracleConfig {
    env.storage()
        .instance()
        .get(&DataKey::SleeveConfig(sleeve_id.clone()))
        .unwrap_or(SleeveOracleConfig {
            sleeve_id: sleeve_id.clone(),
            deviation_threshold_bps: 200,
            min_valid_sources: MIN_VALID_SOURCES_FLOOR,
            is_paused: false,
            pause_reason: Symbol::new(env, ""),
        })
}

/// Audit O6: validate ONE asset's sources under its sleeve's configuration.
/// Deviation, the minimum-source rule and median/minimum are computed among
/// the same asset's sources only; a deviation pauses the asset's sleeve
/// (NAV-ORACLE Rule 4: "deposits paused for the affected sleeve").
fn evaluate_asset(env: &Env, asset: &Address) -> Result<Vec<OracleSource>, OracleError> {
    let sleeve_id: Symbol = env
        .storage()
        .instance()
        .get(&DataKey::AssetSleeve(asset.clone()))
        .ok_or(OracleError::AssetNotFound)?;
    let mut config = sleeve_config(env, &sleeve_id);
    let source_ids: Vec<Symbol> = env
        .storage()
        .instance()
        .get(&DataKey::AssetSources(asset.clone()))
        .unwrap_or_else(|| Vec::new(env));
    evaluate_sources(env, &source_ids, &sleeve_id, &mut config, true)
}

/// The sleeve's only asset, or None when it has no sources or more than one
/// asset: a sleeve-level price across different assets is meaningless (O6).
fn single_asset_of_sleeve(env: &Env, sleeve_id: &Symbol) -> Option<Address> {
    let source_ids: Vec<Symbol> = env
        .storage()
        .instance()
        .get(&DataKey::SleeveSources(sleeve_id.clone()))
        .unwrap_or_else(|| Vec::new(env));
    let mut asset: Option<Address> = None;
    for id in source_ids.iter() {
        let src: OracleSource = env.storage().instance().get(&DataKey::Source(id))?;
        match &asset {
            None => asset = Some(src.asset),
            Some(a) if *a == src.asset => {}
            Some(_) => return None,
        }
    }
    asset
}

/// Audit O4/O6: evaluate the written asset's sources right after a price
/// write and latch a deviation pause (persisting `NAV_PAUSE`) inside that
/// successful call. Previously deviation was only detected on reads (a read
/// inside a reverting caller rolled the pause back), and it compared every
/// source in the sleeve, including other assets' sources (false pauses).
/// Staleness and insufficient sources remain live checks on every read.
fn latch_deviation_on_write(env: &Env, asset: &Address) {
    let sleeve_id: Symbol = match env.storage().instance().get(&DataKey::AssetSleeve(asset.clone())) {
        Some(s) => s,
        None => return,
    };
    if sleeve_config(env, &sleeve_id).is_paused {
        return;
    }
    // On DeviationExceeded the pause + NAV_PAUSE are already persisted by
    // evaluate_sources; InsufficientSources is intentionally not latched.
    let _ = evaluate_asset(env, asset);
}

/// Reflector-compatible cap on `prices` records (audit W5).
pub const MAX_PRICE_RECORDS: u32 = 20;
const HISTORY_TTL_THRESHOLD: u32 = 29 * 17_280;
const HISTORY_EXTEND_AMOUNT: u32 = 30 * 17_280;

/// Floor a timestamp (seconds) to the resolution grid.
fn on_grid(env: &Env, t: u64) -> u64 {
    let r: u32 = env.storage().instance().get(&DataKey::Resolution).unwrap_or(0);
    if r == 0 {
        t
    } else {
        t / r as u64 * r as u64
    }
}

fn base_asset(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::BaseAsset)
}

fn one(env: &Env) -> i128 {
    let d: u32 = env.storage().instance().get(&DataKey::Decimals).unwrap_or(7);
    10i128.pow(d)
}

/// Conservative (collateral) SEP-40 price of `asset` right now: minimum of
/// its valid sources, stamped with the OLDEST data used, on the grid. The base
/// asset is exactly 1.0. `None` whenever the asset has no valid price.
fn conservative_now(env: &Env, asset: &Address) -> Option<PriceData> {
    if base_asset(env).as_ref() == Some(asset) {
        return Some(PriceData { price: one(env), timestamp: on_grid(env, env.ledger().timestamp()) });
    }
    let valid = evaluate_asset(env, asset).ok()?;
    let mut price = i128::MAX;
    let mut oldest = u64::MAX;
    for s in valid.iter() {
        price = price.min(s.price);
        oldest = oldest.min(s.timestamp);
    }
    Some(PriceData { price, timestamp: on_grid(env, oldest) })
}

fn history(env: &Env, asset: &Address) -> Vec<PriceData> {
    env.storage().persistent().get(&DataKey::History(asset.clone())).unwrap_or_else(|| Vec::new(env))
}

/// After a successful price write, record the asset's conservative price in
/// the grid bucket of the CURRENT ledger time - what the oracle reported then -
/// so a record never contains data newer than its timestamp (no look-ahead).
/// Latest write in a bucket wins; never backwards; capped at MAX_PRICE_RECORDS.
fn record_snapshot(env: &Env, asset: &Address) {
    let Some(live) = conservative_now(env, asset) else { return };
    let pd = PriceData { price: live.price, timestamp: on_grid(env, env.ledger().timestamp()) };
    let mut h = history(env, asset);
    match h.first() {
        Some(newest) if newest.timestamp == pd.timestamp => h.set(0, pd),
        Some(newest) if newest.timestamp > pd.timestamp => return,
        _ => {
            h.push_front(pd);
            while h.len() > MAX_PRICE_RECORDS {
                h.pop_back();
            }
        }
    }
    let key = DataKey::History(asset.clone());
    env.storage().persistent().set(&key, &h);
    env.storage().persistent().extend_ttl(&key, HISTORY_TTL_THRESHOLD, HISTORY_EXTEND_AMOUNT);
}

/// Delay before a change to an already-priced asset's sources takes effect
/// (audit O7a): 48 h, the same window as upgrades, policy amendments and
/// account recovery, so watchers can react before prices can be affected.
pub const SOURCE_CHANGE_DELAY_SECONDS: u64 = 48 * 3600;

/// Whether `asset` has a valid price right now (no side effects).
fn asset_is_priced(env: &Env, asset: &Address) -> bool {
    let Some(sleeve_id) = env.storage().instance().get::<_, Symbol>(&DataKey::AssetSleeve(asset.clone())) else {
        return false;
    };
    let mut config = sleeve_config(env, &sleeve_id);
    let ids: Vec<Symbol> = env.storage().instance().get(&DataKey::AssetSources(asset.clone())).unwrap_or_else(|| Vec::new(env));
    evaluate_sources(env, &ids, &sleeve_id, &mut config, false).is_ok()
}

fn admin_of(env: &Env) -> Address {
    match env.storage().instance().get(&DataKey::Admin) {
        Some(a) => a,
        None => panic_with_error!(env, OracleError::NotInitialized),
    }
}

/// Install a source (indexes, deviation latch, history snapshot).
fn apply_source(env: &Env, source: &OracleSource) {
    env.storage()
        .instance()
        .set(&DataKey::Source(source.source_id.clone()), source);

    // Sleeve sources index
    let mut sleeve_sources: Vec<Symbol> = env
        .storage()
        .instance()
        .get(&DataKey::SleeveSources(source.sleeve.clone()))
        .unwrap_or_else(|| Vec::new(env));
    let mut found_sleeve_source = false;
    for s in sleeve_sources.iter() {
        if s == source.source_id {
            found_sleeve_source = true;
            break;
        }
    }
    if !found_sleeve_source {
        sleeve_sources.push_back(source.source_id.clone());
        env.storage()
            .instance()
            .set(&DataKey::SleeveSources(source.sleeve.clone()), &sleeve_sources);
    }

    // Asset sources index
    let mut asset_sources: Vec<Symbol> = env
        .storage()
        .instance()
        .get(&DataKey::AssetSources(source.asset.clone()))
        .unwrap_or_else(|| Vec::new(env));
    let mut found_asset_source = false;
    for s in asset_sources.iter() {
        if s == source.source_id {
            found_asset_source = true;
            break;
        }
    }
    if !found_asset_source {
        asset_sources.push_back(source.source_id.clone());
        env.storage()
            .instance()
            .set(&DataKey::AssetSources(source.asset.clone()), &asset_sources);
    }

    // Asset sleeve mapping
    env.storage()
        .instance()
        .set(&DataKey::AssetSleeve(source.asset.clone()), &source.sleeve);

    // All assets list
    let mut all_assets: Vec<Address> = env
        .storage()
        .instance()
        .get(&DataKey::AllAssets)
        .unwrap_or_else(|| Vec::new(env));
    let mut found_asset = false;
    for a in all_assets.iter() {
        if a == source.asset {
            found_asset = true;
            break;
        }
    }
    if !found_asset {
        all_assets.push_back(source.asset.clone());
        env.storage().instance().set(&DataKey::AllAssets, &all_assets);
    }

    latch_deviation_on_write(env, &source.asset);
    record_snapshot(env, &source.asset);
}

#[contract]
pub struct NavOracle;

#[contractimpl]
impl NavOracle {
    /// Runs atomically as part of deployment, so no one can claim an
    /// uninitialized oracle (audit M3). There is no `initialize` entrypoint.
    pub fn __constructor(env: Env, admin: Address, base_asset: Address, decimals: u32, resolution: u32) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::PauseAuthority, &admin);
        env.storage().instance().set(&DataKey::BaseAsset, &base_asset);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::Resolution, &resolution);
        let empty_assets: Vec<Address> = Vec::new(&env);
        env.storage().instance().set(&DataKey::AllAssets, &empty_assets);
    }
}

#[contractimpl]
impl NavOracleTrait for NavOracle {

    fn configure_sleeve(
        env: Env,
        sleeve_id: Symbol,
        deviation_threshold_bps: u32,
        min_valid_sources: u32,
    ) {
        let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        };
        admin.require_auth();

        if deviation_threshold_bps == 0
            || deviation_threshold_bps > 10000
            || min_valid_sources < MIN_VALID_SOURCES_FLOOR
        {
            panic_with_error!(&env, OracleError::InvalidThreshold);
        }

        let mut config: SleeveOracleConfig = env
            .storage()
            .instance()
            .get(&DataKey::SleeveConfig(sleeve_id.clone()))
            .unwrap_or(SleeveOracleConfig {
                sleeve_id: sleeve_id.clone(),
                deviation_threshold_bps,
                min_valid_sources,
                is_paused: false,
                pause_reason: Symbol::new(&env, ""),
            });

        config.deviation_threshold_bps = deviation_threshold_bps;
        config.min_valid_sources = min_valid_sources;
        env.storage()
            .instance()
            .set(&DataKey::SleeveConfig(sleeve_id), &config);
    }

    fn register_source(env: Env, source: OracleSource) {
        let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        };
        admin.require_auth();

        if source.staleness_limit_seconds == 0 {
            panic_with_error!(&env, OracleError::PriceStale);
        }
        if source.min_liquidity_depth_usd < 0 {
            panic_with_error!(&env, OracleError::InsufficientLiquidityDepth);
        }
        if source.timestamp > env.ledger().timestamp() {
            panic_with_error!(&env, OracleError::FutureTimestamp);
        }
        // Audit O6: one sleeve per asset; a source id keeps its asset and sleeve.
        if let Some(existing) = env.storage().instance().get::<_, OracleSource>(&DataKey::Source(source.source_id.clone())) {
            if existing.asset != source.asset || existing.sleeve != source.sleeve {
                panic_with_error!(&env, OracleError::AssetSleeveMismatch);
            }
        }
        if let Some(sleeve) = env.storage().instance().get::<_, Symbol>(&DataKey::AssetSleeve(source.asset.clone())) {
            if sleeve != source.sleeve {
                panic_with_error!(&env, OracleError::AssetSleeveMismatch);
            }
        }
        // Audit O7a: changes to an asset that is already priced, and any change
        // to an existing source (feeder rotation, looser limits), wait 48 h.
        // Bootstrapping an unpriced asset takes effect at once: nothing can
        // consume a price that does not exist yet.
        let exists = env.storage().instance().has(&DataKey::Source(source.source_id.clone()));
        if exists || asset_is_priced(&env, &source.asset) {
            let effective_at = env.ledger().timestamp().saturating_add(SOURCE_CHANGE_DELAY_SECONDS);
            let id = source.source_id.clone();
            env.storage().instance().set(&DataKey::PendingSource(id.clone()), &PendingSource { source, effective_at });
            SourceChangeProposed { source_id: id, effective_at }.publish(&env);
            return;
        }
        apply_source(&env, &source);
    }

    fn update_price(
        env: Env,
        source_id: Symbol,
        price: i128,
        liquidity_depth_usd: i128,
        timestamp: u64,
    ) {

        let mut source: OracleSource = match env
            .storage()
            .instance()
            .get(&DataKey::Source(source_id.clone()))
        {
            Some(s) => s,
            None => panic_with_error!(&env, OracleError::AssetNotFound),
        };
        // Audit O7a: only this source's own feeder may push its price.
        source.feeder.require_auth();

        if price <= 0 {
            panic_with_error!(&env, OracleError::PriceStale);
        }
        if liquidity_depth_usd < 0 {
            panic_with_error!(&env, OracleError::InsufficientLiquidityDepth);
        }
        // Audit O2: a future timestamp would never go stale; an older one would
        // overwrite newer data (replay of a still-fresh old price).
        if timestamp > env.ledger().timestamp() {
            panic_with_error!(&env, OracleError::FutureTimestamp);
        }
        if timestamp < source.timestamp {
            panic_with_error!(&env, OracleError::OutOfOrderUpdate);
        }

        source.price = price;
        source.current_liquidity_depth_usd = liquidity_depth_usd;
        source.timestamp = timestamp;
        env.storage()
            .instance()
            .set(&DataKey::Source(source_id.clone()), &source);

        PriceUpdated {
            id: source_id.clone(),
            source_id,
            price,
            liquidity_depth_usd,
            timestamp,
        }
        .publish(&env);
        latch_deviation_on_write(&env, &source.asset);
        record_snapshot(&env, &source.asset);
    }

    fn get_collateral_nav(env: Env, sleeve_id: Symbol) -> Option<SourcedPrice> {
        // Audit O6: only a single-asset sleeve has a sleeve-level price.
        let asset = single_asset_of_sleeve(&env, &sleeve_id)?;
        Self::collateral_price(env, asset)
    }

    fn get_deposit_nav(env: Env, sleeve_id: Symbol) -> Option<SourcedPrice> {
        // Audit O6: only a single-asset sleeve has a sleeve-level price.
        let asset = single_asset_of_sleeve(&env, &sleeve_id)?;
        let valid_sources = evaluate_asset(&env, &asset).ok()?;
        Some(median_price(valid_sources))
    }

    fn collateral_price(env: Env, asset: Address) -> Option<SourcedPrice> {
        let valid_sources = evaluate_asset(&env, &asset).ok()?;
        let mut min_source = valid_sources.get(0).unwrap();
        for s in valid_sources.iter() {
            if s.price < min_source.price {
                min_source = s;
            }
        }
        Some(SourcedPrice {
            price: min_source.price,
            timestamp: min_source.timestamp,
            source: min_source.source_id.clone(),
        })
    }

    fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData> {
        let Asset::Stellar(asset) = asset else { return None };
        if timestamp > env.ledger().timestamp() {
            return None;
        }
        let bucket = on_grid(&env, timestamp);
        if base_asset(&env).as_ref() == Some(&asset) {
            return Some(PriceData { price: one(&env), timestamp: bucket });
        }
        history(&env, &asset).iter().find(|p| p.timestamp == bucket)
    }

    fn asset_price(env: Env, asset: Address) -> Option<SourcedPrice> {
        let base: Address = env.storage().instance().get(&DataKey::BaseAsset)?;
        if asset == base {
            let decimals: u32 = env.storage().instance().get(&DataKey::Decimals)?;
            return Some(SourcedPrice {
                price: 10i128.pow(decimals),
                timestamp: env.ledger().timestamp(),
                source: Symbol::new(&env, "BASE"),
            });
        }
        let valid = evaluate_asset(&env, &asset).ok()?;
        Some(median_price(valid))
    }

    fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        // Audit W5: live evaluation. The former implementation returned a
        // stored price even after the sources went stale or the sleeve paused;
        // a consumer with a looser staleness window (Blend: 24h) would then
        // lend against a price Constella itself considers invalid.
        let Asset::Stellar(asset) = asset else { return None };
        conservative_now(&env, &asset)
    }

    fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
        if records == 0 || records > MAX_PRICE_RECORDS {
            return None;
        }
        let Asset::Stellar(asset) = asset else { return None };
        let mut out = Vec::new(&env);
        if base_asset(&env).as_ref() == Some(&asset) {
            let r: u32 = env.storage().instance().get(&DataKey::Resolution).unwrap_or(0);
            let mut t = on_grid(&env, env.ledger().timestamp());
            for _ in 0..records {
                out.push_back(PriceData { price: one(&env), timestamp: t });
                if r == 0 || t < r as u64 {
                    break;
                }
                t -= r as u64;
            }
            return Some(out);
        }
        for p in history(&env, &asset).iter().take(records as usize) {
            out.push_back(p);
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn base(env: Env) -> Asset {
        match base_asset(&env) {
            Some(a) => Asset::Stellar(a),
            None => panic_with_error!(&env, OracleError::NotInitialized),
        }
    }

    fn decimals(env: Env) -> u32 {
        match env.storage().instance().get(&DataKey::Decimals) {
            Some(d) => d,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        }
    }

    fn resolution(env: Env) -> u32 {
        match env.storage().instance().get(&DataKey::Resolution) {
            Some(r) => r,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        }
    }

    fn assets(env: Env) -> Vec<Asset> {
        let all: Vec<Address> = env.storage().instance().get(&DataKey::AllAssets).unwrap_or_else(|| Vec::new(&env));
        Vec::from_iter(&env, all.iter().map(Asset::Stellar))
    }

    fn is_sleeve_paused(env: Env, sleeve_id: Symbol) -> bool {
        if let Some(config) = env
            .storage()
            .instance()
            .get::<_, SleeveOracleConfig>(&DataKey::SleeveConfig(sleeve_id))
        {
            config.is_paused
        } else {
            false
        }
    }

    fn trigger_circuit_breaker(env: Env, caller: Address, sleeve_id: Symbol, reason: Symbol) {
        let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        };
        // Audit O1: only the pause authority or the admin may pause, and they
        // must actually sign. Previously anyone could pause any sleeve.
        caller.require_auth();
        let pause_authority: Address = env
            .storage()
            .instance()
            .get(&DataKey::PauseAuthority)
            .unwrap_or(admin.clone());
        if caller != pause_authority && caller != admin {
            panic_with_error!(&env, OracleError::Unauthorized);
        }

        let has_sources = env.storage().instance().has(&DataKey::SleeveSources(sleeve_id.clone()));
        let has_config = env.storage().instance().has(&DataKey::SleeveConfig(sleeve_id.clone()));
        if !has_sources && !has_config {
            panic_with_error!(&env, OracleError::SleeveNotFound);
        }

        let mut config: SleeveOracleConfig = env
            .storage()
            .instance()
            .get(&DataKey::SleeveConfig(sleeve_id.clone()))
            .unwrap_or(SleeveOracleConfig {
                sleeve_id: sleeve_id.clone(),
                deviation_threshold_bps: 200,
                min_valid_sources: MIN_VALID_SOURCES_FLOOR,
                is_paused: false,
                pause_reason: Symbol::new(&env, ""),
            });

        config.is_paused = true;
        config.pause_reason = reason.clone();
        env.storage()
            .instance()
            .set(&DataKey::SleeveConfig(sleeve_id.clone()), &config);

        let source_ids: Vec<Symbol> = env
            .storage()
            .instance()
            .get::<_, Vec<Symbol>>(&DataKey::SleeveSources(sleeve_id.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        let now = env.ledger().timestamp();
        let mut sources_valid: u32 = 0;
        for s_id in source_ids.iter() {
            if let Some(source) = env
                .storage()
                .instance()
                .get::<_, OracleSource>(&DataKey::Source(s_id))
            {
                if source.is_active
                    && source.sleeve == sleeve_id
                    && source.current_liquidity_depth_usd >= source.min_liquidity_depth_usd
                    && is_fresh(now, &source)
                    && source.price > 0
                {
                    sources_valid += 1;
                }
            }
        }

        NavPause {
            sleeve_id: sleeve_id.clone(),
            reason,
            sleeve: sleeve_id,
            sources_valid,
            sources_total: source_ids.len(),
            deviation_bps: 0,
            timestamp: now,
        }
        .publish(&env);
    }

    fn clear_circuit_breaker(env: Env, sleeve_id: Symbol) {
        let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        };
        admin.require_auth();

        let mut config: SleeveOracleConfig = match env
            .storage()
            .instance()
            .get(&DataKey::SleeveConfig(sleeve_id.clone()))
        {
            Some(c) => c,
            None => panic_with_error!(&env, OracleError::SleeveNotFound),
        };

        config.is_paused = false;
        config.pause_reason = Symbol::new(&env, "");
        env.storage()
            .instance()
            .set(&DataKey::SleeveConfig(sleeve_id.clone()), &config);

        let source_ids: Vec<Symbol> = env
            .storage()
            .instance()
            .get::<_, Vec<Symbol>>(&DataKey::SleeveSources(sleeve_id.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        let now = env.ledger().timestamp();
        let mut sources_valid: u32 = 0;
        for s_id in source_ids.iter() {
            if let Some(source) = env
                .storage()
                .instance()
                .get::<_, OracleSource>(&DataKey::Source(s_id))
            {
                if source.is_active
                    && source.sleeve == sleeve_id
                    && source.current_liquidity_depth_usd >= source.min_liquidity_depth_usd
                    && is_fresh(now, &source)
                    && source.price > 0
                {
                    sources_valid += 1;
                }
            }
        }

        NavResume {
            sleeve_id: sleeve_id.clone(),
            sleeve: sleeve_id,
            sources_valid,
            timestamp: now,
        }
        .publish(&env);
    }

    fn set_pause_authority(env: Env, new_authority: Address) {
        let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        };
        admin.require_auth();
        let old: Address = env
            .storage()
            .instance()
            .get(&DataKey::PauseAuthority)
            .unwrap_or(admin);
        env.storage()
            .instance()
            .set(&DataKey::PauseAuthority, &new_authority);
        PauseAuthoritySet {
            old_authority: old,
            new_authority,
        }
        .publish(&env);
    }

    fn get_pause_authority(env: Env) -> Address {
        let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        };
        env.storage()
            .instance()
            .get(&DataKey::PauseAuthority)
            .unwrap_or(admin)
    }

    fn get_admin(env: Env) -> Address {
        match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        }
    }

    fn get_source(env: Env, source_id: Symbol) -> Option<OracleSource> {
        env.storage().instance().get(&DataKey::Source(source_id))
    }

    fn get_sleeve_config(env: Env, sleeve_id: Symbol) -> Option<SleeveOracleConfig> {
        env.storage().instance().get(&DataKey::SleeveConfig(sleeve_id))
    }

    fn get_sleeve_sources(env: Env, sleeve_id: Symbol) -> Vec<Symbol> {
        env.storage()
            .instance()
            .get(&DataKey::SleeveSources(sleeve_id))
            .unwrap_or_else(|| Vec::new(&env))
    }

    fn activate_source(env: Env, source_id: Symbol) {
        let pending: PendingSource = env.storage().instance().get(&DataKey::PendingSource(source_id.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, OracleError::NoPendingSourceChange));
        if env.ledger().timestamp() < pending.effective_at {
            panic_with_error!(&env, OracleError::SourceChangeNotReady);
        }
        env.storage().instance().remove(&DataKey::PendingSource(source_id.clone()));
        let mut source = pending.source;
        // An existing source keeps its current price data: registration may
        // change configuration (feeder, limits, active flag), never a price.
        if let Some(current) = env.storage().instance().get::<_, OracleSource>(&DataKey::Source(source_id.clone())) {
            source.price = current.price;
            source.timestamp = current.timestamp;
            source.current_liquidity_depth_usd = current.current_liquidity_depth_usd;
        }
        let feeder = source.feeder.clone();
        apply_source(&env, &source);
        SourceActivated { source_id, feeder }.publish(&env);
    }

    fn cancel_source_change(env: Env, source_id: Symbol) {
        admin_of(&env).require_auth();
        let pending: PendingSource = env.storage().instance().get(&DataKey::PendingSource(source_id.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, OracleError::NoPendingSourceChange));
        env.storage().instance().remove(&DataKey::PendingSource(source_id.clone()));
        SourceChangeCancelled { source_id, effective_at: pending.effective_at }.publish(&env);
    }

    fn deactivate_source(env: Env, source_id: Symbol) {
        admin_of(&env).require_auth();
        let mut source: OracleSource = env.storage().instance().get(&DataKey::Source(source_id.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, OracleError::AssetNotFound));
        source.is_active = false;
        env.storage().instance().set(&DataKey::Source(source_id.clone()), &source);
        SourceDeactivated { source_id, timestamp: env.ledger().timestamp() }.publish(&env);
    }

    fn get_pending_source(env: Env, source_id: Symbol) -> Option<PendingSource> {
        env.storage().instance().get(&DataKey::PendingSource(source_id))
    }

    fn get_sources(env: Env) -> Vec<OracleSource> {
        let mut sources: Vec<OracleSource> = Vec::new(&env);
        let assets: Vec<Address> = env.storage().instance().get(&DataKey::AllAssets).unwrap_or_else(|| Vec::new(&env));
        for asset in assets.iter() {
            let asset_sources: Vec<Symbol> = env
                .storage()
                .instance()
                .get(&DataKey::AssetSources(asset))
                .unwrap_or_else(|| Vec::new(&env));
            for src_id in asset_sources.iter() {
                if let Some(src) = env
                    .storage()
                    .instance()
                    .get::<_, OracleSource>(&DataKey::Source(src_id))
                {
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
        sources
    }
}

#[cfg(test)]
mod test;
