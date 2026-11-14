# Compliance, Identity & Regulatory Architecture

> Guidelines, technical hooks, and policy standards governing eligibility, KYC reuse, and asset protection.

---

## 1. Regulatory Posture

### Tooling Posture for Self-Directed Treasuries (v1)
- **Positioning:** Constella is pure non-custodial software infrastructure and tooling for self-directed treasuries, DAOs, and individuals.
- **No Discretionary Management:** Constella does NOT have discretionary authority over user funds. The AI engine drafts proposals; the user/treasury signs the immutable on-chain policy.
- **No Broker-Dealer Activity:** Constella does not match counterparties, hold custody, or execute orders outside of verified decentralized protocols and issuer primary endpoints.
- **Retail Distribution:** Direct consumer retail distribution is deferred to Phase 4 via licensed partner fintechs or a licensed Manager of Record (MoR) in compliant jurisdictions.

---

## 2. The Eligibility Engine

### Concept & Rule
Before any asset is suggested by the AI engine or deposited into a vault, it must pass the eligibility filter.

```
       User Context
 (Jurisdiction, Investor Class)
              │
              ▼
   ┌──────────────────────┐
   │  Eligibility Engine  │◄──── Diligence Registry Metadata
   └──────────┬───────────┘
              │
    Filtered Candidate List
 (Only legally holdable assets)
```

### Eligibility Parameters
1. **Jurisdiction:**
   - US vs. Non-US (Reg D / Reg S compliance).
   - Sanctioned countries (OFAC list permanently blocked).
   - Specific local regulations (e.g., MiCA in Europe, RBI guidelines in India).
2. **Investor Classification:**
   - **Retail:** Limited to permissionless sleeves (USDC, EURC, Blend, DeFindex).
   - **Professional / Accredited:** Eligible for tokenized T-Bills, corporate credit, yield funds (BENJI, Spiko, USDY).
   - **Qualified Institutional Buyer (QIB):** Eligible for restricted private credit and high-ticket assets (JQQQ, Tradable).

---

## 3. Identity Verification & KYC Reuse

### Philosophy: Zero Redundant KYC
Re-running full identity verification on a retail or treasury customer costs $1–$5 per check, destroying unit economics on small accounts. Constella implements **KYC Reuse via Portable Attestations and Issuer Pass-Through**:

```
                       ┌────────────────────────────┐
                       │ Does user have issuer KYC? │
                       │ (e.g. Spiko, Franklin)     │
                       └─────────────┬──────────────┘
                                     │
                    ┌────────────────┴────────────────┐
                 YES│                                 │NO
                    ▼                                 ▼
         Pass-through approval          ┌────────────────────────────┐
         (Vault is allow-listed;        │ Does partner wallet have   │
          user signs with KYC address)  │ verified KYC attestation?  │
                                        └─────────────┬──────────────┘
                                                      │
                                     ┌────────────────┴────────────────┐
                                  YES│                                 │NO
                                     ▼                                 ▼
                          Verify cryptographic attestation   Fallback: Trigger lightweight
                          via SEP-57 Identity Hook           KYC provider (Fractal / Persona)
```

---

## 4. Stellar Asset Controls Integration

Stellar's native consensus and asset model includes powerful institutional controls. Constella vaults natively handle each:

### 1. `AUTH_REQUIRED` & `AUTH_REVOCABLE`
- Issuers require explicit authorization before an account can hold their asset.
- **Constella Solution:** For private vaults, the vault contract address is registered with the issuer. For strategy vaults, the master pool contract is approved as an omnibus holder.

### 2. Clawback (`AUTH_CLAWBACK_ENABLED`)
- Allows regulated issuers to claw back funds to comply with court orders or regulatory mandates.
- **Constella Isolation:**
  - If an issuer executes a clawback on a specific sleeve asset, a circuit breaker halts that sleeve.
  - The vault triggers an emergency NAV recalculation.
  - Remaining sleeves (e.g. USDC, T-bills from other issuers) remain 100% operational and liquid.

### 3. Protocol 26 Quorum Freeze
- Network-wide emergency mechanism.
- Handled via automated pause hooks in the vault's governance layer.

---

## 5. Pre-Mainnet Legal & Regulatory Checklist

Prior to launching Phase 3 (Mainnet Beta):

- [ ] **Investment Advice Analysis:** Legal opinion confirming that structured LLM thesis generation + user opt-in signing does not constitute regulated financial advice.
- [ ] **Token Classification:** Legal confirmation that SEP-56 vault share tokens representing wrapped underlying RWAs do not constitute an unregistered collective investment scheme (CIS) in launch jurisdictions.
- [ ] **Terms of Service & Risk Disclaimers:** Formalize terms explicitly stating self-directed tooling posture, non-custodial architecture, and standard simulation disclaimers.
- [ ] **OFAC & Sanctions Screening:** Implement automated address-level sanctions screening on all deposit endpoints.
- [ ] **Partner MoR Structure:** Finalize legal framework for third-party licensed managers of record before onboarding European or emerging market retail users.
