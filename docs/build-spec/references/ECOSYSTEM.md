# Stellar Ecosystem Directory & Integrations Map

> Canonical directory of all Stellar protocols, issuers, oracles, and infrastructure relevant to the Constella platform.

---

## 1. RWA Issuers & Tokenized Assets

| Issuer | Assets / Products | Status | Custody / Backing | Gating / Access |
|---|---|---|---|---|
| **Franklin Templeton** | BENJI (US Gov Money Fund) | Live (>$1.1B AUM) | US Treasuries & Repos via BNY Mellon | KYC / Regulated Allow-list |
| **Spiko** | Spiko USD (T-Bills), Spiko EUR (T-Bills) | Live (~$3B AUM) | CACEIS (Crédit Agricole / Santander) | EU MiCA / Professional & Institutional |
| **Ondo Finance** | USDY (Yield-Bearing Dollar) | Live | Short-term US Treasuries & Bank Deposits | KYC / Non-US Persons |
| **Figure Markets** | YLDS (Yield Dollar), wYLDS (Wrapper) | Live (May 2026) | Prime Commercial Paper & Treasuries | Allow-list (wYLDS planned permissionless) |
| **Janus Henderson / Centrifuge** | JQQQ (Tokenized Nasdaq-100) | Live (Nov 2026) | Nasdaq-100 Synthetic / Custodied Equities | Non-US Professional (Min $100k) |
| **WisdomTree** | WisdomTree US Government Money Fund | Live | US Treasuries | Institutional / Direct app |
| **Etherfuse** | Stablebonds (Mexican Cetes, Brazilian Tesouro) | Live | Sovereign debt held at BBVA | Latin America / Emerging Markets |
| **Archax** | €GOVY (European Sovereign Bonds) | Announced Oct 2026 | European Sovereign Debt | UK / EU Qualified Investors |
| **M1X** | USDM1 | Live / Repo Pilot | 1:1 US Treasury backing, hourly repo roll | Institutional Collateral Pilot |
| **Circle** | USDC, EURC | Live | 100% Cash & Short-dated US Treasuries | Permissionless |

---

## 2. DeFi Lending & Money Markets (Collateral Sinks)

| Protocol | Category | TVL (Nov 2026) | Significance to Constella |
|---|---|---|---|
| **Blend Protocol** | Decentralized Money Market | ~$140M–$145M | Dominant lending venue; primary target for basket collateral listing |
| **Liqvid** | RWA Collateral Lending | Seed / SCF #45 | Purpose-built for RWA collateral; launch co-design partner |
| **Lucent Finance** | Institutional Lending Pool | Emerging | Secondary institutional money market |
| **DeFindex** | Yield Strategy Aggregator | ~$18.5M | Candidate yield sleeve for liquid cash USDC |

---

## 3. Decentralized Exchanges & Liquidity Pools

| Venue | Architecture | TVL / Volume | Role in Constella |
|---|---|---|---|
| **Soroswap** | Uniswap v2 style AMM | ~$1.6M Daily Vol | Primary venue for small residual rebalance trades |
| **Aquarius (Aqua)** | Liquidity Incentivization & AMM | Active | Secondary AMM liquidity pool |
| **Stellar Classic DEX (SDEX)** | Native on-chain orderbook | ~$16M TVL | Classic orderbook for stablecoin netting & EURC pricing |

---

## 4. Oracles & Pricing Infrastructure

| Provider | Type | Latency / Frequency | Status on Stellar |
|---|---|---|---|
| **Reflector** | Stellar-native decentralized oracle | Multi-node consensus | Live; primary decentralized price source |
| **RedStone** | Modular pull/push oracle | Cryptographically signed feeds | Launched on Stellar March 2026; high-speed feeds |
| **Issuer NAV Feeds** | Direct HTTPS / JSON REST APIs | Daily / Hourly publisher NAV | Primary source for fund tokens (BENJI, Spiko, USDY) |

---

## 5. Developer Tools & Frameworks

| Tool | Maintainer | Version | Purpose in Constella |
|---|---|---|---|
| **soroban-sdk** | SDF | v28 / v30-ready | Rust smart contract development |
| **stellar-cli** | SDF | Latest | Deployment, fleet upgrades, and network simulation |
| **stellar-contracts** | OpenZeppelin | **v0.8.0** | Audited SEP-56, SEP-57, and smart-account modules |
| **stellar-wallets-kit** | Ecosystem / Creit | Latest | Multi-wallet frontend connectivity (Freighter, xBull, passkeys) |
| **Galexie** | SDF | Active | Ingestion and indexing of Soroban ledger events |
| **PyPortfolioOpt** | Open Source | Latest | Hierarchical Risk Parity (HRP) portfolio optimization library |
| **LangChain** | Open Source | Latest | Orchestration of LLM thesis parsing and schema generation |
