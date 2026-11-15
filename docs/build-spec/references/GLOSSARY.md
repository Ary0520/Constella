# Constella Ecosystem Glossary

> Comprehensive dictionary of technical, financial, and protocol terminology used across the Constella specification.

---

## 1. Constella Product Terminology

| Term | Definition |
|---|---|
| **Basket** | A rule-bound collection of tokenized Stellar assets (RWAs, stablecoins, lending positions) managed as a unified portfolio and represented by a single share token. |
| **Strategy Vault** | A pooled, shared SEP-56 vault contract serving multiple participants with uniform exposure; the low-gas default for retail and embedded partners. |
| **Private Vault** | An isolated, institution-specific vault instantiated by a factory contract with customized policy rules, dedicated admin keys, and private accounting. |
| **Numéraire** | The single base settlement currency (USDC or EURC) in which deposits, redemptions, and NAV calculations are executed. |
| **Share Token** | The SEP-56 / SEP-41 compliant token minted to users representing their fractional, non-custodial ownership of underlying vault assets. |
| **Sleeve** | A logical partition within a basket grouping assets by risk/asset class (e.g., `CASH_STABLE`, `USD_GOV_FUNDS`, `EUR_SOVEREIGN`, `LENDING`, `EQUITY`). |
| **Liquidity Sleeve** | The mandatory cash-equivalent portion of a basket (typically 5%–15% in USDC/EURC) reserved for instant redemptions and liquidations. |
| **Redemption Ladder** | A structured view showing what percentage of basket capital can be exited across time horizons (< 1 hour, 24 hours, 7 days, illiquid). |
| **Band** | The permissible drift percentage around a sleeve's target weight (e.g., Target 40% with ±5% band = 35%–45%) before triggering keeper rebalancing. |
| **Issuer Cap** | A maximum protocol or user-defined ceiling preventing any single asset issuer (e.g., Spiko) from exceeding a fixed percentage (default 30%) of basket AUM. |
| **Conservative NAV** | The lowest price among all verified, non-stale oracle feeds; used for Collateral Passport risk evaluations **and for redemptions/withdrawals**, so departing holders never extract more than the most conservative valuation. |
| **Deposit NAV** | The median price of verified oracle feeds; used for deposits/minting so new entrants are not under-priced. Invariant: Conservative NAV ≤ Deposit NAV. |
| **Circuit Breaker** | An automated contract pause that isolates an individual sleeve when price deviation exceeds thresholds (> 2%–3%) or oracle feeds go stale. |
| **Collateral Passport** | The standardized on-chain data package (conservative NAV, instant liquidity, risk sheet, liquidation path) that allows lenders to accept basket shares. |
| **Listing Kit** | The turnkey configuration package (addresses, risk parameters, oracle adapters) provided to Blend and Liqvid pool creators to list basket tokens. |
| **Thesis Engine** | The off-chain AI pipeline that translates natural-language investment requirements into a strictly typed, mathematically optimized policy object. |
| **Signed Policy** | The canonical JSON policy specification hashed (SHA-256) and cryptographically signed on-chain by the user, binding contract execution. |
| **Keeper** | An automated off-chain worker that monitors sleeve drift, submits rebalance proposals, and extends Soroban state TTLs. |

---

## 2. Stellar & Soroban Blockchain Terminology

| Term | Definition |
|---|---|
| **Soroban** | Stellar's Rust-based, WebAssembly (WASM) smart contract execution platform. |
| **Stellar Classic** | The base-layer ledger protocol handling fast payments, trustlines, multi-sig, and the native decentralized exchange (SDEX). |
| **Stroop** | The smallest divisible unit of XLM (1 stroop = 0.0000001 XLM). Base network fee is 100 stroops. |
| **Fee-Bump Transaction** | A Stellar transaction envelope allowing a sponsor account to pay transaction fees on behalf of another signing account. |
| **Trustline** | An explicit opt-in balance record required on Stellar Classic to hold a specific non-native asset; requires a 0.5 XLM reserve deposit. |
| **Passkey Smart Account** | A Soroban custom account contract utilizing `secp256r1` (WebAuthn) elliptic curve signatures, enabling Face ID / Touch ID authentication without seed phrases. |
| **TTL (Time-To-Live)** | Soroban's state expiration mechanism; contract storage entries must have their TTL periodically extended to prevent data eviction. |
| **CAP-84 (W-Address)** | A multiplexed contract address format introduced in Bell (Protocol 30) enabling one master contract to route payments for millions of sub-accounts via 64-bit mux IDs. |
| **CAP-85 (Fleet Upgrade)** | Protocol 28 feature allowing atomic upgrades of multiple contract instances sharing the same WASM bytecode hash in a single transaction. |
| **CAP-87 (ML-DSA)** | Module-Lattice-Based Digital Signature Algorithm; quantum-resistant signature verification primitive added to Soroban in Protocol 30. |
| **CAP-88 (Millisecond Time)** | Protocol 30 update recording ledger block timestamps in milliseconds, paving the way for sub-second consensus. |
| **SEP-40** | Standard oracle interface for Soroban price feeds. |
| **SEP-41** | Standard fungible token interface on Soroban (ERC-20 equivalent). |
| **SEP-56** | Standard tokenized vault interface on Soroban (ERC-4626 equivalent). |
| **SEP-57** | Standard regulated token framework with compliance and identity verifiers (ERC-3643 / T-REX equivalent). |
| **Galexie** | High-performance ledger ingestion framework for streaming and indexing Soroban events. |

---

## 3. Financial & Regulatory Terminology

| Term | Definition |
|---|---|
| **RWA (Real-World Asset)** | Traditional physical or financial assets (Treasuries, real estate, commercial paper) represented on-chain via cryptographic tokens. |
| **LTV (Loan-to-Value)** | The ratio of borrowed funds to the market value of deposited collateral (e.g. 80% LTV on $100 collateral allows borrowing $80). |
| **Haircut** | The risk discount deducted from collateral value (e.g. a 15% haircut corresponds to an 85% maximum LTV). |
| **SOFR (Secured Overnight Financing Rate)** | The broad benchmark rate for overnight cash borrowing collateralized by US Treasuries. |
| **MiCA** | The European Union's comprehensive Markets in Crypto-Assets regulatory framework. |
| **Manager of Record (MoR)** | A licensed regulatory entity that assumes legal and advisory responsibility for asset management products distributed to retail users. |
| **Hierarchical Risk Parity (HRP)** | A machine-learning-driven portfolio allocation algorithm that clusters assets by covariance to produce stable, robust risk-weighted allocations without matrix inversion. |
