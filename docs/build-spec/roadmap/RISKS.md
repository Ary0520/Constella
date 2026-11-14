# Risk Register & Mitigations

> Comprehensive risk assessment covering technical, market, regulatory, and protocol vulnerabilities.

---

## Risk Summary Matrix

```
  HIGH  │ [R03] Oracle Failure       [R02] Issuer Allow-List Block    [R05] Regulatory Drift
        │ [R04] Thin DEX Slippage
        │
 IMPACT │ [R08] Smart Contract Bug   [R07] Issuer Concentration       [R09] Protocol Upgrade Slippage
        │
  LOW   │ [R10] Sponsor Gas Drain    [R11] AI Hallucination           [R12] Simulation Misinterpretation
        └────────────────────────────────────────────────────────────────────────────────────────────►
          LOW                               PROBABILITY                                    HIGH
```

---

## Detailed Risk Assessments

### [R01] Asset Universe Too Small or Illiquid
- **Risk:** Most institutional RWAs on Stellar are heavily gated, and on-chain equities are limited ($26M across 6 instruments in Q2).
- **Probability:** Medium | **Impact:** High
- **Mitigation:**
  - Launch with battle-tested liquid sleeves: cash (USDC/EURC), government T-bills (BENJI, Spiko), and vetted lending pools (Blend).
  - Explicitly mark the equity module as gated on DTCC mainnet arrival (H1 2027).
  - Never allow illiquid private credit into liquid treasury baskets.

---

### [R02] Issuers Refuse to Allow-List Vault Contract
- **Risk:** Regulated issuers (e.g., Franklin Templeton, Figure) operate strict permissioning and may hesitate to approve smart contracts as omnibus holders.
- **Probability:** Medium | **Impact:** Critical
- **Mitigation:**
  - Lead with issuers already built for third-party DeFi integration (Spiko, Ondo).
  - Provide issuers with a distribution thesis: Constella is an inbound pipeline for recurring institutional sticky capital, not an arbitrage bot.
  - Implement SEP-57 compliant gated share classes with transparent pass-through KYC attestations so issuers maintain 100% regulatory oversight.
  - Maintain a permissionless-only fallback basket (USDC + EURC + Blend) that requires zero issuer permissions.

---

### [R03] Oracle Misconfiguration or Price Manipulation
- **Risk:** The YieldBlox incident ($10M drain in Feb 2026) demonstrated that pricing collateral off thin orderbooks enables catastrophic exploits.
- **Probability:** Low | **Impact:** Catastrophic
- **Mitigation:**
  - **Rule 1:** Fund tokens MUST use issuer-published NAV APIs as primary.
  - **Rule 2:** Thin AMM or SDEX orderbooks are ignored unless they exceed strict depth floors (e.g. $100k+).
  - **Rule 3:** Collateral NAV is strictly the **minimum** of all valid sources; deposit NAV is the **median**.
  - **Rule 4:** Deviation circuit breakers automatically pause the affected sleeve if sources diverge by > 2–3%.
  - **Rule 5:** Hard halt if fewer than 2 valid, non-stale oracle feeds remain.

---

### [R04] Thin DEX Liquidity Causing Rebalance Slippage
- **Risk:** Stellar DEX TVL is ~$16M and Soroswap TVL is ~$1.2M; executing large portfolio rebalances on AMMs could incur excessive price impact.
- **Probability:** High | **Impact:** Medium
- **Mitigation:**
  - Prioritize **flow-based netting**: direct new inbound deposits to underweight sleeves and outbound redemptions to overweight sleeves.
  - Execute primary market mint/redeem directly with issuers at NAV for size.
  - Restrict AMM trading to small residual rebalances with hardcoded 0.5% slippage caps. If caps are breached, trade aborts and keeper notifies operators.

---

### [R05] Regulatory Reclassification as Advisory or Fund
- **Risk:** Regulators may deem AI-driven portfolio construction and automated rebalancing as discretionary asset management or broker-dealer activity.
- **Probability:** Medium | **Impact:** High
- **Mitigation:**
  - **Strict Tooling Posture:** Constella is software. The AI only proposes; the user explicitly reviews, customizes, and cryptographically signs their own policy.
  - No discretionary authority resides with Constella operators.
  - Route all consumer retail distribution through licensed partners (e.g. licensed European/LatAm fintechs acting as Manager of Record).
  - Commission formal legal opinions in launch jurisdictions before deploying Mainnet Beta.

---

### [R06] Fee Compression on Wrapper Products
- **Risk:** Users may bypass Constella and hold underlying fund tokens directly to avoid wrapper fees.
- **Probability:** High | **Impact:** Low–Medium
- **Mitigation:**
  - Keep management fees ultralow (0.05%–0.25%/yr), well below robo-adviser industry averages.
  - Build revenue durability through **Treasury SaaS subscriptions** ($199–$499/mo) and B2B partner licensing, making the business resilient against AUM fee compression.
  - Deliver unique utility impossible to achieve manually: automated currency matching, instant redemption ladders, and collateral readiness via the Collateral Passport.

---

### [R07] High Issuer Concentration
- **Risk:** A single issuer (such as Spiko, which held ~39% of Stellar RWA value in August) could suffer an outage or regulatory action.
- **Probability:** Medium | **Impact:** High
- **Mitigation:**
  - Enforce strict default **issuer caps (maximum 30% per issuer)** across all baskets.
  - Build modular sleeve isolation: if an issuer halts or freezes their asset, only that specific sleeve is paused. The remainder of the basket remains liquid and uncompromised.

---

### [R08] Smart Contract Vulnerability or State Corruption
- **Risk:** Logic bugs or rent eviction in Soroban contracts resulting in locked funds.
- **Probability:** Low | **Impact:** Critical
- **Mitigation:**
  - Build exclusively on OpenZeppelin's audited `stellar-contracts` v0.8.0 primitives for core vault and token accounting.
  - Mandate independent third-party audits before deploying mainnet beta.
  - Property-based testing and formal verification of all share dilution and NAV accounting math.
  - Gradual TVL ramp: $1M cap during Phase 3 beta.
  - Automated keeper bots extending state TTL rent well before expiration.

---

### [R09] Protocol Upgrade Slippage (Bell Protocol 30 or DTCC)
- **Risk:** Mainnet vote on Bell (Nov 23) delays, or DTCC integration slips beyond H1 2027.
- **Probability:** Medium | **Impact:** Low
- **Mitigation:**
  - **Architectural Independence:** v1 is 100% functional on Protocol 28 (Adapter).
  - Bell features (W-addresses, ML-DSA) are modular, additive enhancements.
  - The core product launches on cash, T-bills, and lending; equity module is a pluggable future upgrade.

---

### [R10] Fee Sponsorship Account Drained
- **Risk:** Malicious actors spam vault endpoints to exhaust Constella's sponsored XLM balance.
- **Probability:** Medium | **Impact:** Low
- **Mitigation:**
  - Sponsor account implements hard daily spend limits and per-address rate limiting.
  - Sponsor transactions only validate against approved Constella vault contracts.
  - Automated alerting fires when sponsor balance drops below safe reserve thresholds.

---

### [R11] AI Engine Hallucination or Faulty Logic
- **Risk:** LLM recommends non-existent assets or invalid weights.
- **Probability:** Low | **Impact:** Medium
- **Mitigation:**
  - **Closed Universe Rule:** The AI can only select assets from the verified on-chain registry. Hallucinated tickers cannot pass retrieval.
  - Schema-constrained function calling strictly validates data types and ranges.
  - Final human review: The UI displays an explicit diff between the user's thesis and the proposed policy before signing.
  - The model has ZERO on-chain authority; the contract only enforces signed hashes.

---

### [R12] Simulation Data Misinterpreted as Guaranteed Return
- **Risk:** Users claim historical replay was an investment guarantee during market drawdowns.
- **Probability:** High | **Impact:** Medium
- **Mitigation:**
  - Prominent, unmissable disclaimers on every simulation view: *"This is a risk and liquidity simulation, not a performance forecast. Historical data does not predict future returns."*
  - Use term "Simulation" rather than "Backtest" across all documentation and UI copy.
