# Living Research & Ecosystem Updates Log

> Tracked intelligence and ecosystem changes occurring after the foundational research document (dated 30 October 2026). All agents must check here for current figures.  
> **Last Verified:** 14 November 2026

---

## 1. Protocol 30 ("Bell") Upgrade Status

| Milestone | Date | Status | Technical Details |
|---|---|---|---|
| Core v30.0.0 Release | 16 Oct 2026 | ✅ Complete | Includes CAP-84, CAP-87, CAP-88 |
| Testnet Upgrade Vote | 03 Nov 2026, 17:00 UTC | ✅ Passed | Testnet successfully running Protocol 30 |
| **Mainnet Upgrade Vote** | **23 Nov 2026, 17:00 UTC** | ⏳ **PENDING (9 days)** | Validator coordination underway |

### Impact on Constella Build
- v1 core implementation remains strictly decoupled from Bell.
- Client SDKs (`@stellar/stellar-sdk`) and contract crates (`soroban-sdk`) must be updated to v30-compatible builds immediately to prevent parsing errors when CAP-88 millisecond timestamps go live.
- W-Address omnibus accounts and ML-DSA signature support are scheduled as additive Phase 2 modules following the Nov 23 vote.

---

## 2. Stellar DeFi TVL Adjustments

| Metric | Source Doc (Oct 30) | Verified (Nov 14) | Variance / Analysis |
|---|---|---|---|
| **Stellar Total DeFi TVL** | ~$265M–$273M | **~$195M–$199M** | -27% (Correction following August AMM liquidity spike; Blend remains dominant) |
| **Blend Protocol TVL** | ~$165M | **~$140M–$145M** | Primary lending venue on Stellar |
| **DeFindex TVL** | ~$20M | **~$18.5M** | Stable yield aggregator volume |
| **Soroswap 24h Volume** | ~$1.2M TVL | **~$1.6M Daily Vol** | Consistent AMM activity |

---

## 3. Real-World Asset (RWA) Developments

### A. Franklin Templeton (BENJI)
- **Milestone:** BENJI has officially surpassed **$1.1B in total on-chain AUM**, maintaining its position as one of the largest tokenized government funds in the global blockchain ecosystem.
- **Multichain:** Active on both Stellar and BNB Chain.

### B. Spiko (USD & EUR T-Bills)
- **Status:** Spiko manages over **$3B in tokenized assets**, with approximately 90% held by enterprise treasuries, DAOs, venture funds, and public institutions.
- **Constella Synergy:** Spiko's customer base matches Constella's exact buyer profile for Phase 1/Phase 2 launch.

### C. Janus Henderson / Centrifuge (JQQQ)
- **Status Update:** The original draft noted "subscriptions not yet open." **JQQQ is now officially LIVE and accepting subscriptions.**
- **Details:** First blockchain-native tokenized Nasdaq-100 tracker. Minimum ticket size is $100,000 for non-US qualified professional investors.
- **Inclusion:** Added to Constella's Gated Equities candidate registry for institutional treasuries.

### D. DTCC Tokenization Service (DTS)
- **Launch Date:** Officially launched on **28 October 2026**.
- **Rollout Phase:** Initial deployment on Canton Network and an enterprise Besu private network.
- **Stellar Roadmap:** Public network expansion to Stellar remains targeted for **H1 2027** under an SEC No-Action letter framework.
- **Asset Classes:** Digital twin representations of Russell 1000 equities, major index ETFs, and US Treasuries.

---

## 4. Developer Tooling & OpenZeppelin v0.8.0

In late October 2026, OpenZeppelin released `stellar-contracts` **v0.8.0**, introducing pivotal primitives for Constella:
1. **Audited SEP-56 Vault & SEP-57 RWA Modules:** Core primitives ready for production integration.
2. **Confidential Token Standard:** Native implementation of Pedersen commitments and UltraHonk ZK proofs (via Noir circuits) running on Soroban. Accelerates our ZK eligibility proof roadmap.
3. **RWA Compliance Toolkit:** Standardized interfaces for `AUTH_REQUIRED`, issuer clawbacks, and frozen account management.
4. **Smart Account Framework:** Turnkey passkey (secp256r1) signing and fee abstraction utilities.

---

## 5. Stellar Community Fund (SCF) Status

- **SCF Round #45:** Concluded 2 October 2026; distributed ~$4.04M in XLM across 40 projects (including Liqvid's RWA lending protocol).
- **SCF Round #46:** Applications closed 9 November 2026; currently under community review.
- **SCF Round #47:** Opening target late November / early December 2026. Constella application dossier is being prepared for Round #47.
