# Constella Build Specification

> **Working name:** Constella  
> **Version:** Production-Grade Build Spec v1.0  
> **Last updated:** 14 November 2026  
> **Foundational document:** Product, Architecture & Business Plan · 30 October 2026

---

## What This Folder Is

This is the **single source of truth** for every agent, engineer, auditor, and contributor working on Constella. Before implementing any feature, writing any Soroban smart contract, or configuring any AI pipeline, you must consult the corresponding specification in this directory.

**Non-Negotiable Rule:** If an implementation detail, customer suggestion, or third-party guideline conflicts with what is specified here, the specification takes precedence until this repository is formally updated and committed.

---

## One-Paragraph Product Summary

Constella turns a plain-English investment thesis into a rule-bound basket of Stellar's tokenized real-world assets (RWAs), packaged as a single non-custodial vault-share token on Soroban. A user types a thesis; an AI engine proposes an optimized basket using only assets that actually exist on Stellar and that this user is legally eligible to hold; a historical risk and redemption simulation is displayed; and a vault-share token is minted on Soroban. The vault self-rebalances within signed on-chain guardrails, costs 0.05%–0.25% per year with zero gas for the user, and ships with a **Collateral Passport** (multi-source conservative NAV, instant-redemption capacity, dynamic risk sheets, and compliant liquidation paths) so money markets and lenders on Stellar can accept it as collateral.

**The Market Gap:** Stellar has $4.1B+ of tokenized RWAs, but only ~$195M–$273M of DeFi TVL — meaning over 95% of tokenized value sits idle in wallets. Constella provides the infrastructure to move Stellar RWAs from "useful" (earning idle yield) to "used" (active daily collateral).

---

## Complete Build Spec Directory Index

### Core Governance & Guidelines
- [`AGENT-RULES.md`](AGENT-RULES.md) — Mandatory operational laws for all AI agents and engineers.

### 1. Architecture Layer (`architecture/`)
- [`architecture/OVERVIEW.md`](architecture/OVERVIEW.md) — High-level architecture, layer interactions, and data flow.
- [`architecture/VAULT-DESIGN.md`](architecture/VAULT-DESIGN.md) — SEP-56 vault mechanics, sleeve architecture, and share dilution.
- [`architecture/NAV-ORACLE.md`](architecture/NAV-ORACLE.md) — Multi-source pricing, circuit breakers, and YieldBlox exploit prevention rules.
- [`architecture/EXECUTION.md`](architecture/EXECUTION.md) — Flow-based rebalancing, keeper triggers, and DEX slippage caps.
- [`architecture/ACCOUNTS.md`](architecture/ACCOUNTS.md) — Passkey smart accounts, institutional multisig, fee sponsorship, and Bell features.

### 2. Smart Contracts Layer (`contracts/`)
- [`contracts/CONTRACTS-SPEC.md`](contracts/CONTRACTS-SPEC.md) — Full Soroban Rust contract specifications, structs, events, and traits.
- [`contracts/SEPS.md`](contracts/SEPS.md) — Ecosystem standards implementation: SEP-56, SEP-57, SEP-40, SEP-41, SEP-24.
- [`contracts/PROTOCOLS.md`](contracts/PROTOCOLS.md) — Protocol upgrade roadmap (Protocols 23–30 / Bell) and CAP technical details.

### 3. AI & Diligence Layer (`ai-engine/`)
- [`ai-engine/THESIS-ENGINE.md`](ai-engine/THESIS-ENGINE.md) — 6-stage AI pipeline (Parse → Retrieve → Optimise → Simulate → Explain → Sign).
- [`ai-engine/REGISTRY.md`](ai-engine/REGISTRY.md) — Curated closed-universe asset registry, diligence criteria, and jurisdiction matrices.

### 4. Business & Go-to-Market (`business/`)
- [`business/BUSINESS-MODEL.md`](business/BUSINESS-MODEL.md) — Revenue streams, pricing tiers, unit economics, and 24-month financial scenarios.
- [`business/GTM.md`](business/GTM.md) — Go-to-market strategy, buyer personas, 10-design-partner playbook, and utility metrics.

### 5. Compliance & Regulatory (`compliance/`)
- [`compliance/COMPLIANCE.md`](compliance/COMPLIANCE.md) — Software tooling posture, eligibility engine, KYC reuse, and asset controls.

### 6. Product Roadmap & Governance (`roadmap/`)
- [`roadmap/ROADMAP.md`](roadmap/ROADMAP.md) — Phased milestones (Phase 1 Testnet MVP to Phase 5 Scale) with strict gating criteria.
- [`roadmap/RISKS.md`](roadmap/RISKS.md) — Comprehensive risk register and active mitigations.
- [`roadmap/DECISIONS.md`](roadmap/DECISIONS.md) — Architectural Decision Records (ADRs D01–D09).

### 7. References & Ecosystem Intel (`references/`)
- [`references/COLLATERAL-PASSPORT.md`](references/COLLATERAL-PASSPORT.md) — Complete specification of Constella's collateral engine for lenders.
- [`references/RESEARCH-UPDATES.md`](references/RESEARCH-UPDATES.md) — Living log tracking post-Oct 30 2026 intelligence (Bell vote, DTCC status, JQQQ, TVL).
- [`references/ECOSYSTEM.md`](references/ECOSYSTEM.md) — Ecosystem map of RWA issuers, lending protocols, AMMs, and developer tooling.
- [`references/SOURCES.md`](references/SOURCES.md) — Primary citations, official documentation links, and audit reports.
- [`references/GLOSSARY.md`](references/GLOSSARY.md) — Comprehensive definitions of product, financial, and Soroban terms.

---

## Non-Negotiable Core Principles

1. **Non-Custodial Ownership:** Users or their designated smart accounts own their shares. Protocol admin keys CANNOT move user capital.
2. **AI Proposes; Contracts Dispose:** The AI model only drafts proposals. Only a cryptographically signed, bounded policy hash executes on-chain. Nothing in the contract layer trusts raw model outputs.
3. **Closed Universe Rule:** The model may ONLY select assets registered and active in the on-chain Diligence Registry. Hallucinated tickers cannot appear.
4. **Zero Gas for Users:** All transaction fees, trustline reserves, and state rents are abstracted away and sponsored by Constella.
5. **No Double Wrapping Fees:** Management fees stay ultralow (0.05%–0.25%/year) to prevent fee stacking over underlying fund expense ratios.
6. **Audited Foundation:** Never hand-roll novel vault accounting math. Build exclusively on OpenZeppelin's audited `stellar-contracts` v0.8.0 primitives.

---

## Key Milestone Gates & Dates (as of November 2026)

| Date | Milestone / Event | Significance |
|---|---|---|
| **03 Nov 2026** | Bell (Protocol 30) Testnet Vote | Successfully upgraded testnet validators to Protocol 30. |
| **23 Nov 2026 (17:00 UTC)** | Bell (Protocol 30) Mainnet Vote | Network vote for W-addresses (CAP-84) and ML-DSA (CAP-87). |
| **Late Nov 2026** | Phase 1 Testnet MVP Launch | End-to-end prototype live on Stellar Testnet with permissionless assets. |
| **Dec 2026** | SCF Round #47 Application | Formal submission to the Stellar Community Fund. |
| **Q1 2027** | External Audit & Mainnet Beta | Capped $1M mainnet launch with 5–10 anchor enterprise treasuries. |
| **H1 2027** | DTCC Tokenization Service Launch | DTCC brings Russell 1000 and ETF digital twins to Stellar; activates Equity sleeve. |
| **Q2 2027** | Partner W-Address Integrations | Rollout of Partner SDK allowing consumer wallets to route via Bell omnibus accounts. |
