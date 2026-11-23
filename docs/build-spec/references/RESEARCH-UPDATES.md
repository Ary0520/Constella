# Living Research & Ecosystem Updates Log

> Tracked intelligence and ecosystem changes occurring after the foundational research document (dated 30 October 2026). All agents must check here for current figures.  
> **Last Verified:** 24 November 2026 (Bell section); 15 November 2026 (others)

---

## 1. Protocol 30 ("Bell") Upgrade Status

| Milestone | Date | Status | Technical Details |
|---|---|---|---|
| Core v30.0.0 Release | 16 Oct 2026 | ✅ Complete | Includes CAP-84, CAP-87, CAP-88 |
| Testnet Upgrade Vote | 03 Nov 2026, 17:00 UTC | ✅ Passed | Testnet successfully running Protocol 30 |
| **Mainnet Upgrade Vote** | **23 Nov 2026, 17:00 UTC** | ✅ **Passed** | Mainnet ledger 65633432 closed 17:00:06Z on protocol 30 (Horizon: core 30.0.0). Testnet switched at ledger 5540313, 3 Nov 17:00:05Z. Verified from ledger data 23 Nov; no SDF announcement yet at that time |

### What Protocol 30 changed (verified 24 Nov 2026)
- **CAP-84 W-addresses (muxed contract accounts):** new `SCAddress` arm = contract id + u64 id. Valid only as a transfer DESTINATION (SAC `transfer` `to`; not `mint`, not `transfer_from`, never a source, never a storage key, rejected where a plain `Address` is expected). Events carry the C address plus `to_muxed_id`. OZ 0.8.0 fungible `transfer` already takes `MuxedAddress` and our vault emits `to_muxed_id` (MuxedTransfer).
- **CAP-87 ML-DSA:** host fns `verify_sig_ml_dsa_44/65/87` (FIPS 204 pure form, context string 0-255 B); soroban-sdk 30 `env.crypto().ml_dsa_44_verify(...)`. Keys 1312/1952/2592 B, signatures 2420/3309/4627 B. Transaction envelopes stay Ed25519.
- **CAP-88 millisecond close time:** ledger header change; Soroban `ledger_timestamp` stays in seconds.
- Sources: stellar.org/blog/developers/bell-protocol-30-upgrade-guide; stellar/stellar-protocol core CAPs; Horizon mainnet/testnet ledgers.

### Impact on Constella Build (24 Nov 2026)
- **Nothing breaks:** contracts built with soroban-sdk 28 keep running; our clients are already P30-ready: `@stellar/stellar-sdk` 17.4.0 (W support needs >= 17.3.0; Wallets Kit 2.8.0 dedupes to 17.4.0), Python `stellar-sdk` 17.0.1, stellar-cli 30.0.1.
- **soroban-sdk 30 is still rc** (30.0.0-rc.1, 21 Oct) - needed only to call ML-DSA. OpenZeppelin stellar-contracts has no release after 0.8.0 and no ML-DSA verifier.
- **W-address partner omnibus** (founding doc 4.8): additive Phase 2 module; first a testnet test that our sdk-28 vault credits a W destination correctly (unverified).
- **ML-DSA admin keys for treasury vaults** (founding doc 4.8): needs our own verifier contract on soroban-sdk 30 stable; signatures 2.4-4.6 KB raise resource fees. Planned for the mainnet track.

---

## 2. Stellar DeFi TVL Adjustments

| Metric | Source Doc (Oct 30) | Verified (Nov 14) | Variance / Analysis |
|---|---|---|---|
| **Stellar Total DeFi TVL** | ~$265M–$273M | **~$198M (15 Nov)** | -27% (Correction following August AMM liquidity spike; Blend remains dominant) |
| **Blend Protocol TVL** | ~$165M | **~$105M (Blend Pools V2, DefiLlama, 15 Nov)** | Primary lending venue on Stellar; ~$113M on 9 Nov, ATH $173M on 1 Oct |
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
- **SCF Round #46:** Build submissions closed 8 Nov 2026; panel review 11–18 Nov; community vote 23 Nov – 3 Dec 2026 (communityfund.stellar.org/awards).
- **SCF Round #47:** No official dates published as of 15 Nov 2026 (unverified). Constella application dossier is being prepared for Round #47.

---

## 6. Verification Pass — 15 November 2026

| Topic | Verified fact | Source |
|---|---|---|
| `soroban-sdk` | Latest stable **29.0.1**; **30.0.0-rc.1** (21 Oct) is the only Protocol 30 build — do not pin an RC before the Bell vote passes. 28.0.0 released 18 Sep for Protocol 28. | crates.io/crates/soroban-sdk |
| OpenZeppelin `stellar-contracts` | **v0.8.0 released 20 Oct 2026**, audit report in repo. Crates: `stellar-tokens` (fungible, vault, rwa), `stellar-accounts`, `stellar-access`, `stellar-contract-utils`, `stellar-macros`, `stellar-confidential`. Workspace pins **`soroban-sdk = 28.0.0`**. v0.7.3 is a security fix for smart-account delegated signers (GHSA-jpg9-m7w4-c7wc). | github.com/OpenZeppelin/stellar-contracts/releases/tag/v0.8.0 |
| SEP status | SEP-40 Draft v0.1.0, SEP-41 Draft v0.5.2, SEP-56 Draft v0.1.2, SEP-57 Draft v0.4.0. | github.com/stellar/stellar-protocol/tree/master/ecosystem |
| CAP-85 | "Externally managed contract executables" — shipped in **Protocol 28** (Final). No `stellar contract deploy --upgrade-fleet` flag exists in stellar-cli (latest v30.0.1, 3 Nov). | stellar-protocol/core/cap-0085.md; github.com/stellar/stellar-cli |
| Protocol 29 | No official name, date, or CAPs found; stellar-cli jumped v28.1.0 → v30.0.1. Treat as **unverified**. | stellar.org/protocol-upgrades |
| Bell mainnet vote | 23 Nov 2026, 17:00 UTC (subject to change). CAP-84 (`W` muxed contract strkey), CAP-87 (ML-DSA), CAP-88 (ms close time) are real. | stellar.org/protocol-upgrades |
| YieldBlox | 22 Feb 2026; USTRY pushed ~$1.05 → ~$106.7 on SDEX, reported via Reflector into a Blend V2 community pool; loss ≈ $10.2–10.97M; pool/oracle configuration, not a Blend core bug. | blocksec.com; rekt.news/yieldblox-rekt |
| RedStone | Mainnet feeds live 4 Mar 2026; SEP-40 compatible. | blog.redstone.finance |
| Blend oracle usage | Blend v2 calls only `lastprice(Asset::Stellar(addr))` via the `sep-40-oracle` crate; rejects price ≤ 0 or timestamps older than 24h. | github.com/blend-capital/blend-contracts-v2 |
