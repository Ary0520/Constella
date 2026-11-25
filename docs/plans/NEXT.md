# Next: from "safe USDC vault" to the founding-doc product

Written 23 Nov 2026. Source of truth for scope: the founding doc "Constella: Thesis-Driven Baskets of
Stellar's Real-World Assets" (30 Oct 2026) and `docs/build-spec/`. Open bugs: `docs/OPEN-ISSUES.md`.
Every step: red test first, mutation-test new rules, full gate (release build, workspace tests,
clippy -D warnings), commit, update docs and memory, and it must work for real on testnet.

## Where we are
Built and on testnet: registry, policy, multi-sleeve vault (dual NAV, fees, frozen sleeves, caps,
timelocks), private-vault factory, NAV oracle (multi-source, breaker, Reflector, RedStone, ECB feeder),
on-chain Collateral Passport views + collateral oracle, keeper (Soroswap, Blend), passkey accounts,
sponsorship, devices, wallets, AI engine (LLM on, guarded).

The core promise is NOT real yet: the optimiser has one hard-coded profile (Cash Reserve 85/15), so
every thesis becomes the same basket, and a drafted policy cannot become the user's own vault.

## Steps, in order

| # | Step | Why now | Done when |
|---|---|---|---|
| 1 | ✅ DONE 24 Nov: OPEN-ISSUES #1 (device rule can reach recovery) and #3 (policy hash) fixed, live on testnet API/console; factory WASM activation due 25 Nov 19:01 UTC | Safety and correctness; small | Red tests pass; hash == exact on-chain terms |
| 2 | ✅ DONE 24 Nov (D-O): fee model stays D-D; factory vault fees now set by Constella (creator alone gets the default; discounts need Constella's co-signature); new factory live on testnet | Contract change; easier before more vaults exist | Proven on chain: creator-only fee 0 refused |
| 3 | ✅ DONE 24 Nov (live on testnet; see STEP-03-THESIS-TO-VAULT.md): **Thesis -> your own vault, end to end**: draft -> review -> passkey signs -> factory creates a private vault -> `bind_policy` with the user as owner -> deposit | Makes the AI do real work; the first "it's Constella" moment | On testnet, a typed thesis produces a live vault the user owns, enforcing that thesis |
| 4 | ✅ DONE 25 Nov (D-P; live on testnet): **Real thesis-driven optimiser**: profiles out, constraints from the thesis (currency split, liquidity floor, issuer caps, liquidity tiers), HRP default, over every active registry asset | The founding doc's core | Different theses give different, explained baskets; infeasible ones say why |
| 5 | Multi-asset sleeves on testnet: EURC sleeve live (needs Soroswap EURC depth), Blend EURC reserve, any real permissionless testnet RWA we can find (research: Etherfuse, Spiko, Franklin testnet tokens) | Baskets need more than USDC to be baskets | >= 3 priced sleeves rebalanced by the keeper |
| 6 | Flow netting: deposits buy underweight sleeves, withdrawals come from overweight ones | Founding doc 4.5; cheaper, fewer trades | Keeper trades only residuals |
| 7 | ✅ DONE 25 Nov (live on the hosted console): Simulation to spec: redemption ladder, depeg, issuer outage, history replay, worst exit time; explain = diff vs thesis | Founding doc 2.2 / 4.7 | Shown before every signature |
| 8 | 🟡 LIVE 25 Nov on testnet + hosted console (guardians, M-of-N approval, propose, cancel); finish proof due 27 Nov 13:00 UTC (`testnet/e2e_recovery.py --finish`). Account recovery (OPEN-ISSUES #2): guardians setup + recovery, sponsored | Users must not lose funds | Recover an account with a new device on testnet |
| 9 | Collateral Passport live: vault share listed in our Blend pool (needs BLND backstop: ask Blend team) | The headline metric | Borrow USDC against a vault share on testnet |
| 10 | Real product app (design from the landing direction; reuse sandbox passkey/chain/wallet code) | Replace the throwaway console | Every live flow above in the real UI |

## Mainnet track (after the above)
External audit; multisig admin (threshold policy) + KMS keys; SEP-57 gated share class + KYC reuse
(needed for BENJI, USDY, Spiko); issuer mint/redeem adapters; indexing and treasury reporting; partner
SDK (W-addresses: Bell passed 23 Nov 2026 - test a W destination against our sdk-28 vault first); ML-DSA admin
verifier once soroban-sdk 30 is stable; watchdog email; capped mainnet beta. See RESEARCH-UPDATES section 1.
