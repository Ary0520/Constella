#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, Address, Env, Vec,
};

// Events (audit W4.1b: typed #[contractevent]s, byte-identical to the former
// Events::publish encodings; pinned by golden/events.txt).

/// Topics: [SPONSOR_INITIALIZED, admin]. Data: daily budget (stroops).
#[contractevent(topics = ["SPONSOR_INITIALIZED"], data_format = "single-value")]
pub struct SponsorInitialized {
    #[topic]
    pub admin: Address,
    pub daily_budget_stroops: u64,
}

/// Topics: [CONTRACT_WHITELISTED, admin]. Data: target contract.
#[contractevent(topics = ["CONTRACT_WHITELISTED"], data_format = "single-value")]
pub struct ContractWhitelisted {
    #[topic]
    pub admin: Address,
    pub target: Address,
}

/// Topics: [CONTRACT_UNWHITELISTED, admin]. Data: target contract.
#[contractevent(topics = ["CONTRACT_UNWHITELISTED"], data_format = "single-value")]
pub struct ContractUnwhitelisted {
    #[topic]
    pub admin: Address,
    pub target: Address,
}

/// Topics: [SPONSORED_OP, user, target_contract]. Data: fee spent (stroops).
#[contractevent(topics = ["SPONSORED_OP"], data_format = "single-value")]
pub struct SponsoredOp {
    #[topic]
    pub user: Address,
    #[topic]
    pub target_contract: Address,
    pub fee_spent_stroops: u64,
}

/// Topics: [LIMITS_UPDATED, admin]. Data: daily budget (stroops).
#[contractevent(topics = ["LIMITS_UPDATED"], data_format = "single-value")]
pub struct LimitsUpdated {
    #[topic]
    pub admin: Address,
    pub daily_budget_stroops: u64,
}

/// Topics: [SPONSOR_ADMIN_TRANSFERRED, old_admin]. Data: new admin.
#[contractevent(topics = ["SPONSOR_ADMIN_TRANSFERRED"], data_format = "single-value")]
pub struct SponsorAdminTransferred {
    #[topic]
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Topics: [SPONSOR_PAUSED, admin]. Data: void.
/// Decision D-I: a relayer account joined / left the pool.
#[contractevent(topics = ["RELAYER_ADDED"], data_format = "single-value")]
pub struct RelayerAdded {
    #[topic]
    pub admin: Address,
    pub relayer: Address,
}

#[contractevent(topics = ["RELAYER_REMOVED"], data_format = "single-value")]
pub struct RelayerRemoved {
    #[topic]
    pub admin: Address,
    pub relayer: Address,
}

/// Decision D-I: maximum relayer accounts in the pool (bounded, so the set can
/// live in instance storage without the F1a growth problem).
pub const MAX_RELAYERS: u32 = 16;

#[contractevent(topics = ["SPONSOR_PAUSED"], data_format = "single-value")]
pub struct SponsorPaused {
    #[topic]
    pub admin: Address,
}

/// Topics: [SPONSOR_UNPAUSED, admin]. Data: void.
#[contractevent(topics = ["SPONSOR_UNPAUSED"], data_format = "single-value")]
pub struct SponsorUnpaused {
    #[topic]
    pub admin: Address,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SponsorError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    SponsorPaused = 4,
    TargetNotWhitelisted = 5,
    HourlyLimitExceeded = 6,
    DailyLimitExceeded = 7,
    DailyBudgetExceeded = 8,
    ContractAlreadyWhitelisted = 9,
    ContractNotWhitelisted = 10,
    InvalidConfig = 11,
    /// No relayer has been designated yet (audit F1b).
    RelayerNotSet = 12,
    /// The address is already in the relayer set (decision D-I).
    RelayerExists = 13,
    /// The relayer set is full (MAX_RELAYERS).
    TooManyRelayers = 14,
    /// The address is not in the relayer set.
    NotARelayer = 15,
    /// Step 3: the factory is already trusted.
    FactoryAlreadyTrusted = 16,
    /// Step 3: the factory is not trusted.
    FactoryNotTrusted = 17,
    /// Step 3: the trusted-factory list is full (MAX_TRUSTED_FACTORIES).
    TooManyFactories = 18,
}

/// Step 3: at most this many trusted vault factories (each adds one
/// cross-contract read to `is_whitelisted`).
pub const MAX_TRUSTED_FACTORIES: u32 = 4;

/// The one function of `constella-vault-factory` the sponsor reads: true only
/// for contracts that factory deployed from its approved vault code.
#[contractclient(name = "VaultFactoryClient")]
pub trait VaultFactoryInterface {
    fn is_vault(env: Env, vault: Address) -> bool;
}

/// Topics: [FACTORY_TRUSTED, admin]. Data: factory.
#[contractevent(topics = ["FACTORY_TRUSTED"], data_format = "single-value")]
pub struct FactoryTrusted {
    #[topic]
    pub admin: Address,
    pub factory: Address,
}

/// Topics: [FACTORY_UNTRUSTED, admin]. Data: factory.
#[contractevent(topics = ["FACTORY_UNTRUSTED"], data_format = "single-value")]
pub struct FactoryUntrusted {
    #[topic]
    pub admin: Address,
    pub factory: Address,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct SponsorConfig {
    pub admin: Address,
    pub max_ops_per_hour: u32,
    pub max_ops_per_day: u32,
    pub daily_budget_stroops: u64,
    pub is_paused: bool,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct UserSponsorStats {
    pub hourly_ops: u32,
    pub daily_ops: u32,
    pub last_op_timestamp: u64,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct GlobalSponsorStats {
    pub total_ops_today: u32,
    pub total_fees_today_stroops: u64,
    pub current_day_epoch: u64,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Config,
    Whitelisted(Address),
    WhitelistList,
    UserStats(Address),
    GlobalStats,
    /// Audit F1b / decision D-I: the bounded set of relayer accounts allowed
    /// to record sponsored operations (a pool, so throughput scales).
    Relayers,
    /// Step 3: vault factories whose `is_vault` contracts are sponsorable.
    TrustedFactories,
}

/// Audit F1a: TTLs (in ledgers, ~5 s each). Per-user stats live in their own
/// persistent entries; the instance (config, global stats, whitelist - all
/// bounded) is extended on use so the sponsor never archives while in use.
const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 29 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 30 * DAY_LEDGERS;

fn extend_instance(env: &Env) {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

#[contract]
pub struct FeeSponsorContract;

#[contractimpl]
impl FeeSponsorContract {
    /// Configure guardrails at deployment (atomic, so no one can claim an
    /// uninitialized sponsor - audit M3). There is no `initialize` entrypoint.
    pub fn __constructor(
        env: Env,
        admin: Address,
        max_ops_per_hour: u32,
        max_ops_per_day: u32,
        daily_budget_stroops: u64,
    ) {
        if max_ops_per_hour == 0
            || max_ops_per_day < max_ops_per_hour
            || daily_budget_stroops == 0
        {
            soroban_sdk::panic_with_error!(&env, SponsorError::InvalidConfig);
        }

        let config = SponsorConfig {
            admin: admin.clone(),
            max_ops_per_hour,
            max_ops_per_day,
            daily_budget_stroops,
            is_paused: false,
        };

        let now = env.ledger().timestamp();
        let global_stats = GlobalSponsorStats {
            total_ops_today: 0,
            total_fees_today_stroops: 0,
            current_day_epoch: now / 86400,
        };

        env.storage().instance().set(&DataKey::Config, &config);
        env.storage()
            .instance()
            .set(&DataKey::GlobalStats, &global_stats);
        let empty_list: Vec<Address> = Vec::new(&env);
        env.storage()
            .instance()
            .set(&DataKey::WhitelistList, &empty_list);

        SponsorInitialized { admin, daily_budget_stroops }.publish(&env);

    }

    /// Whitelist an approved Constella contract (e.g. strategy vault, policy) for sponsored gas
    pub fn whitelist_contract(env: Env, target: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;

        if env
            .storage()
            .instance()
            .has(&DataKey::Whitelisted(target.clone()))
        {
            return Err(SponsorError::ContractAlreadyWhitelisted);
        }

        env.storage()
            .instance()
            .set(&DataKey::Whitelisted(target.clone()), &true);

        let mut list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::WhitelistList)
            .unwrap_or_else(|| Vec::new(&env));
        list.push_back(target.clone());
        env.storage().instance().set(&DataKey::WhitelistList, &list);

        ContractWhitelisted { admin: config.admin, target }.publish(&env);

        Ok(())
    }

    /// Remove a contract from the sponsorship whitelist (storage rent hygiene enforced)
    pub fn remove_contract(env: Env, target: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;

        if !env
            .storage()
            .instance()
            .has(&DataKey::Whitelisted(target.clone()))
        {
            return Err(SponsorError::ContractNotWhitelisted);
        }

        // Storage rent hygiene: completely remove boolean flag
        env.storage()
            .instance()
            .remove(&DataKey::Whitelisted(target.clone()));

        let list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::WhitelistList)
            .unwrap_or_else(|| Vec::new(&env));

        let mut new_list = Vec::new(&env);
        for item in list.iter() {
            if item != target {
                new_list.push_back(item);
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::WhitelistList, &new_list);

        ContractUnwhitelisted { admin: config.admin, target }.publish(&env);

        Ok(())
    }

    /// Check if target contract is approved for fee sponsorship
    pub fn is_whitelisted(env: Env, target: Address) -> bool {
        if env.storage().instance().get(&DataKey::Whitelisted(target.clone())).unwrap_or(false) {
            return true;
        }
        // Step 3: a vault proven by a trusted Constella factory.
        let factories: Vec<Address> = env.storage().instance().get(&DataKey::TrustedFactories).unwrap_or_else(|| Vec::new(&env));
        factories.iter().any(|f| VaultFactoryClient::new(&env, &f).is_vault(&target))
    }

    /// Step 3: trust a Constella vault factory - every contract it proves
    /// (`is_vault`) becomes sponsorable. Admin only; bounded.
    pub fn trust_factory(env: Env, factory: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;
        let mut list: Vec<Address> = env.storage().instance().get(&DataKey::TrustedFactories).unwrap_or_else(|| Vec::new(&env));
        if list.contains(&factory) {
            return Err(SponsorError::FactoryAlreadyTrusted);
        }
        if list.len() >= MAX_TRUSTED_FACTORIES {
            return Err(SponsorError::TooManyFactories);
        }
        list.push_back(factory.clone());
        env.storage().instance().set(&DataKey::TrustedFactories, &list);
        extend_instance(&env);
        FactoryTrusted { admin: config.admin, factory }.publish(&env);
        Ok(())
    }

    /// Step 3: stop trusting a factory (its vaults lose sponsorship at once).
    pub fn untrust_factory(env: Env, factory: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;
        let list: Vec<Address> = env.storage().instance().get(&DataKey::TrustedFactories).unwrap_or_else(|| Vec::new(&env));
        let Some(i) = list.first_index_of(&factory) else { return Err(SponsorError::FactoryNotTrusted) };
        let mut list = list;
        list.remove(i);
        env.storage().instance().set(&DataKey::TrustedFactories, &list);
        extend_instance(&env);
        FactoryUntrusted { admin: config.admin, factory }.publish(&env);
        Ok(())
    }

    /// Step 3: the trusted vault factories.
    pub fn get_trusted_factories(env: Env) -> Vec<Address> {
        env.storage().instance().get(&DataKey::TrustedFactories).unwrap_or_else(|| Vec::new(&env))
    }

    /// Validate whether an operation is eligible for fee sponsorship (stateless check against limits)
    pub fn validate_sponsorship(
        env: Env,
        user: Address,
        target_contract: Address,
        fee_bid_stroops: u64,
    ) -> Result<(), SponsorError> {
        let config: SponsorConfig = match env.storage().instance().get(&DataKey::Config) {
            Some(c) => c,
            None => return Err(SponsorError::NotInitialized),
        };

        if config.is_paused {
            return Err(SponsorError::SponsorPaused);
        }

        if !Self::is_whitelisted(env.clone(), target_contract) {
            return Err(SponsorError::TargetNotWhitelisted);
        }

        // Global daily budget validation
        let global_stats = Self::get_global_stats(env.clone());
        let projected_global_fees = global_stats
            .total_fees_today_stroops
            .saturating_add(fee_bid_stroops);

        if projected_global_fees > config.daily_budget_stroops {
            return Err(SponsorError::DailyBudgetExceeded);
        }

        // Per-user quota validation (hourly & daily)
        let user_stats = Self::get_user_stats(env.clone(), user);

        if user_stats.hourly_ops.saturating_add(1) > config.max_ops_per_hour {
            return Err(SponsorError::HourlyLimitExceeded);
        }

        if user_stats.daily_ops.saturating_add(1) > config.max_ops_per_day {
            return Err(SponsorError::DailyLimitExceeded);
        }

        Ok(())
    }

    /// Audit F1b / D-I: add a relayer account to the pool - a service key
    /// that pays fees and records sponsored operations. Admin only. Least
    /// privilege: a relayer can only record (within the limits); it cannot
    /// change limits, the whitelist, the admin, the pool, or unpause.
    pub fn add_relayer(env: Env, relayer: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;
        let mut set = Self::get_relayers(env.clone());
        if set.contains(&relayer) {
            return Err(SponsorError::RelayerExists);
        }
        if set.len() >= MAX_RELAYERS {
            return Err(SponsorError::TooManyRelayers);
        }
        set.push_back(relayer.clone());
        env.storage().instance().set(&DataKey::Relayers, &set);
        extend_instance(&env);
        RelayerAdded { admin: config.admin, relayer }.publish(&env);
        Ok(())
    }

    /// Remove a relayer account (e.g. a rotated or compromised key). Admin only.
    pub fn remove_relayer(env: Env, relayer: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;
        let mut set = Self::get_relayers(env.clone());
        let Some(i) = set.first_index_of(&relayer) else {
            return Err(SponsorError::NotARelayer);
        };
        set.remove(i);
        env.storage().instance().set(&DataKey::Relayers, &set);
        extend_instance(&env);
        RelayerRemoved { admin: config.admin, relayer }.publish(&env);
        Ok(())
    }

    pub fn get_relayers(env: Env) -> Vec<Address> {
        env.storage().instance().get(&DataKey::Relayers).unwrap_or_else(|| Vec::new(&env))
    }

    pub fn is_relayer(env: Env, relayer: Address) -> bool {
        Self::get_relayers(env).contains(&relayer)
    }

    /// Record a sponsored operation BEFORE the relayer submits it: enforces
    /// the whitelist, pause, per-user quotas and the global daily budget on
    /// chain, and consumes `fee_spent_stroops` (the relayer's fee cap for the
    /// operation - an upper bound). `relayer` must be in the pool and sign
    /// (audit F1b, decision D-I).
    pub fn record_sponsored_op(
        env: Env,
        relayer: Address,
        user: Address,
        target_contract: Address,
        fee_spent_stroops: u64,
    ) -> Result<(), SponsorError> {
        let set = Self::get_relayers(env.clone());
        if set.is_empty() {
            return Err(SponsorError::RelayerNotSet);
        }
        if !set.contains(&relayer) {
            return Err(SponsorError::NotARelayer);
        }
        relayer.require_auth();

        // Ensure operation adheres to whitelist, circuit breaker, and budget before recording
        Self::validate_sponsorship(
            env.clone(),
            user.clone(),
            target_contract.clone(),
            fee_spent_stroops,
        )?;

        let now = env.ledger().timestamp();
        let today_epoch = now / 86400;

        // Update Global Stats
        let global_stats = Self::get_global_stats(env.clone());
        let updated_global_stats = GlobalSponsorStats {
            total_ops_today: global_stats.total_ops_today.saturating_add(1),
            total_fees_today_stroops: global_stats
                .total_fees_today_stroops
                .saturating_add(fee_spent_stroops),
            current_day_epoch: today_epoch,
        };
        env.storage()
            .instance()
            .set(&DataKey::GlobalStats, &updated_global_stats);

        // Update User Stats
        let current_user_stats = Self::get_user_stats(env.clone(), user.clone());
        let updated_user_stats = UserSponsorStats {
            hourly_ops: current_user_stats.hourly_ops.saturating_add(1),
            daily_ops: current_user_stats.daily_ops.saturating_add(1),
            last_op_timestamp: now,
        };

        let key = DataKey::UserStats(user.clone());
        env.storage().persistent().set(&key, &updated_user_stats);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
        extend_instance(&env);

        SponsoredOp { user, target_contract, fee_spent_stroops }.publish(&env);

        Ok(())
    }

    /// Configure rate limits and daily budget
    pub fn set_limits(
        env: Env,
        max_ops_per_hour: u32,
        max_ops_per_day: u32,
        daily_budget_stroops: u64,
    ) -> Result<(), SponsorError> {
        let mut config = Self::require_admin(&env)?;

        if max_ops_per_hour == 0
            || max_ops_per_day < max_ops_per_hour
            || daily_budget_stroops == 0
        {
            return Err(SponsorError::InvalidConfig);
        }

        config.max_ops_per_hour = max_ops_per_hour;
        config.max_ops_per_day = max_ops_per_day;
        config.daily_budget_stroops = daily_budget_stroops;

        env.storage().instance().set(&DataKey::Config, &config);

        LimitsUpdated { admin: config.admin, daily_budget_stroops }.publish(&env);

        Ok(())
    }

    /// Transfer sponsorship administration to a new admin address
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), SponsorError> {
        let mut config = Self::require_admin(&env)?;
        let old_admin = config.admin.clone();
        config.admin = new_admin.clone();
        env.storage().instance().set(&DataKey::Config, &config);

        SponsorAdminTransferred { old_admin, new_admin }.publish(&env);

        Ok(())
    }

    /// Emergency circuit breaker: pause all sponsorship immediately
    pub fn pause(env: Env) -> Result<(), SponsorError> {
        let mut config = Self::require_admin(&env)?;
        config.is_paused = true;
        env.storage().instance().set(&DataKey::Config, &config);

        SponsorPaused { admin: config.admin }.publish(&env);

        Ok(())
    }

    /// Emergency circuit breaker: resume sponsorship
    pub fn unpause(env: Env) -> Result<(), SponsorError> {
        let mut config = Self::require_admin(&env)?;
        config.is_paused = false;
        env.storage().instance().set(&DataKey::Config, &config);

        SponsorUnpaused { admin: config.admin }.publish(&env);

        Ok(())
    }

    /// Get current sponsor configuration
    pub fn get_config(env: Env) -> Result<SponsorConfig, SponsorError> {
        match env.storage().instance().get(&DataKey::Config) {
            Some(c) => Ok(c),
            None => Err(SponsorError::NotInitialized),
        }
    }

    /// Query usage statistics for a specific user (evaluates effective quota for current timestamp)
    pub fn get_user_stats(env: Env, user: Address) -> UserSponsorStats {
        let now = env.ledger().timestamp();
        let stored: Option<UserSponsorStats> =
            env.storage().persistent().get(&DataKey::UserStats(user));
        match stored {
            Some(stats) => {
                let hourly = if stats.last_op_timestamp == 0
                    || (now / 3600 != stats.last_op_timestamp / 3600)
                {
                    0
                } else {
                    stats.hourly_ops
                };

                let daily = if stats.last_op_timestamp == 0
                    || (now / 86400 != stats.last_op_timestamp / 86400)
                {
                    0
                } else {
                    stats.daily_ops
                };

                UserSponsorStats {
                    hourly_ops: hourly,
                    daily_ops: daily,
                    last_op_timestamp: stats.last_op_timestamp,
                }
            }
            None => UserSponsorStats {
                hourly_ops: 0,
                daily_ops: 0,
                last_op_timestamp: 0,
            },
        }
    }

    /// Query global sponsorship stats for today (resets if day rolled over)
    pub fn get_global_stats(env: Env) -> GlobalSponsorStats {
        let now = env.ledger().timestamp();
        let today_epoch = now / 86400;

        let stored: Option<GlobalSponsorStats> =
            env.storage().instance().get(&DataKey::GlobalStats);
        match stored {
            Some(stats) if stats.current_day_epoch == today_epoch => stats,
            _ => GlobalSponsorStats {
                total_ops_today: 0,
                total_fees_today_stroops: 0,
                current_day_epoch: today_epoch,
            },
        }
    }

    /// Query all currently whitelisted target contracts
    pub fn get_whitelisted_contracts(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::WhitelistList)
            .unwrap_or_else(|| Vec::new(&env))
    }

    fn require_admin(env: &Env) -> Result<SponsorConfig, SponsorError> {
        let config: SponsorConfig = match env.storage().instance().get(&DataKey::Config) {
            Some(c) => c,
            None => return Err(SponsorError::NotInitialized),
        };
        config.admin.require_auth();
        Ok(config)
    }
}

#[cfg(test)]
mod test;

