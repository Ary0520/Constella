#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Env, Vec,
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
/// Audit F1b: the relayer (fee-paying service key) was (re)designated.
#[contractevent(topics = ["RELAYER_SET"], data_format = "single-value")]
pub struct RelayerSet {
    #[topic]
    pub admin: Address,
    pub relayer: Address,
}

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
    /// Audit F1b: the only address allowed to record sponsored operations.
    Relayer,
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
        env.storage()
            .instance()
            .get(&DataKey::Whitelisted(target))
            .unwrap_or(false)
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

    /// Audit F1b: designate the relayer - the service key that pays fees and
    /// records sponsored operations. Admin only. Least privilege: the relayer
    /// can only record (within the limits); it cannot change limits, the
    /// whitelist, the admin, or unpause.
    pub fn set_relayer(env: Env, relayer: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;
        env.storage().instance().set(&DataKey::Relayer, &relayer);
        extend_instance(&env);
        RelayerSet { admin: config.admin, relayer }.publish(&env);
        Ok(())
    }

    pub fn get_relayer(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Relayer)
    }

    /// Record a sponsored operation BEFORE the relayer submits it: enforces
    /// the whitelist, pause, per-user quotas and the global daily budget on
    /// chain, and consumes `fee_spent_stroops` (the relayer's fee cap for the
    /// operation - an upper bound). Relayer only (audit F1b).
    pub fn record_sponsored_op(
        env: Env,
        user: Address,
        target_contract: Address,
        fee_spent_stroops: u64,
    ) -> Result<(), SponsorError> {
        let relayer: Address = env.storage().instance().get(&DataKey::Relayer).ok_or(SponsorError::RelayerNotSet)?;
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

