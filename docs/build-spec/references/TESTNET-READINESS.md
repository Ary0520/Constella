# Testnet Readiness — every feature, for real

> **Owner requirement (15 Nov 2026):** the whole product — every contract feature, user flow, oracle
> source, passkey login, fee sponsorship and the UI — must work on Stellar **testnet as the actual
> thing, not a simulation**, with an easy testnet/mainnet switch. Tests, demo days and user feedback
> happen on testnet before mainnet. This file tracks, per component, the real testnet path and any gap.
> Facts below were checked live on testnet on 15 Nov 2026 (read-only simulated calls).

| Component | Real testnet path | Status / gap |
|---|---|---|
| Contracts (registry, policy, oracle, vault, smart account, verifier, threshold policy, fee sponsor) | Same WASM deployed to testnet (protocol 28+) | Deploy scripts + e2e pending (Phase 1 local-network → testnet) |
| Numéraire / assets | Circle **testnet** USDC `CBIELTK6…DAMA`, EURC `CCUUDM43…MCGZ` (real SACs, faucet at faucet.circle.com) | ✅ available |
| Oracle — USDC | Base asset (exactly 1.0) | ✅ |
| Oracle — EURC, source 1 | Reflector testnet CEX/DEX `CCYOZJCO…RN63`: `Other("EURC")` ÷ `Other("USDC")` (ViaUsdc, O7b). Live: EURC 1.1001 USD, USDC 1.0004 USD, updated every 5 min | ✅ works with O7b today. Caveat: testnet Reflector is a **single-node** cluster |
| Oracle — EURC, source 2 (independent) | (a) Constella feeder service pushing an FX-derived EURC price with its own key (O7a), or (b) Reflector testnet FX `CCSSOHTB…NV4W` `Other("EUR")` cross-rated with the CEX feed's USDC | **(a) ✅ built** (`services/feeder`, `b9451e4`): end-to-end tested on a real protocol-30 local network, the same as testnet; it runs on testnet once the oracle is deployed there. (b) optional cross-feed quote, logged |
| Oracle — RedStone | Testnet adapter `CBIHT4HV…6BK3`: USDC (data 4.8 h old at check), XLM (~55 min), BTC; **no EUROC** | ⚠️ too sparse for EURC on testnet; usable for XLM/BTC. O7c (RedStone pull sources) is implemented; on testnet it can price XLM/BTC, and on mainnet EURC/BENJI/USDY |
| Oracle — issuer NAVs (BENJI, USDY, Spiko) | Issuer testnet tokens not confirmed | ❌ to investigate before Phase 3 assets |
| Rebalancing (W1a + W1b post-trade check, Soroswap) | Soroswap **testnet** router `CCJUD55A…7BRD` (factory `CDP3HMUH…JTBY`; testnet redeployed 22 Dec 2025) | ⚠️ the testnet Circle USDC/EURC pool `CDA3ZNR4…MJPXI` is tiny and lopsided (~3,903 USDC vs 0.12 EURC on 15 Nov 2026), so any real trade would fail our value-based slippage check. Before demos: add balanced liquidity to that pool with Circle-faucet USDC/EURC (real, on testnet), or pick a deeper pool. The W1b post-trade check needs nothing testnet-specific (it uses the same oracle prices and registry) |
| Blend positions (adapter) | Blend v2 testnet deployment | To verify when the adapter is built |
| Passkeys (WebAuthn) | Same contracts; real browser authenticators work on testnet | ✅ contract side; UI e2e pending |
| Fee sponsorship | Real fee-bump relayer against testnet | ❌ relayer not built (F1) |
| Reflector asset expiry (XRF fees) | Testnet feeds are maintained by Reflector | Monitor; mainnet needs XRF budget |
