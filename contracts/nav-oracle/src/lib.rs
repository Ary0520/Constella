#![no_std]
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Env, Symbol,
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
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
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
    LastPrice(Address),
    PauseAuthority,
}

pub trait NavOracleTrait {
    /// Initialize oracle with admin, base asset (numéraire), decimals, and resolution
    fn initialize(env: Env, admin: Address, base_asset: Address, decimals: u32, resolution: u32);

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
    fn get_collateral_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>;

    /// Rule 3: Deposit NAV (median of valid sources) for share pricing
    fn get_deposit_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>;

    /// SEP-40: Standard price query
    fn price(env: Env, asset: Address) -> Option<PriceData>;

    /// SEP-40: Standard last price query
    fn lastprice(env: Env, asset: Address) -> Option<PriceData>;

    /// SEP-40: Standard batch price query
    fn prices(env: Env, assets: Vec<Address>, records: u32) -> Option<Vec<Option<PriceData>>>;

    /// SEP-40 metadata queries
    fn base(env: Env) -> Address;
    fn decimals(env: Env) -> u32;
    fn resolution(env: Env) -> u32;
    fn assets(env: Env) -> Vec<Address>;

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

fn evaluate_sources(
    env: &Env,
    source_ids: &Vec<Symbol>,
    sleeve_id: &Symbol,
    config: &mut SleeveOracleConfig,
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
            // Filter 3: Staleness check (Rule 5)
            if now.saturating_sub(source.timestamp) > source.staleness_limit_seconds {
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

        env.events().publish(
            (Symbol::new(env, "NAV_PAUSE"), sleeve_id.clone()),
            NavPauseData {
                reason: Symbol::new(env, "DEVIATION"),
                sleeve: sleeve_id.clone(),
                sources_valid: valid_sources.len(),
                sources_total: source_ids.len(),
                deviation_bps: dev_bps_clamped,
                timestamp: now,
            },
        );
        return Err(OracleError::DeviationExceeded);
    }

    Ok(valid_sources)
}

/// Audit O4: evaluate a sleeve's sources right after a price write and latch a
/// deviation pause (persisting `NAV_PAUSE`) inside that successful call.
///
/// Previously deviation was only detected on reads; a read inside a caller
/// that later reverted rolled the pause and its event back. Staleness and
/// insufficient-sources remain live checks on every read (never latched).
fn latch_deviation_on_write(env: &Env, sleeve_id: &Symbol) {
    let mut config: SleeveOracleConfig = env
        .storage()
        .instance()
        .get(&DataKey::SleeveConfig(sleeve_id.clone()))
        .unwrap_or(SleeveOracleConfig {
            sleeve_id: sleeve_id.clone(),
            deviation_threshold_bps: 200,
            min_valid_sources: 2,
            is_paused: false,
            pause_reason: Symbol::new(env, ""),
        });
    if config.is_paused {
        return;
    }
    let source_ids: Vec<Symbol> = env
        .storage()
        .instance()
        .get(&DataKey::SleeveSources(sleeve_id.clone()))
        .unwrap_or_else(|| Vec::new(env));
    // On DeviationExceeded the pause + NAV_PAUSE are already persisted by
    // evaluate_sources; InsufficientSources is intentionally not latched.
    let _ = evaluate_sources(env, &source_ids, sleeve_id, &mut config);
}

#[contract]
pub struct NavOracle;

#[contractimpl]
impl NavOracleTrait for NavOracle {
    fn initialize(env: Env, admin: Address, base_asset: Address, decimals: u32, resolution: u32) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, OracleError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::PauseAuthority, &admin);
        env.storage().instance().set(&DataKey::BaseAsset, &base_asset);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::Resolution, &resolution);
        let empty_assets: Vec<Address> = Vec::new(&env);
        env.storage().instance().set(&DataKey::AllAssets, &empty_assets);
    }

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
            || min_valid_sources == 0
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

        env.storage()
            .instance()
            .set(&DataKey::Source(source.source_id.clone()), &source);

        // Sleeve sources index
        let mut sleeve_sources: Vec<Symbol> = env
            .storage()
            .instance()
            .get(&DataKey::SleeveSources(source.sleeve.clone()))
            .unwrap_or_else(|| Vec::new(&env));
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
            .unwrap_or_else(|| Vec::new(&env));
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
            .unwrap_or_else(|| Vec::new(&env));
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

        latch_deviation_on_write(&env, &source.sleeve);
    }

    fn update_price(
        env: Env,
        source_id: Symbol,
        price: i128,
        liquidity_depth_usd: i128,
        timestamp: u64,
    ) {
        let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic_with_error!(&env, OracleError::NotInitialized),
        };
        admin.require_auth();

        let mut source: OracleSource = match env
            .storage()
            .instance()
            .get(&DataKey::Source(source_id.clone()))
        {
            Some(s) => s,
            None => panic_with_error!(&env, OracleError::AssetNotFound),
        };

        if price <= 0 {
            panic_with_error!(&env, OracleError::PriceStale);
        }
        if liquidity_depth_usd < 0 {
            panic_with_error!(&env, OracleError::InsufficientLiquidityDepth);
        }

        source.price = price;
        source.current_liquidity_depth_usd = liquidity_depth_usd;
        source.timestamp = timestamp;
        env.storage()
            .instance()
            .set(&DataKey::Source(source_id.clone()), &source);

        env.events().publish(
            (Symbol::new(&env, "PRICE_UPDATED"), source_id.clone()),
            PriceUpdatedData {
                source_id,
                price,
                liquidity_depth_usd,
                timestamp,
            },
        );

        latch_deviation_on_write(&env, &source.sleeve);
    }

    fn get_collateral_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData> {
        let mut config: SleeveOracleConfig = env
            .storage()
            .instance()
            .get(&DataKey::SleeveConfig(sleeve_id.clone()))
            .unwrap_or(SleeveOracleConfig {
                sleeve_id: sleeve_id.clone(),
                deviation_threshold_bps: 200,
                min_valid_sources: 2,
                is_paused: false,
                pause_reason: Symbol::new(&env, ""),
            });

        let source_ids: Vec<Symbol> = env
            .storage()
            .instance()
            .get(&DataKey::SleeveSources(sleeve_id.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        let valid_sources = evaluate_sources(&env, &source_ids, &sleeve_id, &mut config).ok()?;

        let mut min_source = valid_sources.get(0).unwrap();
        for s in valid_sources.iter() {
            if s.price < min_source.price {
                min_source = s;
            }
        }

        let price_data = PriceData {
            price: min_source.price,
            timestamp: min_source.timestamp,
            source: min_source.source_id.clone(),
        };

        env.storage()
            .instance()
            .set(&DataKey::LastPrice(min_source.asset.clone()), &price_data);

        Some(price_data)
    }

    fn get_deposit_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData> {
        let mut config: SleeveOracleConfig = env
            .storage()
            .instance()
            .get(&DataKey::SleeveConfig(sleeve_id.clone()))
            .unwrap_or(SleeveOracleConfig {
                sleeve_id: sleeve_id.clone(),
                deviation_threshold_bps: 200,
                min_valid_sources: 2,
                is_paused: false,
                pause_reason: Symbol::new(&env, ""),
            });

        let source_ids: Vec<Symbol> = env
            .storage()
            .instance()
            .get(&DataKey::SleeveSources(sleeve_id.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        let valid_sources = evaluate_sources(&env, &source_ids, &sleeve_id, &mut config).ok()?;

        let sorted = sort_sources_by_price(valid_sources);
        let len = sorted.len();

        let price_data = if len % 2 == 1 {
            let mid = len / 2;
            let median_source = sorted.get(mid).unwrap();
            PriceData {
                price: median_source.price,
                timestamp: median_source.timestamp,
                source: median_source.source_id.clone(),
            }
        } else {
            let mid = len / 2;
            let s1 = sorted.get(mid - 1).unwrap();
            let s2 = sorted.get(mid).unwrap();
            let median_price = s1.price + (s2.price - s1.price) / 2;
            PriceData {
                price: median_price,
                timestamp: s1.timestamp.min(s2.timestamp),
                source: s2.source_id.clone(),
            }
        };

        Some(price_data)
    }

    fn price(env: Env, asset: Address) -> Option<PriceData> {
        let sleeve_id: Symbol = env
            .storage()
            .instance()
            .get(&DataKey::AssetSleeve(asset.clone()))?;

        let mut config: SleeveOracleConfig = env
            .storage()
            .instance()
            .get(&DataKey::SleeveConfig(sleeve_id.clone()))
            .unwrap_or(SleeveOracleConfig {
                sleeve_id: sleeve_id.clone(),
                deviation_threshold_bps: 200,
                min_valid_sources: 2,
                is_paused: false,
                pause_reason: Symbol::new(&env, ""),
            });

        let source_ids: Vec<Symbol> = env
            .storage()
            .instance()
            .get(&DataKey::AssetSources(asset.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        let valid_sources = evaluate_sources(&env, &source_ids, &sleeve_id, &mut config).ok()?;

        let mut min_source = valid_sources.get(0).unwrap();
        for s in valid_sources.iter() {
            if s.price < min_source.price {
                min_source = s;
            }
        }

        let price_data = PriceData {
            price: min_source.price,
            timestamp: min_source.timestamp,
            source: min_source.source_id.clone(),
        };

        env.storage()
            .instance()
            .set(&DataKey::LastPrice(asset), &price_data);

        Some(price_data)
    }

    fn lastprice(env: Env, asset: Address) -> Option<PriceData> {
        if let Some(lp) = env
            .storage()
            .instance()
            .get::<_, PriceData>(&DataKey::LastPrice(asset.clone()))
        {
            return Some(lp);
        }

        // If not explicitly stored in LastPrice, look for active source with valid depth
        let source_ids: Vec<Symbol> = env
            .storage()
            .instance()
            .get(&DataKey::AssetSources(asset))
            .unwrap_or_else(|| Vec::new(&env));

        let mut min_price = i128::MAX;
        let mut min_source: Option<OracleSource> = None;

        for s_id in source_ids.iter() {
            if let Some(s) = env
                .storage()
                .instance()
                .get::<_, OracleSource>(&DataKey::Source(s_id))
            {
                if s.is_active
                    && s.current_liquidity_depth_usd >= s.min_liquidity_depth_usd
                    && s.price > 0
                    && s.price < min_price
                {
                    min_price = s.price;
                    min_source = Some(s);
                }
            }
        }

        min_source.map(|s| PriceData {
            price: s.price,
            timestamp: s.timestamp,
            source: s.source_id,
        })
    }

    fn prices(env: Env, assets: Vec<Address>, records: u32) -> Option<Vec<Option<PriceData>>> {
        if records == 0 {
            return Some(Vec::new(&env));
        }
        let mut results = Vec::new(&env);
        for asset in assets.iter() {
            let p = Self::price(env.clone(), asset);
            results.push_back(p);
        }
        Some(results)
    }

    fn base(env: Env) -> Address {
        match env.storage().instance().get(&DataKey::BaseAsset) {
            Some(a) => a,
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

    fn assets(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::AllAssets)
            .unwrap_or_else(|| Vec::new(&env))
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
                min_valid_sources: 2,
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
                    && now.saturating_sub(source.timestamp) <= source.staleness_limit_seconds
                    && source.price > 0
                {
                    sources_valid += 1;
                }
            }
        }

        env.events().publish(
            (Symbol::new(&env, "NAV_PAUSE"), sleeve_id.clone()),
            NavPauseData {
                reason,
                sleeve: sleeve_id,
                sources_valid,
                sources_total: source_ids.len(),
                deviation_bps: 0,
                timestamp: now,
            },
        );
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
                    && now.saturating_sub(source.timestamp) <= source.staleness_limit_seconds
                    && source.price > 0
                {
                    sources_valid += 1;
                }
            }
        }

        env.events().publish(
            (Symbol::new(&env, "NAV_RESUME"), sleeve_id.clone()),
            NavResumeData {
                sleeve: sleeve_id,
                sources_valid,
                timestamp: now,
            },
        );
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
        env.events().publish(
            (Symbol::new(&env, "PAUSE_AUTHORITY_SET"), old),
            new_authority,
        );
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

    fn get_sources(env: Env) -> Vec<OracleSource> {
        let mut sources: Vec<OracleSource> = Vec::new(&env);
        let assets = Self::assets(env.clone());
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
