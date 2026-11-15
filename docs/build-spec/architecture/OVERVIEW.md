# System Architecture Overview

> Reference: Product doc §4.1 Principles, §4.2 Layers

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                        EXPERIENCE LAYER                          │
│   Web App (React/Next.js) · Partner SDK · Treasury Dashboard     │
│   Stellar Wallets Kit · Passkey Sign-in · SEP-24 Anchor UI      │
└──────────────────────────┬──────────────────────────────────────┘
                           │ HTTPS / WebSocket
┌──────────────────────────▼──────────────────────────────────────┐
│                      AI THESIS ENGINE (OFF-CHAIN)                │
│   Parse → Retrieve → Optimise → Simulate → Explain → Sign       │
│   LLM (schema-constrained) · Vector search (pgvector/Chroma)    │
│   PyPortfolioOpt · NAV history DB · LangChain orchestration      │
└──────────────────────────┬──────────────────────────────────────┘
                           │ signed policy hash
┌──────────────────────────▼──────────────────────────────────────┐
│                     REGISTRY & ELIGIBILITY                        │
│   On-chain registry contract + Off-chain diligence DB            │
│   Asset metadata · Issuer rules · Jurisdiction checks            │
│   SEP-57 identity verifier integration                           │
└──────────────────────────┬──────────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────────┐
│                        VAULT LAYER (Soroban)                      │
│   SEP-56 Strategy Vault · SEP-56 Private Vault Factory           │
│   SEP-57 Gated Share Class (Q1 2027+)                           │
│   Share accounting · Deposits · Redemptions · Fee accrual        │
│   OpenZeppelin stellar-contracts (audited)                       │
└──────────┬──────────────────────────────┬───────────────────────┘
           │                              │
┌──────────▼──────────┐       ┌───────────▼──────────────────────┐
│   POLICY CONTRACT   │       │        EXECUTION LAYER            │
│   Enforce weights   │       │   Keeper network · Rebalance bot  │
│   bands, caps       │◄──────│   Issuer mint/redeem adapters     │
│   Signed policy     │       │   Soroswap/Aquarius/SDEX routes   │
│   Rebalance gate    │       │   Blend adapter · DeFindex adapter│
└──────────┬──────────┘       └───────────────────────────────────┘
           │
┌──────────▼──────────────────────────────────────────────────────┐
│                    NAV & ORACLE LAYER                             │
│   Issuer NAV feeds · Reflector · RedStone                        │
│   Minimum-of-valid-sources (collateral) · Median (deposits)      │
│   SEP-40 compatible interface · Deviation circuit breakers       │
└──────────┬──────────────────────────────────────────────────────┘
           │
┌──────────▼──────────────────────────────────────────────────────┐
│                  DATA & INDEXING LAYER                            │
│   Stellar RPC · Galexie ledger ingestion · Unified events        │
│   PostgreSQL · Time-series store (NAV history, yield, flows)     │
│   Treasury reporting · Tax lots · Audit exports                  │
└─────────────────────────────────────────────────────────────────┘
           │
┌──────────▼──────────────────────────────────────────────────────┐
│               SECURITY & GOVERNANCE LAYER                         │
│   Multisig + timelock (admin) · Separate pause authority         │
│   Per-sleeve circuit breakers · CAP-85 fleet upgrades            │
│   TVL caps + per-user caps (mainnet beta) · Bug bounty           │
└─────────────────────────────────────────────────────────────────┘
```

---

## Layer Descriptions

### 1. Experience Layer
**Technology:** React / Next.js · Stellar Wallets Kit · Passkey API  
**Responsibilities:**
- Chat-to-basket flow: natural language input → policy proposal UI
- Simulation display (redemption ladder, stress scenarios, yield history)
- Portfolio dashboard (NAV, weights, income by sleeve)
- Treasury reporting (tax lots, audit exports — paid tier)
- Partner SDK (TypeScript, embeddable components)
- Passkey sign-in (no seed phrase required for consumer path)

**Key constraint:** The UI must always show the signed policy and diff it against the thesis, so mismatches are visible. No auto-submit of AI proposals.

---

### 2. AI Thesis Engine (Off-Chain)
**Technology:** LLM with schema-constrained function calling · pgvector or Chroma · PyPortfolioOpt · LangChain  
**Responsibilities:**
- Parse plain-English thesis into a typed `PolicyObject`
- Vector-search registry for candidate assets
- Run constrained portfolio optimisation
- Simulate NAV history, yield, redemption ladders, stress scenarios
- Generate plain-language explanation and thesis diff
- Produce a policy hash for user signing

**Key constraint:** Only registry IDs are valid outputs. Hallucinated tickers cannot appear. Full spec in `ai-engine/THESIS-ENGINE.md`.

---

### 3. Registry & Eligibility
**Technology:** Soroban contract + PostgreSQL off-chain DB  
**Responsibilities:**
- Canonical list of approved, diligenced assets with metadata
- Eligibility checks: jurisdiction, investor class, issuer allow-list
- Issuer rule enforcement (clawback, freeze, transfer restrictions)
- SEP-57 identity verifier integration

**Key constraint:** An asset enters the registry ONLY after diligence (issuer, custody, redemption terms, oracle availability, transfer rules). Full spec in `ai-engine/REGISTRY.md`.

---

### 4. Vault Layer
**Technology:** Soroban (Rust) · OpenZeppelin stellar-contracts  
**Responsibilities:**
- SEP-56 tokenized vault — share minting, burning, NAV per share
- Two vault types: Strategy (shared, cheap) and Private (per-institution)
- Two share classes: Open (permissionless assets) and Gated (SEP-57, Q1 2027+)
- Liquidity sleeve enforcement (5–15% minimum in cash-like assets)
- Fee accrual by share dilution (on-chain, visible)
- CAP-85 fleet upgrades (Adapter, Protocol 28)

**Full spec:** `architecture/VAULT-DESIGN.md`

---

### 5. Policy Contract
**Technology:** Soroban (Rust)  
**Responsibilities:**
- Store the signed policy hash per vault
- Validate every rebalance proposal against policy bounds
- Enforce: weight bands, issuer caps, liquidity floor, asset exclusions
- Reject any operation that would violate signed policy

**Key constraint:** The policy contract is the only on-chain authority. Keepers propose; policy contract approves or rejects.

---

### 6. Execution Layer
**Technology:** TypeScript keeper bots · Soroban adapters (Rust)  
**Responsibilities:**
- Monitor weight drift, trigger rebalances when bands are breached
- Flow-based rebalancing first (new deposits → underweight, withdrawals → overweight)
- Primary market (issuer mint/redeem) before AMMs
- AMM/SDEX routes with strict slippage caps for small residuals
- Atomic, idempotent, safe-resume-on-failure rebalance sequences
- Adapters: Blend, DeFindex, Soroswap, Aquarius, SDEX, issuer mint/redeem

**Full spec:** `architecture/EXECUTION.md`

---

### 7. NAV & Oracle Layer
**Technology:** Issuer NAV feeds · Reflector · RedStone · On-chain computation  
**Responsibilities:**
- Multi-source NAV with staleness limits and deviation detection
- SEP-40 compatible interface for lender integration
- Collateral NAV = minimum of valid sources
- Deposit/share NAV = median of valid sources

**Full spec:** `architecture/NAV-ORACLE.md` ← **Read this before any oracle implementation.**

---

### 8. Data & Indexing Layer
**Technology:** Stellar RPC · Galexie · PostgreSQL · TimescaleDB (or similar)  
**Responsibilities:**
- Ingest all vault events from the ledger
- Store NAV history, yield history, flow data, rebalance history
- Feed the thesis engine's simulation module
- Power treasury reporting, tax-lot tracking, audit exports

**Note:** Bell (Protocol 30) has a breaking ledger header format change. Every indexer must update XDR definitions before Bell mainnet (Nov 23 vote). Plan for this upgrade.

---

### 9. Security & Governance Layer
**Technology:** Multisig contracts · OpenZeppelin governance utilities  
**Responsibilities:**
- Multisig + timelock on all admin operations
- Separate pause authority (can pause without being able to upgrade)
- Per-sleeve circuit breakers (a freeze on one issuer doesn't break the whole basket)
- TVL caps and per-user caps during mainnet beta
- CAP-85 atomic fleet upgrades
- Incident runbook (must be public)

---

## Technology Stack Summary

| Layer | Primary Tech |
|---|---|
| Smart contracts | Rust · Soroban SDK 28.x · OpenZeppelin stellar-contracts v0.8.0 |
| Frontend | TypeScript · React/Next.js · Stellar Wallets Kit |
| AI engine | Python · LangChain · PyPortfolioOpt · pgvector |
| Data | PostgreSQL · Galexie · Stellar RPC |
| Oracles | Reflector · RedStone (live on Stellar since Mar 2026) |
| Testing | Soroban test framework · property testing for share maths |
| Deployment | Stellar testnet → audited mainnet |

---

## Stellar Network Parameters (as of Nov 2026)

| Parameter | Value |
|---|---|
| Fee floor | 100 stroops = 0.00001 XLM |
| XLM price (Oct 29) | ~$0.17 |
| Cost per classic op | < $0.001 |
| Ledger close (post-Adapter) | ~5 seconds |
| Uptime | 99.99% |
| Soroban SDK | 28.0.0 (18 Sep); latest stable 29.0.1; 30.0.0-rc.1 only |
| Active protocol | 28 (Adapter) |
| Next protocol | 30 (Bell) — testnet vote Nov 3, mainnet vote Nov 23 |
