# Stellar Protocol Upgrades & CAP Roadmap

> Technical breakdown of Stellar protocol upgrades relevant to Constella architecture, gas optimization, and execution security.

---

## Protocol Upgrade Matrix

| Protocol | Codename | Mainnet Date | Key CAPs | Direct Impact on Constella |
|---|---|---|---|---|
| **Protocol 23** | Whisk | H1 2025 | Parallel execution | Parallel rebalancing across isolated sleeves |
| **Protocol 25** | X-Ray | Testnet Jan 7, 2026 | CAP-74, CAP-75 | BN254 elliptic curves & Poseidon hashes for future ZK eligibility proofs |
| **Protocol 26** | Yardstick | Mainnet May 6, 2026 | CAP-77, CAP-78, CAP-80 | Quorum Freeze, 256-bit arithmetic, precise TTL rent control |
| **Protocol 27** | Zipper | Mainnet Jul 8, 2026 | CAP-71 | Authentication delegation for custom accounts; passkey replay protection |
| **Protocol 28** | Adapter | Mainnet Sep 16, 2026 | CAP-85, AddressV2 | Fast ~5s ledger close, atomic fleet contract upgrades |
| **Protocol 29** | *(Patch)* | Mainnet Oct 1, 2026 | Security patch | Maintenance & security stability; no new CAPs |
| **Protocol 30** | Bell | **Mainnet Nov 23, 2026** | CAP-84, CAP-87, CAP-88 | W-addresses (muxed contracts), ML-DSA (quantum prep), ms timestamps |

---

## Detailed CAP Analysis

### Protocol 26 (Yardstick)

#### 1. CAP-77: Quorum Freeze
- **Mechanism:** Allows validators to execute a governed freeze on compromised contracts or assets in extreme network emergencies.
- **Constella Role:** Acts as an ecosystem backstop. If a systemic exploit or extreme bridge failure occurs on Stellar, Quorum Freeze halts state corruption while Constella circuit breakers isolate affected sleeves.

#### 2. CAP-78: Checked 256-bit Arithmetic & Precise TTL Control
- **Arithmetic:** Native 256-bit safe integer math prevents overflow in complex NAV calculations when basket AUM reaches institutional scale ($100M+).
- **TTL Control:** Soroban requires state rent. CAP-78 enables granular ledger entry life extensions, allowing keeper bots to batch-extend contract TTLs cheaply without overpaying rent.

---

### Protocol 27 (Zipper)

#### CAP-71: Authentication Delegation for Custom Accounts
- **Mechanism:** Permits smart contract accounts to delegate signature verification to external address-bound credentials.
- **Constella Role:** Enables frictionless passkey onboarding (WebAuthn / secp256r1) and enterprise multisig setups with social recovery. Users authenticate with biometrics; Constella sponsors the transaction fees seamlessly.

---

### Protocol 28 (Adapter)

#### CAP-85: Atomic Fleet Upgrades
- **Mechanism:** Allows an entire fleet of contract instances that share the same WASM bytecode hash to be upgraded in a single atomic transaction.
- **Constella Role:** When Constella upgrades the vault logic, hundreds of deployed strategy and private vaults can be upgraded simultaneously. Users never suffer downtime, and migration scripts run deterministically.

#### Fast Ledger Consensus
- Ledger close times were reduced from ~5.7s to ~5.0s, reducing trade settlement latency during keeper rebalancing operations.

---

### Protocol 30 (Bell) — Mainnet Vote: 23 November 2026 `[BELL]`

> ⚠️ **Status Note:** Stellar Core v30.0.0 released Oct 16. Testnet vote passed Nov 3. Mainnet vote scheduled for **Nov 23, 2026, 17:00 UTC**.

#### 1. CAP-84: W-Addresses (Muxed Contract Accounts)
- **What it does:** Introduces multiplexed contract addresses (`W`-prefixed), allowing a single contract address to carry a 64-bit routing/mux ID.
- **Constella Impact:**
  - Game-changer for **Partner / Wallet integrations**.
  - A partner fintech or wallet needs only **one master omnibus contract**.
  - Each customer deposit includes a unique 64-bit user ID embedded in the W-address.
  - Events emit the customer ID natively, allowing the partner to reconcile tens of thousands of users off a single contract without deploying individual vaults.

#### 2. CAP-87: ML-DSA Verification in Contracts
- **What it does:** Native host functions to verify NIST FIPS 204 Module-Lattice-Based Digital Signature Algorithm (ML-DSA) signatures inside Soroban.
- **Constella Impact:**
  - Institutional Treasury clients can configure their private vault admin signers to require post-quantum secure ML-DSA keys.
  - Establishes Constella as the first quantum-prepared asset management protocol on Stellar.

#### 3. CAP-88: Millisecond Ledger Close Time
- **What it does:** Updates ledger header schema to record block time in milliseconds rather than integer seconds.
- **Breaking Change:** All indexers, nodes, and SDKs must update their XDR definitions to parse the new ledger header.
- **Constella Impact:** Prepares the foundation for sub-second rebalancing and higher frequency oracle updates.

---

## v1 Architecture Boundary Rule

```
┌─────────────────────────────────────────────────────────────┐
│                    V1 RELEASE CONSTRAINT                    │
│                                                             │
│  1. Constella v1 MUST NOT treat Bell as a hard blocker.     │
│  2. Build and test on Protocol 28 (Adapter) standards.      │
│  3. Bell features (CAP-84 W-Addresses & CAP-87 ML-DSA)      │
│     are engineered as modular, opt-in plugins activated     │
│     after the Nov 23 mainnet vote clears successfully.      │
│  4. Upgrade all SDKs (JS & Rust) to v30-compatible headers. │
└─────────────────────────────────────────────────────────────┘
```
