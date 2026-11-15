# Open Architecture & Strategic Decisions (ADR Register)

> Living register of critical product, legal, and engineering decisions. All agents must consult this before making architectural trade-offs.

---

## Decision Summary Table

| ID | Title | Status | Recommendation | Deadline |
|---|---|---|---|---|
| **D01** | Beachhead: Treasuries vs. Partner Wallets | ⏳ Open | **Treasuries First** | Pre-Phase 1 Launch |
| **D02** | Regulatory Path: Self-Directed vs. Licensed MoR | ⏳ Open | **Tooling Only (v1)**; evaluate MoR later | Pre-Phase 3 Mainnet |
| **D03** | Product Brand & Trademark | ⏳ Open | Retain "Constella" as working title; run search | Pre-Public Launch |
| **D04** | Commercial Relationship with Ondo Finance | ⏳ Open | **Asset Supplier First**; compete on collateral | Pre-Ondo Outreach |
| **D05** | First Lending Integration Partner | ⏳ Open | **Liqvid first** (co-design), then Blend (scale) | Dec 2026 (Phase 2) |
| **D06** | First Permissioned Issuer for Allow-Listing | ⏳ Open | **Spiko first** (largest RWA issuer on Stellar) | Dec 2026 (Phase 2) |
| **D07** | Fee Sponsorship Capital Source | ⏳ Open | Seed capital / SCF award XLM allocation | Pre-Testnet Launch |
| **D08** | Keeper Network Architecture | ⏳ Open | Constella-operated (v1) → Multi-keeper (Phase 4) | Phase 4 Planning |
| **D09** | ZK Eligibility Proof Feasibility | ⏳ Open | Spike OpenZeppelin v0.8.0 UltraHonk ZK module | Pre-Phase 3 Scope |

---

## Detailed Decision Records

### [D01] Beachhead: Treasury-First vs. Partner-First

- **Context:** Deciding which customer segment receives our primary focus during initial testnet and mainnet beta.
- **Options:**
  1. **Treasuries First:** Focus on crypto startups, DAOs, and VC funds needing automated runway management.
  2. **Partner-First (Wallets/Anchors):** Focus on consumer wallets embedding savings baskets.
- **Recommendation:** **Treasuries First.**
  - **Rationale:** Treasuries have larger tickets ($250k+), higher willingness to pay monthly SaaS fees ($199–$499/mo), and urgent pain around runway management. They provide high-bandwidth product feedback. Partner integrations require longer sales cycles and lower margins.
- **Status:** ⏳ Open (Recommended direction: Treasuries First).

---

### [D02] Regulatory Path: Tooling vs. Early Manager of Record (MoR)

- **Context:** How Constella structures its legal entity and user terms regarding portfolio assembly and rebalancing.
- **Options:**
  1. **Software Tooling Posture:** Non-custodial software provider for self-directed entities. No discretionary management.
  2. **Early Licensed Manager of Record (MoR):** Form or partner with a regulated entity to act as an investment adviser from Day 1.
- **Recommendation:** **Software Tooling Posture for Launch.**
  - **Rationale:** Launching as non-custodial tooling allows fast iteration and testnet deployment. We can partner with a licensed MoR later when distributing to European retail under MiCA.
- **Status:** ⏳ Open (Commission formal legal opinion prior to Phase 3).

---

### [D03] Name & Branding Compliance

- **Context:** "Constella" is currently a working name. Stellar Development Foundation (SDF) brand policies restrict using "Stellar" in naming to avoid confusing brand association.
- **Actions Required:**
  1. Conduct international trademark clearance for "Constella" in financial software classes.
  2. Confirm compliance with SDF trademark guidelines.
  3. Secure primary web domains (`.app`, `.finance`, `.io`).
- **Deadline:** Prior to public website launch and PR campaigns.
- **Status:** ⏳ Open.

---

### [D04] Relationship with Ondo Finance: Supplier vs. Competitor

- **Context:** Ondo launched Intelligent Portfolios on Ethereum/Solana in late 2026. Ondo's tokenized dollar (USDY) is live on Stellar.
- **Options:**
  1. **Directly Compete:** Position Constella as an Ondo competitor and avoid listing Ondo assets.
  2. **Supplier & Packaging Partner:** Include Ondo USDY as a premier constituent in Constella baskets.
- **Recommendation:** **Supplier Partner First.**
  - **Rationale:** Constella provides personalization, Stellar-native basket wrappers, and the Collateral Passport — capabilities Ondo Intelligent Portfolios does not currently offer on Stellar. Using USDY increases basket yield while leveraging Ondo's institutional brand.
- **Status:** ⏳ Open.

---

### [D05] First Lending Protocol Integration

- **Context:** We need at least one lending market to accept Constella basket shares as collateral during Phase 3.
- **Candidates:**
  - **Liqvid (SCF #45):** Purpose-built RWA collateral money market on Stellar.
  - **Blend Protocol:** Stellar's largest lending protocol (~$105M TVL on Blend Pools V2, 15 Nov 2026).
  - **Lucent Finance:** Emerging institutional money market.
- **Recommendation:** **Liqvid First (Co-Design Partner), Blend Second (Scale).**
  - **Rationale:** Liqvid is purpose-built for RWA collateral and has strong incentive to co-design the Collateral Passport interface. Once proven on Liqvid, we bring the listing kit to Blend pool operators for scale.
- **Status:** ⏳ Open.

---

### [D06] First Permissioned Issuer for Contract Allow-Listing

- **Context:** Regulated assets require issuer allow-listing of the vault contract address.
- **Candidates:**
  - **Spiko:** Represents ~39% of Stellar RWA value; institutional focus, rapid growth.
  - **Ondo Finance (USDY):** Established on-chain compliance; non-US retail/professional focus.
  - **Franklin Templeton (BENJI):** >$1.1B AUM; premier institutional brand.
- **Recommendation:** **Spiko First, Ondo Second, Franklin Templeton Stretch Goal.**
  - **Rationale:** Spiko's institutional client base directly overlaps with our launch treasuries, and their team is highly responsive to Stellar ecosystem integration.
- **Status:** ⏳ Open.

---

### [D07] Fee Sponsorship Capital Allocation

- **Context:** Constella sponsors all user fees and reserves. A sponsor wallet must hold sufficient XLM.
- **Options:**
  1. Fund 100% from founder capital.
  2. Earmark a dedicated 15% tranche of any received SCF grant awards into the sponsor balance.
  3. Maintain protocol treasury reserves in XLM to fund transaction fees natively.
- **Recommendation:** Combination of **Founder capital for Testnet** + **SCF Grant allocation for Mainnet**.
- **Status:** ⏳ Open.

---

### [D08] Keeper Architecture Evolution

- **Context:** Rebalancing and Soroban state TTL extensions require automated transaction submission.
- **Evolution Plan:**
  - **Phase 1–2:** Single Constella-operated TypeScript keeper.
  - **Phase 3:** Redundant multi-region keeper nodes running behind rate-limited RPCs.
  - **Phase 4+:** Permissionless / decentralized keeper network with stake incentives and slashing.
- **Status:** ⏳ Open (Architecture documented; implementation phased).

---

### [D09] ZK Eligibility Proofs Spike (OpenZeppelin v0.8.0)

- **Context:** The original research roadmap placed zero-knowledge eligibility proofs in late 2027. However, OpenZeppelin's October 2026 release of `stellar-contracts` v0.8.0 included an unexpected **Confidential Token Standard** featuring Pedersen commitments and UltraHonk ZK proofs (Noir circuits).
- **Action:** Execute a technical spike in Sprint 2 to evaluate whether OpenZeppelin's ZK module can be adapted for private compliance credentials ahead of schedule.
- **Deadline:** Prior to Phase 3 scope freeze.
- **Status:** ⏳ Open (Technical Spike Scheduled).
