# Stellar Ecosystem Proposals (SEPs) Reference

> Technical standards powering the Constella architecture. All implementations must strictly conform to these SEPs.  
> **Status (verified 15 Nov 2026):** SEP-40 Draft v0.1.0 · SEP-41 Draft v0.5.2 · SEP-56 Draft v0.1.2 · SEP-57 Draft v0.4.0. Drafts can change — re-check the upstream text before each slice.

---

## 1. SEP-41: Fungible Token Standard

### Overview
SEP-41 defines the standard token interface for Soroban smart contracts, acting as the Stellar equivalent of Ethereum's ERC-20.

### Replicating Properties
- **Decimals & Metadata:** Token name, symbol, decimals (default 7 for classic Stellar asset parity).
- **Core Operations:** `allowance`, `approve(from, spender, amount, live_until_ledger)`, `balance`, `transfer(from, to: MuxedAddress, amount)`, `transfer_from(spender, from, to, amount)`, **`burn(from, amount)`**, **`burn_from(spender, from, amount)`**, `decimals`, `name`, `symbol`.
- **Events (CAP-67 format):** `transfer` topics `["transfer", from, to]` data `amount` (or `{amount, to_muxed_id}`); `approve` topics `["approve", from, spender]` data `[amount, live_until_ledger]`; `mint` topics `["mint", to]` data `amount`; `burn` topics `["burn", from]` data `amount`. Event names are SEP-41-defined lower-case and take precedence over Rule 8 for token events.
- **Data formats (v0.5.0+):** every event may use either the single-value/vec format above or a map format (`{amount}`, `{amount, live_until_ledger}`, `{amount, to_muxed_id}`); both conform. Indexers must accept both. Constella emits single-value/vec, like the Stellar Asset Contract. SEP-41 also defines a `clawback` event (`[clawback, from]`, data `amount`); Constella shares have no clawback.
- **Status:** Draft v0.5.2 (updated 24 Aug 2026; re-verified 15 Nov 2026), de-facto standard across Soroban DeFi.

### Implementation in Constella
Every Constella basket token (vault share token) **must** implement SEP-41 so that:
- Users can hold, transfer, and trade shares in standard Stellar wallets (Freighter, xBull, Lobstr).
- AMMs (Soroswap, Aquarius) and DEX orderbooks can list the basket token.
- Lending markets (Blend, Liqvid) can accept the basket token as collateral.

**Status (15 Nov 2026, audit W6 fixed):** all core operations and events above are implemented on the vault share token, including `burn`/`burn_from`, validated `live_until_ledger`, and `mint` events for accrued fee shares. `transfer` accepts a muxed destination (W4.2b) and reports its id in the event.

---

## 2. SEP-56: Tokenized Vault Standard

### Overview
SEP-56 (Draft v0.1.2, authored by OpenZeppelin) is Stellar's ecosystem adaptation of Ethereum's ERC-4626 Tokenized Vault Standard. It provides a standardized API for yield-bearing single-asset deposit vaults.

### Core Functions & Properties
```rust
pub trait SEP56Vault {
    // Deposit / Mint
    fn deposit(env: Env, assets: i128, receiver: Address) -> i128;
    fn mint(env: Env, shares: i128, receiver: Address) -> i128;

    // Withdraw / Redeem
    fn withdraw(env: Env, assets: i128, receiver: Address, owner: Address) -> i128;
    fn redeem(env: Env, shares: i128, receiver: Address, owner: Address) -> i128;

    // Accounting & Pricing
    fn total_assets(env: Env) -> i128;
    fn total_supply(env: Env) -> i128;
    fn convert_to_shares(env: Env, assets: i128) -> i128;
    fn convert_to_assets(env: Env, shares: i128) -> i128;
    fn asset(env: Env) -> Address; // Underlying numéraire
    
    // Limits
    fn max_deposit(env: Env, receiver: Address) -> i128;
    fn max_mint(env: Env, receiver: Address) -> i128;
    fn max_withdraw(env: Env, owner: Address) -> i128;
    fn max_redeem(env: Env, owner: Address) -> i128;
}
```

### How Constella Extends SEP-56 for Multi-Asset Baskets
While classic SEP-56 targets a single underlying asset, Constella achieves SEP-56 compliance for multi-asset baskets by:
1. **Single Numéraire Settlement:** All deposits and redemptions enter and exit in a single base currency (USDC or EURC).
2. **Internal Multi-Asset Accounting:** Internally, the vault allocates numéraire across multiple sleeve constituents.
3. **NAV-Based Conversion:** `convert_to_shares` and `convert_to_assets` evaluate the aggregate basket NAV against the numéraire.
4. **Base Module:** OpenZeppelin `stellar-contracts` v0.8.0. Status (15 Nov 2026): the share ledger runs on OZ `stellar-tokens` fungible storage (audit W4.2a); routing the SEP-56 entry points through the OZ vault module, where it can carry dual NAV, caps, fees and the liquidity floor, is W4.2b.

---

## 3. SEP-57: T-REX (Regulated Token Standard)

### Overview
SEP-57 is Stellar's standard for compliant permissioned Real-World Assets, modeled after the Ethereum ERC-3643 (T-REX) framework.

### Architecture Components
1. **Identity Registry:** Maps Stellar addresses to verified identity hashes/claims (KYC/AML status, investor classification, country of residence).
2. **Compliance Engine:** Modular rule contracts that validate whether a transfer is legal prior to execution.
3. **Asset Features:**
   - Enforced transfer restrictions.
   - Forced transfer / recovery (for legal lost-key recovery).
   - Issuer freeze and clawback integration.
   - Lifecycle hooks (dividend distributions, voting snapshots).

### Implementation in Constella
- **Open Share Classes:** Use standard SEP-41 / SEP-56 (permissionless constituents only).
- **Gated Share Classes (Phase 3+):** When a basket contains permissioned assets (e.g., BENJI, USDY, Spiko, JQQQ), the basket share token itself must be an SEP-57 token.
- **Identity Verifier Hook:** Before minting or transferring gated shares, the vault queries the SEP-57 identity registry to confirm the recipient is eligible.
- **Liquidation Path:** Ensures only compliant, whitelisted liquidators can seize and unwind gated collateral.

---

## 4. SEP-40: Oracle Interface Standard

### Overview
SEP-40 specifies the standard interface for price oracle contracts on Soroban, standardizing how DeFi protocols fetch asset prices, base assets, and resolution.

### Core Interface
```rust
pub enum Asset { Stellar(Address), Other(Symbol) }
pub struct PriceData { pub price: i128, pub timestamp: u64 }

pub trait SEP40Oracle {
    fn base(env: Env) -> Asset;
    fn assets(env: Env) -> Vec<Asset>;
    fn decimals(env: Env) -> u32;
    fn resolution(env: Env) -> u32; // In seconds
    fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData>;
    fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>>;
    fn lastprice(env: Env, asset: Asset) -> Option<PriceData>;
}
```

> Blend v2 consumes oracles through the `sep-40-oracle` crate and calls only `lastprice(&Asset::Stellar(addr))`, rejecting prices ≤ 0 or older than 24h.

### Implementation in Constella
- Constella's NAV Oracle contract implements SEP-40.
- When Blend, Liqvid, or another lending market integrates a Constella basket token, they query our SEP-40 interface.
- Returns the **Conservative NAV** (minimum of valid sources) to safeguard lenders from collateral inflation.

---

## 5. Other Relevant SEPs

| SEP | Title | Relevance to Constella |
|---|---|---|
| **SEP-01** | `stellar.toml` | Declares issuer metadata, currency definitions, documentation, and contract addresses |
| **SEP-10** | Stellar Web Authentication | Cryptographic challenge-response authentication for API/Web logins using Stellar keypairs |
| **SEP-24** | Hosted Deposit & Withdrawal | Fiat on/off-ramp integration for partner wallets (fiat → USDC → basket) |
| **SEP-38** | Anchor RFQ API | Real-time quotes for cross-currency anchor swaps (e.g., fiat EUR to EURC) |
