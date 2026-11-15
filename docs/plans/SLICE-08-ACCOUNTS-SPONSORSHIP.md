# Slice 8 Micro-Spec: Accounts, Zero-Gas Fee Sponsorship & Passkeys

## 1. Context & Objectives
- **Target Contracts:**
  - `contracts/smart-account`: Passkey Smart Account contract implementing Soroban's `CustomAccountInterface` with `secp256r1` (P-256) biometric authentication.
  - `contracts/fee-sponsor`: On-chain quota, rate-limiting, whitelist, and budget enforcement contract for fee sponsorship.
- **Build Spec References:**
  - `docs/build-spec/architecture/ACCOUNTS.md` (Passkey smart accounts, fee sponsorship architecture, drain prevention)
  - `docs/build-spec/roadmap/ROADMAP.md` (§1.6 Fee Sponsorship & §1.7 Passkey Auth)
  - `docs/build-spec/contracts/CONTRACTS-SPEC.md` (§ Security Properties & Account Abstraction)
- **Role:** Deliver frictionless consumer onboarding with zero XLM requirements, zero seed phrases, and zero user-paid gas fees.

---

## 2. Invariants & Rules

1. **Passkey secp256r1 Authentication:**
   - Signatures are verified against the standard NIST P-256 (`secp256r1`) curve using `env.crypto().secp256r1_verify(&public_key, &msg_hash, &signature)`.
   - Public keys are 65-byte uncompressed SEC-1 format (`0x04 || X || Y`).
   - Signatures are 64-byte raw format (`r || s`, 32 bytes each).
2. **CustomAccountInterface Conformance:**
   - Implements `CustomAccountInterface` so that any `Address::require_auth()` directed to the smart account executes `__check_auth`.
3. **Threshold & Multi-Signer Model:**
   - Each registered signer has a `weight: u32` and `role: SignerRole` (`Owner`, `Backup`, `Guardian`).
   - Authentication succeeds if and only if the sum of valid, non-duplicate signer weights is `>= threshold`.
4. **Signer Rotation & Safety:**
   - Signer additions, removals, and threshold changes require self-authorization (`contract_address.require_auth()`).
   - Removing a signer or updating threshold cannot leave the account in an unreachable state (remaining weight must be `>= threshold`).
5. **Fee Sponsorship Whitelisting:**
   - Sponsor contract only permits fee sponsorship for calls to vetted Constella contracts (`vault-strategy`, `policy`, `registry`).
   - Interactions with unauthorized or foreign contracts are rejected (`SponsorError::TargetNotWhitelisted`).
6. **Drain-Attack Prevention & Rate Limiting:**
   - Per-user hourly quota (e.g. 10 ops/hour) and daily quota (e.g. 50 ops/day).
   - Global daily budget limit in stroops.
   - Any breach reverts with explicit errors (`SponsorError::HourlyLimitExceeded`, `SponsorError::DailyBudgetExceeded`).
7. **Emergency Circuit Breaker:**
   - Sponsor contract can be paused instantly by sponsor admin to stop all sponsored operations.
8. **Storage Hygiene:**
   - Storage entries are cleaned up upon signer removal to conserve ledger rent.

---

## 3. Data Structures & Storage Layout

### Smart Account (`contracts/smart-account`)
```rust
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SignerRole {
    Owner = 0,
    Backup = 1,
    Guardian = 2,
}

#[contracttype]
#[derive(Clone)]
pub struct Signer {
    pub public_key: BytesN<65>,
    pub weight: u32,
    pub role: SignerRole,
    pub name: String,
}

#[contracttype]
#[derive(Clone)]
pub struct PasskeySignature {
    pub public_key: BytesN<65>,
    pub signature: BytesN<64>,
}

#[contracttype]
#[derive(Clone)]
pub struct PasskeyAuth {
    pub signatures: Vec<PasskeySignature>,
}
```

### Fee Sponsor (`contracts/fee-sponsor`)
```rust
#[contracttype]
#[derive(Clone)]
pub struct SponsorConfig {
    pub admin: Address,
    pub max_ops_per_hour: u32,
    pub max_ops_per_day: u32,
    pub daily_budget_stroops: u64,
    pub is_paused: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct UserSponsorStats {
    pub hourly_ops: u32,
    pub daily_ops: u32,
    pub last_op_timestamp: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct GlobalSponsorStats {
    pub total_ops_today: u32,
    pub total_fees_today_stroops: u64,
    pub current_day_epoch: u64,
}
```

---

## 4. Test Matrix (T47 – T55)

| Test ID | Objective | Scenario | Expected Outcome |
|---|---|---|---|
| **T47** | Passkey Account Init | Initialize smart account with primary passkey and threshold 1 | Account initialized, signer stored, threshold set |
| **T48** | secp256r1 Valid Auth | User executes call authorized with valid P-256 signature | `__check_auth` succeeds; operation executes |
| **T49** | Invalid Signature Rejection | Tampered signature or wrong key submitted | `__check_auth` fails with `AccError::InvalidSignature` |
| **T50** | Multi-Signer Threshold | 2-of-3 threshold setup with guardian keys | Single key fails; two valid signatures succeed |
| **T51** | Signer Rotation & Safety | Add backup passkey; remove old key; update threshold | Signer updated; cannot reduce weight below threshold |
| **T52** | Sponsorship Whitelist | User calls whitelisted vault vs foreign contract | Whitelisted contract passes; foreign contract rejected |
| **T53** | Sponsorship Rate Limits | User executes multiple ops in rapid succession | Exceeding hourly quota rejected (`HourlyLimitExceeded`) |
| **T54** | Sponsorship Budget Limit | Cumulative operations exceed daily budget | Rejects further sponsorship (`DailyBudgetExceeded`) |
| **T55** | End-to-End Sponsored Vault Call | Passkey smart account calls `vault.deposit()` under fee sponsorship | Operation succeeds with zero user XLM |

---

## 5. Execution & Verification Results

All deliverables and tests T47–T55 are implemented, verified, and passing:
- `contracts/smart-account`: Passkey smart account implementing `CustomAccountInterface`, `secp256r1` signature validation, multi-signer weighting, anti-bricking safety, and storage rent hygiene.
- `contracts/fee-sponsor`: Fee sponsorship guardrails contract enforcing target contract whitelists, hourly & daily operation quotas, cumulative daily budgets, and circuit breaker pause/unpause.
- Test Suite: 60 total unit and integration tests passing across all 6 workspace crates (`cargo test --manifest-path contracts/Cargo.toml`):
  - `constella_smart_account`: 7/7 tests passing (T47–T51, unauthorized self-auth enforcement, and T55).
  - `constella_fee_sponsor`: 4/4 tests passing (T52–T54, dynamic stats rollover, admin transfer, and limit reconfig).
  - `constella_vault_strategy`: 13/13 tests passing.
  - `constella_policy`: 18/18 tests passing.
  - `constella_nav_oracle`: 13/13 tests passing.
  - `constella_registry`: 5/5 tests passing.
- Linting: 0 warnings on strict lint (`cargo clippy --manifest-path contracts/Cargo.toml --all-targets -- -D warnings`).
- WASM Builds: All 6 contracts build to compact release WASMs via `stellar contract build`:
  - `constella_smart_account.wasm`: 9,101 bytes
  - `constella_fee_sponsor.wasm`: 9,423 bytes
  - `constella_vault_strategy.wasm`: 33,702 bytes
  - `constella_nav_oracle.wasm`: 19,058 bytes
  - `constella_policy.wasm`: 15,304 bytes
  - `constella_registry.wasm`: 6,497 bytes
