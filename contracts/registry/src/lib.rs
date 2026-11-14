#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, Symbol, Vec,
};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SleeveType {
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
pub struct AssetEntry {
    pub contract_address: Address,
    pub asset_id: Symbol,
    pub sleeve_type: SleeveType,
    pub liquidity_tier: u32, // 0=instant, 1=24h, 2=1week, 3=illiquid
    pub permissionless: bool,
    pub oracle_sources: Vec<Symbol>,
    pub eligible_jurisdictions: Vec<Symbol>, // empty = all
    pub blocked_jurisdictions: Vec<Symbol>,
    pub investor_classes_required: Vec<Symbol>, // empty = all
    pub is_active: bool,
    pub added_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Asset(Symbol),
    AssetByAddress(Address),
    AssetList,
}

pub trait AssetRegistryTrait {
    /// Initialize the registry with an authorized admin address (multisig)
    fn initialize(env: Env, admin: Address);

    /// Add an approved asset to the closed universe (admin only)
    fn add_asset(env: Env, entry: AssetEntry);

    /// Deactivate an asset, disabling fresh allocations (admin only)
    fn deactivate_asset(env: Env, asset_id: Symbol);

    /// Query an asset entry by its symbolic identifier
    fn get_asset(env: Env, asset_id: Symbol) -> Option<AssetEntry>;

    /// Query an asset entry by its Soroban contract address
    fn get_asset_by_address(env: Env, contract_address: Address) -> Option<AssetEntry>;

    /// Check if a contract address is currently an active registered asset
    fn is_registered(env: Env, contract_address: Address) -> bool;

    /// Return all registered asset entries
    fn list_assets(env: Env) -> Vec<AssetEntry>;

    /// Return the current registry admin address
    fn get_admin(env: Env) -> Address;
}

#[contract]
pub struct AssetRegistry;

#[contractimpl]
impl AssetRegistryTrait for AssetRegistry {
    fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    fn add_asset(env: Env, entry: AssetEntry) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let mut asset_list: Vec<Symbol> = env
            .storage()
            .instance()
            .get(&DataKey::AssetList)
            .unwrap_or_else(|| Vec::new(&env));

        // If replacing an existing entry with a different address, clean old address mapping
        if let Some(old_entry) = env
            .storage()
            .instance()
            .get::<_, AssetEntry>(&DataKey::Asset(entry.asset_id.clone()))
        {
            if old_entry.contract_address != entry.contract_address {
                env.storage()
                    .instance()
                    .remove(&DataKey::AssetByAddress(old_entry.contract_address));
            }
        } else {
            // New asset ID: append to asset list
            asset_list.push_back(entry.asset_id.clone());
            env.storage().instance().set(&DataKey::AssetList, &asset_list);
        }

        env.storage()
            .instance()
            .set(&DataKey::Asset(entry.asset_id.clone()), &entry);
        env.storage().instance().set(
            &DataKey::AssetByAddress(entry.contract_address.clone()),
            &entry.asset_id,
        );

        env.events().publish(
            (Symbol::new(&env, "ASSET_ADDED"), entry.asset_id.clone()),
            entry.contract_address.clone(),
        );
    }

    fn deactivate_asset(env: Env, asset_id: Symbol) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let mut entry: AssetEntry = env
            .storage()
            .instance()
            .get(&DataKey::Asset(asset_id.clone()))
            .expect("asset not found");

        entry.is_active = false;
        env.storage()
            .instance()
            .set(&DataKey::Asset(asset_id.clone()), &entry);

        env.events().publish(
            (Symbol::new(&env, "ASSET_DEACTIVATED"), asset_id.clone()),
            entry.contract_address.clone(),
        );
    }

    fn get_asset(env: Env, asset_id: Symbol) -> Option<AssetEntry> {
        env.storage().instance().get(&DataKey::Asset(asset_id))
    }

    fn get_asset_by_address(env: Env, contract_address: Address) -> Option<AssetEntry> {
        let asset_id: Symbol = env
            .storage()
            .instance()
            .get(&DataKey::AssetByAddress(contract_address))?;
        env.storage().instance().get(&DataKey::Asset(asset_id))
    }

    fn is_registered(env: Env, contract_address: Address) -> bool {
        match Self::get_asset_by_address(env, contract_address) {
            Some(entry) => entry.is_active,
            None => false,
        }
    }

    fn list_assets(env: Env) -> Vec<AssetEntry> {
        let symbols: Vec<Symbol> = env
            .storage()
            .instance()
            .get(&DataKey::AssetList)
            .unwrap_or_else(|| Vec::new(&env));

        let mut result = Vec::new(&env);
        for sym in symbols.iter() {
            if let Some(entry) = env.storage().instance().get(&DataKey::Asset(sym)) {
                result.push_back(entry);
            }
        }
        result
    }

    fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized")
    }
}

#[cfg(test)]
mod test;
