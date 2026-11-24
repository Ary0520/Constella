# Business Model & Economics

> Pricing tiers, revenue streams, unit economics, 24-month financial modeling, and defensible moat.

---

## 1. Core Monetization Principles

1. **Zero User Gas:** Transaction fees and trustline reserves are sponsored by Constella.
2. **Zero In/Out Friction:** No deposit fees, no redemption fees, no protocol-level bid/ask spread.
3. **Issuer Cost Pass-Through:** Underlying asset management fees (e.g. Franklin Templeton's or Ondo's expense ratios) are passed through at net cost with 100% transparency.
4. **Thin Wrapper Pricing (decision D-D, 16 Nov 2026):** Constella charges a **performance fee of 10 % of the profit** a vault earns above its high-water mark — nothing on principal, nothing when the vault earns nothing — with an **all-in ceiling of 0.20 % / year** of assets, enforced in the vault contract. Accrued via on-chain share dilution, at most once per day.
5. **Free Thesis Discovery:** Unlimited AI thesis generations and simulation scenarios are always free.

---

## 2. Revenue Streams

```
                       ┌───────────────────────────────┐
                       │   Constella Revenue Engine    │
                       └──────────────┬────────────────┘
                                      │
     ┌──────────────────┬─────────────┴────────────┬──────────────────┐
     ▼                  ▼                          ▼                  ▼
1. Management      2. Treasury SaaS          3. Partner B2B      4. Curator
   Fee (AUM)          Subscriptions             Platform Fees       Marketplace
   0.05%–0.25%/yr     $199–$499/month           Licensing + rev     30% cut of
   dilution-based     private vaults, audits    share (post-Bell)   curator markup
```

### Stream Details
1. **Performance Fee (Core, D-D):**
   - 10 % of profit above the per-share high-water mark (conservative NAV); capped at 0.20 % of assets per year all-in; crystallizes at most daily via share dilution.
   - Why: Phase-1 yields are low (Blend V2 average supply APY ≈ 2.4 %, DefiLlama Nov 2026); a 0.20 % AUM fee would take most of a cash-heavy holder's yield. Competitors charge on yield (Morpho curators 0–20 % of interest; many vaults 0 % management fee).
   - Requires zero invoicing; non-custodial and programmatic. Revenue in Phase 1 is correspondingly small; it grows with yield-bearing sleeves (T-bills in Phase 3).
2. **Treasury SaaS Subscriptions (High Margin):**
   - Monthly retainer for institutional features: private vault factory deployment, multi-entity accounting, automated tax-lot harvesting, SOC2-compliant audit logs, and dedicated keeper gas pools.
3. **Partner Platform Fees & Revenue Share (Scale Distribution):**
   - Wallets and fintechs embedding the Constella SDK pay a platform fee or share 30% of the vault performance fee.
   - Unlocked at massive scale via Bell's CAP-84 W-address omnibus accounts.
4. **Curator Marketplace (Ecosystem Growth):**
   - Community analysts and asset managers publish verified theses.
   - A curator's share comes **out of** the same 10 % performance fee (never on top); the 0.20 %/yr all-in ceiling always holds.

---

## 3. Pricing Matrix

| Plan | Target Customer | Features Included | Fee Structure |
|---|---|---|---|
| **Explorer** | Individuals & retail testers | Unlimited drafts & simulations, 1 active basket up to $25k | 10% of profit, max 0.20% / yr (No SaaS fee) |
| **Pro** | Power users & angels | Unlimited baskets, custom rebalance bands, CSV exports | 10% of profit, max 0.20% / yr + $19 / month |
| **Treasury** | Startups, DAOs, VC Funds, Foundations | Private vaults, multisig & ML-DSA, tax-lot accounting, API access | $199–$499 / mo + performance fee set per vault (≤ 10% of profit; e.g. 5% above $5M AUM). The rate is set by Constella: factory vaults get the default 10 %; a discount needs Constella's co-signature on `create_vault` (D-O) |
| **Partner** | Wallets, Anchors, Fintechs | White-label UI, SDK access, W-address omnibus, priority routing | 30% of the performance fee shared with the partner (or flat enterprise licence) |
| **Curator** | Strategy publishers | Public thesis publishing, distribution channel | Curator's share comes out of the 10% performance fee |

**Hard Ceiling (D-D, enforced on chain):** fees never exceed **10% of profit** and never **0.20% of assets per year**, all-in. Issuer fund fees are passed through at cost and shown separately.

---

## 4. Unit Economics Deep Dive

### A. Retail / Consumer via Partner (Average Ticket: $1,500)
- **Gross Revenue (0.20%):** $3.00 / year
- **Sponsored Gas (~12 ops/yr @ Soroban rates):** $0.15
- **Allocated AI Inference Cost (LangChain + GPT-4o-mini):** $0.15
- **Allocated Indexing & RPC Overhead:** $0.30
- **KYC Amortization (Reused via Issuer/Partner):** $0.00 ($1.50 if non-reusable)
- **Net Margin:** ~$2.40 / year (~80% margin when KYC is reused)
- **Takeaway:** Retail is purely an AUM volume engine, not a high-margin cash cow. KYC reuse is mandatory.

### B. Institutional Treasury Customer ($250,000 Initial AUM)
- **Gross Revenue:**
  - AUM Fee (0.10%): $250 / year
  - Mid-tier SaaS Subscription ($299/mo): $3,588 / year
  - **Total Revenue:** **$3,838 / year**
- **Direct Cost to Serve (Custom RPC, monitoring, dedicated keeper):** ~$350 / year
- **Net Contribution Margin:** **~$3,488 / year (~91% margin)**
- **Takeaway:** Treasury SaaS is the operational profitability engine. 50 active treasuries fully sustain a core engineering team.

---

## 5. 24-Month Projections (From Testnet Launch)

| Metric | Conservative | Base Case (5% Stellar RWAs) | Upside Case |
|---|---|---|---|
| **Platform AUM** | **$60,000,000** | **$200,000,000** | **$600,000,000** |
| Blended AUM Fee | 0.15% | 0.14% | 0.12% |
| **Annual AUM Revenue** | $90,000 | $280,000 | $720,000 |
| Active Treasury Clients | 80 ($1,200/yr avg) | 250 ($1,800/yr avg) | 600 ($2,400/yr avg) |
| **Treasury SaaS Revenue** | $96,000 | $450,000 | $1,440,000 |
| Partner B2B Licences | 1 ($60,000) | 3 ($225,000) | 8 ($800,000) |
| **Total Annualized Revenue** | **$246,000** | **$955,000** | **$2,960,000** |
| Core Team & OpEx Run-Rate | ~$650,000 | ~$750,000 | ~$1,100,000 |
| **Net Operational Cashflow** | (-$404,000) *(Funded via SCF/Grants)* | **+$205,000** *(Self-sustaining)* | **+$1,860,000** *(Expansion)* |

---

## 6. Year 1 Capital Strategy

1. **Stellar Community Fund (SCF):**
   - **Build Awards:** Up to $150,000 in XLM per milestone.
   - **Integration Awards:** For native integrations with Blend, DeFindex, Soroswap, and Stellar Wallets Kit.
   - **Next Action:** Prepare application for SCF Round #47 immediately following Round #46 reviews.
2. **SDF Matching Fund:**
   - 1:1 match of institutional lead investor capital up to $500,000.
3. **Design-Partner Pilot Retainers:**
   - Paid beta agreements with 5 anchor crypto treasuries offering lifetime discounted rates in exchange for feedback and case studies.

---

## 7. The Competitive Moat

1. **The Diligence Registry:** Documenting oracle models, redemption queuing, and regulatory edge-cases across 15+ Stellar RWA issuers is a massive operational moat that competitors cannot easily copy.
2. **Issuer Allow-Listing Exclusivity:** Getting a smart contract whitelisted by institutions like Franklin Templeton, Ondo, and Spiko takes months of compliance and legal coordination.
3. **Lender Acceptance Network Effects:** As more lending pools (Blend, Liqvid) accept Constella vault shares as collateral, our tokens gain liquidity and borrowing utility, locking in AUM.
4. **Incident-Free Risk Track Record:** In a post-YieldBlox DeFi ecosystem, treasuries prioritize battle-tested safety mechanisms above raw yield chasing.
