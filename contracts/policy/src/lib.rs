#![no_std]
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error, token, Address, BytesN,
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

#[contractclient(name = "OraclePriceClient")]
pub trait OraclePriceInterface {
    fn asset_price(env: Env, asset: Address) -> Option<OraclePriceRef>;
}

#[contractclient(name = "AssetRegistryClient")]
pub trait AssetRegistryInterface {
    fn is_registered(env: Env, contract_address: Address) -> bool;
    fn get_asset(env: Env, asset_id: Symbol) -> Option<RegistryAssetEntry>;
    fn get_asset_by_address(env: Env, contract_address: Address) -> Option<RegistryAssetEntry>;
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
}

#[cfg(test)]
mod test;

pub trait PolicyContractTrait {
    /// Initialize with a governance admin and the NAV oracle used to value
    /// both legs of every trade (audit P1). The oracle is fixed for the life
    /// of this contract: there is deliberately no setter.
    fn initialize(env: Env, admin: Address, oracle: Address);

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

    /// Amend a vault's policy. Requires a fresh signature from the stored
    /// policy owner; the protocol admin cannot amend (audit P3).
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

fn is_cash_sleeve(env: &Env, st: &Symbol) -> bool {
    let sym_str: SymbolStr = SymbolStr::from_val(env, &st.to_symbol_val());
    let bytes: &[u8] = sym_str.as_ref();
    bytes.eq_ignore_ascii_case(b"cash")
        || bytes.eq_ignore_ascii_case(b"cash_stable")
        || bytes.eq_ignore_ascii_case(b"cash_sleeve")
}

fn validate_terms(
    env: &Env,
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
impl PolicyContractTrait for PolicyContract {
    fn initialize(env: Env, admin: Address, oracle: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, PolicyError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
    }

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
            .instance()
            .has(&DataKey::PolicyConfig(vault.clone()))
        {
            panic_with_error!(&env, PolicyError::PolicyAlreadyRegistered);
        }

        validate_terms(
            &env,
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
            .instance()
            .set(&DataKey::PolicyConfig(vault.clone()), &config);
        env.storage()
            .instance()
            .set(&DataKey::SleeveTargets(vault.clone()), &sleeves);

        env.events().publish(
            (Symbol::new(&env, "POLICY_REGISTERED"), vault),
            (policy_hash, owner, terms_hash),
        );
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
        let mut config: PolicyConfig = match env
            .storage()
            .instance()
            .get(&DataKey::PolicyConfig(vault.clone()))
        {
            Some(cfg) => cfg,
            None => panic_with_error!(&env, PolicyError::PolicyNotFound),
        };
        // Only the policy owner can amend — never the protocol admin (audit P3).
        config.owner.require_auth();

        validate_terms(
            &env,
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            &new_sleeves,
        );

        config.policy_hash = new_hash.clone();
        config.version += 1;
        config.min_liquidity_sleeve_bps = min_liquidity_sleeve_bps;
        config.max_single_asset_bps = max_single_asset_bps;
        config.max_issuer_bps = max_issuer_bps;
        config.max_slippage_bps = max_slippage_bps;
        config.rebalance_band_bps = rebalance_band_bps;
        config.terms_hash = compute_terms_hash(
            &env,
            &config.registry_address,
            min_liquidity_sleeve_bps,
            max_single_asset_bps,
            max_issuer_bps,
            max_slippage_bps,
            rebalance_band_bps,
            &new_sleeves,
        );

        env.storage()
            .instance()
            .set(&DataKey::PolicyConfig(vault.clone()), &config);
        env.storage()
            .instance()
            .set(&DataKey::SleeveTargets(vault.clone()), &new_sleeves);

        env.events().publish(
            (Symbol::new(&env, "POLICY_UPDATED"), vault),
            (new_hash, config.version, config.terms_hash),
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

        // Verify proposal is destined for the queried vault
        if proposal.vault != vault {
            let res = ValidationResult {
                is_valid: false,
                code: PolicyError::Unauthorized as u32,
                message: Symbol::new(&env, "VAULT_MISMATCH"),
            };
            env.events().publish(
                (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                (res.is_valid, res.code),
            );
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
                    env.events().publish(
                        (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                        (res.is_valid, res.code),
                    );
                    return res;
                }
            }
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
                    env.events().publish(
                        (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                        (res.is_valid, res.code),
                    );
                    return res;
                }
            };
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

        // 7. Structural checks on every trade operation
        for i in 0..proposal.operations.len() {
            let op = proposal.operations.get(i).unwrap();
            if op.amount_in <= 0 || op.min_amount_out <= 0 {
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
            if op.asset_in == op.asset_out {
                let res = ValidationResult {
                    is_valid: false,
                    code: PolicyError::SlippageLimitExceeded as u32,
                    message: Symbol::new(&env, "IDENTICAL_ASSETS"),
                };
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
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
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
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
                env.events().publish(
                    (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                    (res.is_valid, res.code),
                );
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
                    env.events().publish(
                        (Symbol::new(&env, "REBALANCE_VALIDATED"), vault),
                        (res.is_valid, res.code),
                    );
                    return res;
                }
            };
            let dec_in = token::Client::new(&env, &op.asset_in).decimals();
            let dec_out = token::Client::new(&env, &op.asset_out).decimals();
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
