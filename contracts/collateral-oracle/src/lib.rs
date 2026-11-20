#![no_std]
//! Constella Collateral Passport price feed (audit L1, decision D-N).
//!
//! A SEP-40 oracle for LENDERS (e.g. a Blend pool). Ordinary assets are
//! quoted by forwarding to the Constella NAV oracle. A LISTED vault's share
//! token is quoted at the vault's CONSERVATIVE NAV per share (minimum of each
//! asset's valid sources; booked-or-real holdings, so a donation cannot
//! inflate it), converted into this feed's base.
//! A separate contract because the NAV oracle cannot ask the vault: the vault
//! asks the NAV oracle for prices, and oracle -> vault -> oracle would be
//! contract re-entry, which Soroban forbids.
//!
//! Halt-on-invalid: when the vault cannot value itself (a funded sleeve is
//! unpriced), the share has NO price here - a lender halts rather than lend or
//! liquidate on a guess. Listing is permanent: a delisting would strip the
//! price from every open loan, so the vault's own validity is the only switch.

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address, Env,
    Symbol, Vec,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Asset {
    Stellar(Address),
    Other(Symbol),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}

/// Mirror of `constella_vault_strategy::VaultConfig` (decoded by field name).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultConfigRef {
    pub admin: Address,
    pub pause_authority: Address,
    pub numeraire: Address,
    pub policy_contract: Address,
    pub oracle_contract: Address,
    pub registry_contract: Address,
    pub treasury: Address,
    pub performance_fee_bps: u32,
    pub tvl_cap: i128,
    pub per_user_cap: i128,
}

#[contractclient(name = "NavOracleClient")]
pub trait NavOracleRef {
    fn lastprice(env: Env, asset: Asset) -> Option<PriceData>;
    fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData>;
    fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>>;
    fn base(env: Env) -> Asset;
    fn decimals(env: Env) -> u32;
    fn resolution(env: Env) -> u32;
}

#[contractclient(name = "VaultClient")]
pub trait VaultRef {
    fn get_nav_conservative(env: Env) -> Option<PriceData>;
    fn get_config(env: Env) -> VaultConfigRef;
    fn decimals(env: Env) -> u32;
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CollateralOracleError {
    AlreadyListed = 1,
    /// The vault does not use this feed's NAV oracle, or its share decimals
    /// differ from the oracle's (no silent rescaling).
    IncompatibleVault = 2,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    NavOracle,
    /// vault share token -> listed (permanent)
    Listed(Address),
}

#[contractevent(topics = ["VAULT_LISTED"], data_format = "single-value")]
pub struct VaultListed {
    #[topic]
    pub vault: Address,
}

const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 29 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 30 * DAY_LEDGERS;

fn nav_oracle(env: &Env) -> NavOracleClient<'_> {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
    let id: Address = env.storage().instance().get(&DataKey::NavOracle).unwrap();
    NavOracleClient::new(env, &id)
}

fn listed(env: &Env, asset: &Address) -> bool {
    let key = DataKey::Listed(asset.clone());
    let yes = env.storage().persistent().has(&key);
    if yes {
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
    }
    yes
}

/// The share's conservative NAV in this feed's base, or None (halt).
fn share_price(env: &Env, vault: &Address) -> Option<PriceData> {
    let oracle = nav_oracle(env);
    let v = VaultClient::new(env, vault);
    let nav = v.get_nav_conservative()?;
    if nav.price <= 0 {
        return None;
    }
    let numeraire = v.get_config().numeraire;
    if oracle.base() == Asset::Stellar(numeraire.clone()) {
        return Some(nav);
    }
    // A vault in another currency: convert at that currency's conservative price.
    let fx = oracle.lastprice(&Asset::Stellar(numeraire))?;
    if fx.price <= 0 {
        return None;
    }
    let price = nav.price.checked_mul(fx.price)? / 10i128.checked_pow(oracle.decimals())?;
    Some(PriceData { price, timestamp: nav.timestamp.min(fx.timestamp) })
}

#[contract]
pub struct CollateralOracle;

#[contractimpl]
impl CollateralOracle {
    pub fn __constructor(env: Env, admin: Address, nav_oracle: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::NavOracle, &nav_oracle);
    }

    /// List a vault's share token (admin; permanent). The vault must price its
    /// sleeves with this feed's NAV oracle and use the oracle's decimals.
    pub fn list_vault(env: Env, vault: Address) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        let key = DataKey::Listed(vault.clone());
        if env.storage().persistent().has(&key) {
            panic_with_error!(&env, CollateralOracleError::AlreadyListed);
        }
        let oracle = nav_oracle(&env);
        let v = VaultClient::new(&env, &vault);
        if v.get_config().oracle_contract != oracle.address || v.decimals() != oracle.decimals() {
            panic_with_error!(&env, CollateralOracleError::IncompatibleVault);
        }
        env.storage().persistent().set(&key, &true);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
        VaultListed { vault }.publish(&env);
    }

    pub fn is_listed(env: Env, vault: Address) -> bool {
        listed(&env, &vault)
    }

    // ------------------------------------------------------------ SEP-40
    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        if let Asset::Stellar(a) = &asset {
            if listed(&env, a) {
                return share_price(&env, a);
            }
        }
        nav_oracle(&env).lastprice(&asset)
    }

    /// Historical share prices are not served (lenders read `lastprice`);
    /// other assets forward to the NAV oracle.
    pub fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData> {
        if let Asset::Stellar(a) = &asset {
            if listed(&env, a) {
                return None;
            }
        }
        nav_oracle(&env).price(&asset, &timestamp)
    }

    /// For a share: at most the current price (no history is invented).
    pub fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
        if let Asset::Stellar(a) = &asset {
            if listed(&env, a) {
                if records == 0 {
                    return None;
                }
                let p = share_price(&env, a)?;
                return Some(Vec::from_array(&env, [p]));
            }
        }
        nav_oracle(&env).prices(&asset, &records)
    }

    pub fn base(env: Env) -> Asset {
        nav_oracle(&env).base()
    }

    pub fn decimals(env: Env) -> u32 {
        nav_oracle(&env).decimals()
    }

    pub fn resolution(env: Env) -> u32 {
        nav_oracle(&env).resolution()
    }

    pub fn assets(env: Env) -> Vec<Asset> {
        Vec::new(&env) // listed shares are discoverable through VAULT_LISTED events
    }
}
