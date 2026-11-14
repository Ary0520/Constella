# Product & Engineering Roadmap

> Phased execution plan with strict gating milestones, acceptance criteria, and sprint breakdowns.

---

## Roadmap Timeline Summary

```
  2026                               2027
  NOV        DEC           Q1                  Q2                  H2
───┬──────────┬─────────────┬───────────────────┬───────────────────┬────────►
   │          │             │                   │                   │
   ▼          ▼             ▼                   ▼                   ▼
Phase 1    Phase 2       Phase 3             Phase 4             Phase 5
Testnet    Bell-Ready    Audit & Capped      Partner SDK &       ZK Proofs &
MVP        & Sim Engine  Mainnet Beta        Equities Module     Scale Performance
```

---

## Phase 1: Testnet MVP (November 2026)

**Goal:** Deliver a working end-to-end prototype on Stellar Testnet featuring permissionless asset baskets, fee sponsorship, passkeys, and the core AI thesis-to-policy pipeline.

### Gate to Exit Phase 1
- [ ] 10 treasury conversations initiated.
- [ ] 3 issuer conversations initiated (Spiko, Ondo, Franklin Templeton).
- [ ] 2 lender conversations initiated (Blend, Liqvid).
- [ ] Full automated test suite passing on Soroban testnet.

### Detailed Deliverables

| ID | Module | Description | Tech Stack |
|---|---|---|---|
| **1.1** | **Vault Contract** | Strategy vault implementing SEP-56 / SEP-41 | Soroban Rust, OpenZeppelin v0.8.0 |
| **1.2** | **Registry Contract** | On-chain registry of 3 permissionless assets: USDC, EURC, Blend USDC | Soroban Rust |
| **1.3** | **Policy Contract** | Storage and cryptographic validation of signed policy hashes | Soroban Rust |
| **1.4** | **Thesis Engine v0** | Parse thesis text → typed JSON schema → vector match → HRP weights | Python, LangChain, PyPortfolioOpt |
| **1.5** | **NAV Oracle v0** | Two-tier pricing for testnet (USDC 1:1, EURC SDEX, Blend pool rate) | Soroban Rust, SEP-40 |
| **1.6** | **Fee Sponsorship** | Sponsor contract executing fee-bump transactions for zero-gas user ops | Stellar Classic fee-bump |
| **1.7** | **Passkey Auth** | WebAuthn / secp256r1 smart account login with zero seed phrase | OpenZeppelin smart accounts |
| **1.8** | **Web UI Prototype** | Interactive dashboard: Chat prompt → Simulation view → Mint shares | Next.js 14, Tailwind, Stellar Wallets Kit |
| **1.9** | **Bell SDK Upgrade** | Upgrade client and contract SDKs to Protocol 30 / Bell compatible versions | `soroban-sdk` & `@stellar/stellar-sdk` |
| **1.10** | **SCF Submission** | Submit formal application to Stellar Community Fund (Round #47) | SCF Portal |

---

## Phase 2: Bell Integration & Simulation Engine (December 2026)

**Goal:** Integrate post-vote Bell modules, build advanced stress simulation, complete first Collateral Passport draft, and prepare for external audit.

### Gate to Exit Phase 2
- [ ] At least 1 anchor design partner testing on testnet.
- [ ] Bell mainnet vote passed (Nov 23) and node/indexer infrastructure updated.
- [ ] Collateral Passport specification formally reviewed by at least one lender (Liqvid or Blend).
- [ ] Smart contract audit firm selected and scope finalized.

### Detailed Deliverables

| ID | Module | Description | Tech Stack |
|---|---|---|---|
| **2.1** | **Bell Additive Modules** | Modular activation of CAP-84 W-Addresses and CAP-87 ML-DSA | Rust Soroban SDK v30 |
| **2.2** | **Stress Simulation** | Replay engine: depeg scenarios, issuer freezing stress, redemption ladder | Python, TimescaleDB, NumPy |
| **2.3** | **Collateral Passport v1** | View functions: `get_nav_conservative()`, `get_redemption_capacity()`, `get_risk_sheet()` | Soroban Rust |
| **2.4** | **Private Vault Factory** | Factory contract allowing institutions to spin up isolated private vaults | Soroban Rust |
| **2.5** | **Automated Keeper v1** | Off-chain TypeScript worker monitoring bands and extending TTL rent | TypeScript, Node.js |
| **2.6** | **Ledger Ingestion** | Full indexer subscribing to contract events via Galexie | Galexie, Go, PostgreSQL |
| **2.7** | **Audit Preparation** | Formal code freeze, test coverage > 95%, slither/cargo fuzz test suite | Rust testing harness |

---

## Phase 3: Audit & Capped Mainnet Beta (Q1 2027)

**Goal:** Securely launch on Stellar Mainnet with strict safety caps, at least 1 issuer allow-listing, and the first live lender collateral integration.

### Gate to Exit Phase 3
- [ ] Comprehensive security audit completed with zero open Critical/High issues.
- [ ] 5–10 committed enterprise design partner treasuries onboarded.
- [ ] At least 1 regulated issuer allow-lists the Constella vault contract.
- [ ] At least 1 active lending pool accepts Constella vault shares as collateral.

### Detailed Deliverables

| ID | Module | Description | Tech Stack |
|---|---|---|---|
| **3.1** | **Security Audit** | Full third-party security audit of Vault, Policy, Registry, and Oracle code | Independent audit firm |
| **3.2** | **Capped Mainnet Launch** | Deploy to mainnet with a strict $1M TVL protocol cap and per-user caps | Mainnet deployment |
| **3.3** | **Gated Share Class** | SEP-57 compliant share token with live identity verifier hooks | OpenZeppelin SEP-57 module |
| **3.4** | **Issuer Mint/Redeem** | First live primary market adapters (Spiko or Ondo USDY) | Custom issuer adapters |
| **3.5** | **First Lender Listing** | Blend or Liqvid pool integration accepting Constella share token | Blend v2 integration |
| **3.6** | **Treasury SaaS Portal** | Multi-user role management, tax-lot accounting, audit-ready CSV exports | Next.js, Postgres |
| **3.7** | **Bug Bounty Program** | Public bug bounty for smart contract vulnerabilities | Immunefi / SDF Bug Bounty |

---

## Phase 4: Partner SDK & Equities Module (Q2 2027)

**Goal:** Scale distribution through partner wallets via W-addresses, launch the equity sleeve upon DTCC integration, and lift TVL caps.

### Gate to Exit Phase 4
- [ ] At least 1 partner wallet or anchor live with integrated Constella SDK.
- [ ] DTCC Tokenization Service live on Stellar with Russell 1000/ETF assets available.
- [ ] Protocol TVL exceeds $25M.

### Detailed Deliverables

| ID | Module | Description | Tech Stack |
|---|---|---|---|
| **4.1** | **Partner TypeScript SDK** | Drop-in SDK for third-party wallets to instantiate baskets and route deposits | TypeScript, npm |
| **4.2** | **W-Address Omnibus** | Single partner contract routing thousands of users via CAP-84 64-bit mux IDs | Soroban Bell features |
| **4.3** | **Equities Sleeve** | Integrate JQQQ and DTC-tokenized equities/ETFs into candidate registry | Soroban adapters |
| **4.4** | **Curator Marketplace** | Beta portal for community curators to publish verified thesis baskets | Web frontend, Soroban |
| **4.5** | **Decentralized Keepers** | Multi-keeper network with stake-based slashing and execution rewards | Keeper contract network |
| **4.6** | **TVL Cap Uncapped** | Lift capped beta limits based on audited performance and insurance pool | Governance action |

---

## Phase 5: ZK Privacy, Local Currencies & Scale (H2 2027)

**Goal:** Institutional-grade quantum security, zero-knowledge compliance, emerging market bond expansion, and sub-second execution.

### Deliverables
- **ML-DSA Admin Accounts:** Standard quantum-resistant multi-signer accounts for enterprise treasuries.
- **ZK Eligibility Proofs:** Privacy-preserving compliance proofs using Protocol 25 BN254/Poseidon primitives and OpenZeppelin v0.8.0 confidential token module.
- **Local Currency Expansion:** Latin American sovereign debt sleeves (Etherfuse Cetes & Brazilian Tesouro) and Archax European sovereigns.
- **Sub-Second Rebalancing:** Leverage millisecond ledger consensus to execute complex multi-hop rebalances within single-second execution windows.
- **White-Label Issuer Engine:** Turnkey white-label basket architecture for sovereign asset managers and banking institutions.
