#![no_std]
#![allow(clippy::too_many_arguments)]

use constella_registry::AssetRegistryClient;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, BytesN, Env,
    FromVal, Symbol, SymbolStr, Vec,
};

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
    pub admin: Address,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum TradeRoute {
    Issuer = 0,
    Soroswap = 1,
    Aquarius = 2,
    Sdex = 3,
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
    PolicyConfig(Address),
    SleeveTargets(Address),
}

#[cfg(test)]
mod test;

pub trait PolicyContractTrait {
    /// Initialize the policy contract with a governance admin
    fn initialize(env: Env, admin: Address);

    /// Register a signed policy hash and configuration for a vault
    fn register_policy(
        env: Env,
        vault: Address,
        policy_hash: BytesN<32>,
        registry_address: Address,
        min_liquidity_sleeve_bps: u32,
        max_single_asset_bps: u32,
        max_issuer_bps: u32,
        max_slippage_bps: u32,
        rebalance_band_bps: u32,
        sleeves: Vec<SleeveTarget>,
    );

    /// Update a vault's policy hash upon user re-signing
    fn update_policy(
        env: Env,
        vault: Address,
        new_hash: BytesN<32>,
        new_sleeves: Vec<SleeveTarget>,
    );

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

    /// Get admin address
    fn get_admin(env: Env) -> Address;
}

#[contract]
pub struct PolicyContract;

fn get_issuer_prefix(env: &Env, asset_id: &Symbol) -> Symbol {
    let sym_str: SymbolStr = SymbolStr::from_val(env, &asset_id.to_symbol_val());
    let bytes: &[u8] = sym_str.as_ref();
    let mut idx = 0;
    let mut found_sep = false;
    while idx < bytes.len() {
        if bytes[idx] == b'_' {
            found_sep = true;
            break;
        }
        idx += 1;
    }
    if found_sep && idx > 0 {
        if let Ok(issuer_str) = core::str::from_utf8(&bytes[..idx]) {
            return Symbol::new(env, issuer_str);
        }
    }
    asset_id.clone()
}

fn is_cash_sleeve(env: &Env, st: &Symbol) -> bool {
    let sym_str: SymbolStr = SymbolStr::from_val(env, &st.to_symbol_val());
    let bytes: &[u8] = sym_str.as_ref();
    bytes.eq_ignore_ascii_case(b"cash")
        || bytes.eq_ignore_ascii_case(b"cash_stable")
        || bytes.eq_ignore_ascii_case(b"cash_sleeve")
}

#[contractimpl]
impl PolicyContractTrait for PolicyContract {
    fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, PolicyError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    fn register_policy(
        env: Env,
        vault: Address,
        policy_hash: BytesN<32>,
        registry_address: Address,
        min_liquidity_sleeve_bps: u32,
        max_single_asset_bps: u32,
        max_issuer_bps: u32,
        max_slippage_bps: u32,
        rebalance_band_bps: u32,
        sleeves: Vec<SleeveTarget>,
    ) {
        let admin = Self::get_admin(env.clone());
        admin.require_auth();

        let existing_version = env
            .storage()
            .instance()
            .get::<_, PolicyConfig>(&DataKey::PolicyConfig(vault.clone()))
            .map(|c| c.version)
            .unwrap_or(0);

        let config = PolicyConfig {
            vault: vault.clone(),
            policy_hash: policy_hash.clone(),
            version: existing_version + 1,
            registry_address,
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            rebalance_band_bps,
            is_active: true,
            admin: admin.clone(),
        };

        env.storage()
            .instance()
            .set(&DataKey::PolicyConfig(vault.clone()), &config);
        env.storage()
            .instance()
            .set(&DataKey::SleeveTargets(vault.clone()), &sleeves);

        env.events().publish(
            (Symbol::new(&env, "POLICY_REGISTERED"), vault),
            policy_hash,
        );
    }

    fn update_policy(
        env: Env,
        vault: Address,
        new_hash: BytesN<32>,
        new_sleeves: Vec<SleeveTarget>,
    ) {
        let admin = Self::get_admin(env.clone());
        admin.require_auth();

        let mut config: PolicyConfig = match env
            .storage()
            .instance()
            .get(&DataKey::PolicyConfig(vault.clone()))
        {
            Some(cfg) => cfg,
            None => panic_with_error!(&env, PolicyError::PolicyNotFound),
        };

        config.policy_hash = new_hash.clone();
        config.version += 1;

        env.storage()
            .instance()
            .set(&DataKey::PolicyConfig(vault.clone()), &config);
        env.storage()
            .instance()
            .set(&DataKey::SleeveTargets(vault.clone()), &new_sleeves);

        env.events().publish(
            (Symbol::new(&env, "POLICY_UPDATED"), vault),
            new_hash,
        );
    }

    fn get_policy_hash(env: Env, vault: Address) -> BytesN<32> {
        match env
            .storage()
            .instance()
            .get::<_, PolicyConfig>(&DataKey::PolicyConfig(vault))
        {
            Some(config) => config.policy_hash,
            None => panic_with_error!(&env, PolicyError::PolicyNotFound),
        }
    }

    fn get_policy_config(env: Env, vault: Address) -> Option<PolicyConfig> {
        env.storage().instance().get(&DataKey::PolicyConfig(vault))
    }

    fn get_sleeve_targets(env: Env, vault: Address) -> Vec<SleeveTarget> {
        env.storage()
            .instance()
            .get(&DataKey::SleeveTargets(vault))
            .unwrap_or_else(|| Vec::new(&env))
    }

    fn validate_rebalance(
        env: Env,
        vault: Address,
        proposal: RebalanceProposal,
    ) -> ValidationResult {
        let config: PolicyConfig = match env
            .storage()
            .instance()
            .get(&DataKey::PolicyConfig(vault.clone()))
        {
            Some(c) => c,
            None => {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::PolicyNotFound as u32,
                    message: Symbol::new(&env, "POLICY_NOT_FOUND"),
                };
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
                return res;
            }
        };

        if !config.is_active {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::PolicyNotFound as u32,
                message: Symbol::new(&env, "POLICY_INACTIVE"),
            };
            env.events().publish(
                (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                (res.is_valid, res.code),
            );
            return res;
        }

        // 1. Verify proposal deadline >= env.ledger().timestamp()
        if proposal.deadline < env.ledger().timestamp() {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::DeadlineExceeded as u32,
                message: Symbol::new(&env, "DEADLINE_EXCEEDED"),
            };
            env.events().publish(
                (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                (res.is_valid, res.code),
            );
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
            env.events().publish(
                (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                (res.is_valid, res.code),
            );
            return res;
        }

        // 3. Verify each proposed weight is within [min_band_bps, max_band_bps] for declared sleeves
        let sleeves: Vec<SleeveTarget> = env
            .storage()
            .instance()
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
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
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
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
                return res;
            }
        }

        // 4. Verify no single asset weight > max_single_asset_bps
        for j in 0..proposal.weights_after.len() {
            let (_, weight) = proposal.weights_after.get(j).unwrap();
            if weight > config.max_single_asset_bps {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::SingleAssetCapExceeded as u32,
                    message: Symbol::new(&env, "SINGLE_ASSET_CAP"),
                };
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
                return res;
            }
        }

        // 5. Verify issuer caps: no single issuer exceeds max_issuer_bps
        for j in 0..proposal.weights_after.len() {
            let (asset_id, _) = proposal.weights_after.get(j).unwrap();
            let issuer = get_issuer_prefix(&env, &asset_id);
            let mut total_issuer_weight = 0u32;
            for k in 0..proposal.weights_after.len() {
                let (other_asset_id, other_weight) = proposal.weights_after.get(k).unwrap();
                let other_issuer = get_issuer_prefix(&env, &other_asset_id);
                if issuer == other_issuer {
                    total_issuer_weight = total_issuer_weight.saturating_add(other_weight);
                }
            }
            if total_issuer_weight > config.max_issuer_bps {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::IssuerCapExceeded as u32,
                    message: Symbol::new(&env, "ISSUER_CAP_EXCEEDED"),
                };
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
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
            env.events().publish(
                (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                (res.is_valid, res.code),
            );
            return res;
        }

        // 7. Verify each trade operation slippage: if amount_in > 0, calculate if min_amount_out respects max_slippage_bps
        for i in 0..proposal.operations.len() {
            let op = proposal.operations.get(i).unwrap();
            if op.amount_in <= 0 {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::SlippageLimitExceeded as u32,
                    message: Symbol::new(&env, "INVALID_TRADE_AMOUNT"),
                };
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
                return res;
            }
            let min_required_out = op
                .amount_in
                .checked_mul((10000 - config.max_slippage_bps) as i128)
                .unwrap_or(0)
                / 10000;
            if op.min_amount_out < min_required_out {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::SlippageLimitExceeded as u32,
                    message: Symbol::new(&env, "SLIPPAGE_EXCEEDED"),
                };
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
                return res;
            }
        }

        // 8. Cross-contract call to constella-registry (is_registered for asset_in and asset_out)
        let registry_client = AssetRegistryClient::new(&env, &config.registry_address);
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
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
                return res;
            }
        }

        // All checks passed!
        let res = ValidationResult {
            is_valid: true,
            code: 0,
            message: Symbol::new(&env, "VALID"),
        };
        env.events().publish(
            (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
            (res.is_valid, res.code),
        );
        res
    }

    fn get_admin(env: Env) -> Address {
        match env.storage().instance().get(&DataKey::Admin) {
            Some(admin) => admin,
            None => panic_with_error!(&env, PolicyError::NotInitialized),
        }
    }
}
