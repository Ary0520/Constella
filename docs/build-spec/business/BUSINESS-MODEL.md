# Business Model & Economics

> Pricing tiers, revenue streams, unit economics, 24-month financial modeling, and defensible moat.

---

## 1. Core Monetization Principles

1. **Zero User Gas:** Transaction fees and trustline reserves are sponsored by Constella.
2. **Zero In/Out Friction:** No deposit fees, no redemption fees, no protocol-level bid/ask spread.
3. **Issuer Cost Pass-Through:** Underlying asset management fees (e.g. Franklin Templeton's or Ondo's expense ratios) are passed through at net cost with 100% transparency.
4. **Thin Wrapper Pricing:** Constella charges a low, predictable management fee (0.05%–0.25%/year) accrued via on-chain share dilution.
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
1. **Management Fee on AUM (Core):**
   - Accrues continuously on-chain via fractional share dilution.
   - Requires zero invoicing; non-custodial and programmatic.
2. **Treasury SaaS Subscriptions (High Margin):**
   - Monthly retainer for institutional features: private vault factory deployment, multi-entity accounting, automated tax-lot harvesting, SOC2-compliant audit logs, and dedicated keeper gas pools.
3. **Partner Platform Fees & Revenue Share (Scale Distribution):**
   - Wallets and fintechs embedding the Constella SDK pay a platform fee or share 30% of basket AUM fees.
   - Unlocked at massive scale via Bell's CAP-84 W-address omnibus accounts.
4. **Curator Marketplace (Ecosystem Growth):**
   - Community analysts and asset managers publish verified theses.
   - Curators can attach up to 0.30% add-on fee; Constella retains a 30% protocol cut.

---

## 3. Pricing Matrix

| Plan | Target Customer | Features Included | Fee Structure |
|---|---|---|---|
| **Explorer** | Individuals & retail testers | Unlimited drafts & simulations, 1 active basket up to $25k | 0.20% / yr on AUM (No SaaS fee) |
| **Pro** | Power users & angels | Unlimited baskets, custom rebalance bands, CSV exports | 0.15% / yr + $19 / month |
| **Treasury** | Startups, DAOs, VC Funds, Foundations | Private vaults, multisig & ML-DSA, tax-lot accounting, API access | $199–$499 / mo + 0.10% / yr (falls to 0.05% for AUM > $5M) |
| **Partner** | Wallets, Anchors, Fintechs | White-label UI, SDK access, W-address omnibus, priority routing | 0.10% / yr + 30% rev-share to partner (or flat enterprise licence) |
| **Curator** | Strategy publishers | Public thesis publishing, distribution channel | Curator sets up to 0.30%; Constella takes 30% |

**Hard Ceiling:** All-in Constella protocol fees never exceed **0.35% / year**.

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
