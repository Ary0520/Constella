# Asset Registry — Specification

> Reference: Product doc §2.3 Asset Universe; `contracts/CONTRACTS-SPEC.md` Registry Contract

---

## Registry Purpose

The registry is the **closed universe**. No asset can appear in any Constella basket unless it is:
1. In the on-chain registry contract
2. Active (not deactivated)
3. The user is eligible to hold it

The registry is both on-chain (contract) and off-chain (PostgreSQL + vector search index for the thesis engine).

---

## Asset Addition Process

Before any asset is added to the registry:

```
1. Submit issuer diligence form → off-chain review
2. Verify on-chain contract address on Stellar mainnet
3. Document oracle sources and test deviation behavior
4. Document transfer rules (allow-list, clawback, freeze)
5. Begin issuer allow-listing conversation (for permissioned assets)
6. Add to registry contract (requires admin multisig + legal/compliance sign-off)
7. Add to vector search index with metadata and description
8. Update this file + notify all agents via research-updates/LOG.md
```

**An asset that skips any step is NOT in the registry.**

---

## Current Asset Universe (November 2026)

### Sleeve: CASH_STABLE
Permissionless. Instant liquidity. Numéraire candidates.

| Asset ID | Asset | Issuer | Permissioned? | Liquidity Tier | Notes |
|---|---|---|---|---|---|
| `USDC` | USD Coin | Circle | No | 0 (instant) | Primary numéraire |
| `EURC` | Euro Coin | Circle | No | 0 (instant) | EUR numéraire |

### Sleeve: USD_GOV_FUNDS
Tokenized US government money-market / T-bill funds.

| Asset ID | Asset | Issuer | Permissioned? | Liquidity Tier | Notes |
|---|---|---|---|---|---|
| `FRANKLIN_BENJI` | BENJI | Franklin Templeton | Yes (KYC) | 1 (24h) | >$1.1B AUM Nov 2026; on Stellar + BNB Chain |
| `ONDO_USDY` | USDY | Ondo | Yes (KYC) | 1 (24h) | Yield-bearing dollar; non-US focus |
| `WISDOMTREE_USD` | WisdomTree USD | WisdomTree | Yes (KYC) | 1 (24h) | — |
| `SPIKO_USD` | Spiko USD T-bills | Spiko | Yes (KYC) | 1 (24h) | Largest Stellar RWA issuer |
| `M1X_USDM1` | USDM1 | M1X | Yes (institutional) | 0–1 | Backed 1:1 by US Treasury; hourly repo roll |

### Sleeve: EUR_SOVEREIGN
Euro and local-currency sovereign bond funds.

| Asset ID | Asset | Issuer | Permissioned? | Liquidity Tier | Notes |
|---|---|---|---|---|---|
| `SPIKO_EUR` | Spiko EUR T-bills | Spiko | Yes (KYC) | 1 (24h) | — |
| `ARCHAX_EGOVY` | €GOVY | Archax | Yes | 1–2 | Announced at Meridian Oct 2026 |
| `ETHERFUSE_CETES` | Cetes stablebonds | Etherfuse | Yes | 2 (1 week) | Mexican gov bonds held at BBVA |

### Sleeve: YIELD_DOLLAR
Yield-bearing dollar tokens (above money-market yield).

| Asset ID | Asset | Issuer | Permissioned? | Liquidity Tier | Notes |
|---|---|---|---|---|---|
| `FIGURE_YLDS` | YLDS | Figure Markets | Yes (allow-list) | 1 (24h) | Live since May 2026; Figure is a Tier 1 Stellar validator |
| `FIGURE_WYLDS` | wYLDS | Figure Markets | No (planned) | 1 (24h) | Permissionless wrapper — not yet launched as of Nov 2026 |

### Sleeve: LENDING
DeFi lending yield on Stellar.

| Asset ID | Asset | Protocol | Permissioned? | Liquidity Tier | Notes |
|---|---|---|---|---|---|
| `BLEND_USDC_SUPPLY` | Blend USDC supply | Blend | No | 1 (24h) | Dominant Stellar lending protocol; ~$195M TVL |
| `DEFINDEX_USDC` | DeFindex USDC | DeFindex | No | 1 (24h) | Yield aggregator; ~$18.5M TVL |

Higher yield, higher risk. Hard-capped at 15% in any basket by default.

### Sleeve: EQUITY *(future, gated)*
| Asset ID | Asset | Issuer | Permissioned? | Liquidity Tier | Notes |
|---|---|---|---|---|---|
| `JH_JQQQ` | JQQQ | Janus Henderson / Centrifuge | Yes (non-US professional, min $100k) | 2 (1 week) | Nasdaq-100; subscriptions open as of Nov 2026 |
| `DTC_RUSSELL1000` | DTC Russell 1000 | DTCC DTS | Yes (institutional) | 1–2 | H1 2027 target; not yet on Stellar |
| `DTC_ETF_*` | DTC ETF tokens | DTCC DTS | Yes (institutional) | 1–2 | H1 2027 target |

**Equity sleeve disabled in v1.** Enable only when DTC-tokenized assets are confirmed live on Stellar.

### Sleeve: XLM *(optional)*
| Asset ID | Asset | Notes |
|---|---|---|
| `XLM` | Stellar Lumens | Native; permissionless; hard-capped at 5% if included |

---

## Issuer Concentration Policy

| Constraint | Default | Configurable by user? |
|---|---|---|
| Max single issuer | 30% | Yes, in signed policy |
| Max single asset | 25% | Yes, in signed policy |
| Spiko-specific warning | Show at 30% | Always shown (issuer concentration risk) |
| Illiquid assets in liquid baskets | 0% | No — hard rule |

---

## Jurisdiction and Investor Class Matrix

| Asset | US persons | Non-US retail | Non-US professional | EU professional | Notes |
|---|---|---|---|---|---|
| USDC / EURC | ✅ | ✅ | ✅ | ✅ | Permissionless |
| BENJI | ✅ (with KYC) | ✅ (with KYC) | ✅ | ✅ | Franklin allow-list |
| USDY | ❌ (US restriction) | ✅ | ✅ | ✅ | Non-US focus |
| Spiko | varies | varies | ✅ | ✅ | EU-regulated |
| YLDS | ✅ | varies | varies | varies | Check current terms |
| JQQQ | ❌ (non-US only) | ❌ (professional only) | ✅ (min $100k) | ✅ | Non-US professional |
| DTC stocks | varies | varies | ✅ | ✅ | H1 2027 |

**Always verify current issuer terms. This table is a snapshot, not legal advice.**

---

## Registry Update Protocol

When a new asset is confirmed for the registry, or an existing asset's status changes:

1. Update this file (asset-registry entry)
2. Update `research-updates/LOG.md` with the date and what changed
3. Update the on-chain registry contract (admin multisig required)
4. Update the vector search index
5. Notify any active agents referencing the registry

**Never add an asset to the thesis engine's search index without also adding it to the on-chain contract first.**
