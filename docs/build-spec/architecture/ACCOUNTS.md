# Accounts & Onboarding Specification

> Reference: Product doc §4.8 Accounts and Onboarding

---

## Design Goals

1. **Zero friction for users** — no XLM required, no seed phrases for consumer path
2. **Zero gas for users** — we sponsor all fees and reserves
3. **Institutional-grade controls** — multisig, timelocks, eventually ML-DSA
4. **Bell-ready** — v1 works without Bell; Bell features are additive modules

---

## Why Classic Accounts Are Problematic for Onboarding

Classic Stellar accounts require:
- **Minimum balance:** 1 XLM base reserve
- **Per-trustline reserve:** 0.5 XLM
- A basket with 5 assets needs ~3.5 XLM just to hold the assets

For users who don't own XLM and don't want to think about blockchain, this is a dealbreaker.

**Solutions:**
1. **Fee sponsorship** — we sponsor the reserve and fees via fee-bump transactions
2. **Contract accounts** — Soroban smart accounts don't have classic reserve requirements in the same way

---

## Account Types in Constella

### 1. Passkey Smart Account (Consumer Default)
- Uses **secp256r1 passkeys** (device biometrics — Face ID, Touch ID, Windows Hello)
- No seed phrase, no private key management by the user
- Social recovery: multiple authorized signers (e.g. phone + email + guardian address)
- **Zipper (Protocol 27, CAP-71):** Authentication delegation for custom accounts — cheaper multi-signer setups with replay protection
- Built on OpenZeppelin `stellar-accounts` context rules (W4.3b). Passkey signatures are WebAuthn assertions, requiring user presence **and** user verification; a shared `webauthn-verifier` contract checks them.
- **Owner rule:** the owner passkey has full control. This rule cannot be removed, cannot expire, and always keeps a signer.
- **Recovery rule:** M-of-N guardians under the shared `threshold-policy`, scoped to the account. Guardians can only **propose** a new owner passkey. It takes effect after **48 hours**, and the current owner can cancel it during that window (decision D-F). After 48 hours anyone can execute it, so a user who lost their phone needs no extra step. Guardians can never move funds or change anything else; only the owner rule manages the account.
- **Known limitation:** recovery protects against a **lost** passkey, not a **stolen** one. Whoever holds the owner passkey can cancel recoveries and already has full control, so a stolen owner key must be treated like a stolen card: move funds or rotate keys at once. Planned mitigations to consider: an owner-rule daily spending limit (OZ `spending_limit` policy), or a guardian-majority freeze (audit S4).

**User experience:** "Create account with Face ID" → deposit → done. No XLM, no seed phrase.

### 2. Institutional Multisig Smart Account (Treasury Default)
- M-of-N multisig (e.g. 3-of-5 for a startup treasury)
- Timelocked operations for large movements
- Policy-based spend controls
- Audit log of all actions (via on-chain events)
- **After Bell (Protocol 30):** Can require ML-DSA signatures for quantum preparedness

### 3. Classic Stellar Account (Partner-Sponsored)
- Used when the partner (wallet, fintech) handles account management
- Constella sponsors reserves and fees for these accounts
- Useful for partners who already have Stellar accounts for their users

---

## Fee Sponsorship Architecture

```
Sponsor Account (Constella-controlled)
  ├── Maintains XLM balance for fees
  ├── Spend limit: configurable maximum per period
  ├── Rate limit: maximum fee-bumps per hour
  └── Signs fee-bump transactions for user operations

User Account
  ├── Holds share tokens
  ├── Never pays gas
  └── All transactions fee-bumped by sponsor
```

**Security against drain attacks:**
- Sponsor account has daily spend limits
- Rate limits on fee-bumps per user
- Minimum trust: fee sponsorship is for vault interactions only
- Monitor for unusual fee patterns; alert if limits approached

**Cost modeling:**
- Classic operation: ~$0.0001 at current XLM prices
- Soroban call: fractions of a cent to a few cents (measure per operation in simulation)
- ~12 user operations per year = $0.05–$0.25 per year per user (see §5.4)

---

## Reserve Sponsorship

For classic accounts needing trustlines:
- Constella's sponsor account establishes the account and trustlines
- Sponsor covers the 0.5 XLM per trustline reserve
- If user later withdraws, trustline reserves are reclaimed by sponsor

For contract accounts:
- No classic reserve structure; Soroban storage rent applies
- Keepers manage rent extensions (TTL control from Protocol 26)

---

## Partner Onboarding Architecture

### Pre-Bell (v1)
- Partners embed the Constella SDK
- Each user gets a Stellar account sponsored by Constella or the partner
- Partner maintains their own customer ledger (off-chain) mapping their user IDs to Stellar addresses
- Reconciliation: partner reads vault events filtered by Stellar address

### Post-Bell (Protocol 30, Q2 2027 feature)
- Partner holds **one contract account**
- Each customer deposit carries a **64-bit customer ID in the W-address** (muxed contract account, CAP-84)
- W-address appears in transfer events automatically
- Partner reconciles thousands of customers from one contract's event log
- No per-customer contract deployment
- Much cheaper at scale

**Bell migration plan:**
- v1 SDK is designed to be compatible with the W-address upgrade
- Migration: existing user accounts get a W-address alias; no new accounts needed
- Partners get a migration guide when Bell is live on mainnet

---

## ML-DSA (Post-Bell, Protocol 30)

For treasury vault admins who want quantum preparedness:
- **CAP-87** adds ML-DSA verification in contracts
- Treasury vault admin accounts can require ML-DSA signatures for all admin operations
- SDF's Quantum Preparedness Plan targets enterprise wallets moving to quantum-safe signing in 2026 via contract accounts
- Implementation: additive module after Bell mainnet vote (Nov 23)

---

## Onboarding Flows

### Consumer Flow (Partner App)
```
1. User opens partner app
2. Partner app calls Constella SDK to create account
3. SDK creates passkey smart account (sponsored)
4. User authenticates with Face ID / Touch ID
5. User deposits (fiat → anchor → USDC, or existing USDC)
6. Constella thesis engine proposes basket (or uses template)
7. User approves policy (signs policy hash with passkey)
8. Shares minted to user's account
```

### Treasury Flow (Direct)
```
1. Treasury team visits constella.app/treasury
2. Set up institutional multisig account (N keyholders, M threshold)
3. Each keyholder adds their key (passkey or hardware wallet)
4. Treasury admin types investment thesis
5. AI engine proposes policy; team reviews weights, bands, caps
6. Treasury admin requests ratification from required signers
7. M-of-N signers approve policy hash
8. Private vault deployed (Q1 2027) or strategy vault assigned
9. First deposit from treasury multisig
```

---

## Wallets Kit Integration

Using **Stellar Wallets Kit** for wallet connection:
- Supports: Freighter, xBull, Lobstr, Rabet, and others
- Handles wallet-specific signing flows
- Upgrade to v28/v30 SDK versions before Bell mainnet vote

---

## Implementation Checklist

- [ ] Passkey smart account using OpenZeppelin smart-account framework
- [ ] Fee-bump transaction sponsorship for all user vault operations
- [ ] Reserve sponsorship for classic accounts (if used)
- [ ] Sponsor account with spend limits and rate limits
- [ ] Stellar Wallets Kit integration (upgraded to v28+)
- [ ] Institutional multisig support (M-of-N)
- [ ] Zipper (Protocol 27) authentication delegation implemented
- [ ] Partner SDK with account creation
- [ ] Bell readiness: W-address (CAP-84) design documented, implementation deferred
- [ ] Bell readiness: ML-DSA (CAP-87) design documented, implementation deferred
- [ ] Emergency: account recovery flow for passkey accounts
- [ ] Security: sponsor account monitoring and alerting
