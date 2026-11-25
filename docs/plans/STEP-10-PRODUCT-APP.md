# Step 10 - the real product app (replaces the throwaway console)

Plan written 25 Nov 2026 before any code (hard rule). Sources: docs/plans/NEXT.md (step 10), founding doc
(thesis-driven RWA baskets + Collateral Passport), build spec `architecture/OVERVIEW.md` (Experience layer),
`AGENT-RULES.md` (Rule 10 disclaimer, Rule 11 slice-and-pause), `architecture/ACCOUNTS.md`,
`architecture/EXECUTION.md`, memory: landing design direction + feature availability.

## What it must do (spec)
- Chat-to-basket: thesis -> policy proposal; the UI always shows the signed policy and its diff against the thesis;
  no auto-submit of AI proposals (OVERVIEW "Key constraint").
- Simulation display (redemption ladder, stress scenarios, history) with the Rule 10 disclaimer, verbatim.
- Portfolio dashboard: NAV, weights vs targets/bands, holdings by sleeve, realised return, fee.
- Passkey sign-in (no seed phrase, no XLM) and Stellar wallet sign-in (Wallets Kit).
- Every live testnet flow (NEXT.md "Done when"): accounts, devices, recovery, thesis -> own vault, deposit /
  redeem / withdraw, frozen-asset claims, keeper activity.
- The founding-doc vision is visible: issuer-gated assets (BENJI, USDY, Spiko, JQQQ), gated vaults, Collateral
  Passport borrowing - shown but DISABLED with an honest note (feature-availability). No fake numbers, no
  "guaranteed/safe/audited" claims.

## Decision D-R: React + TypeScript on Vite, static client-only app (`app/`)
The spec allows "React / Next.js". Everything here runs in the browser (WebAuthn, Wallets Kit, Soroban RPC reads,
relayer calls); there is no server code and the app must carry no secrets, so server rendering adds risk
(WebAuthn / wallet code under SSR) and no value. Vite + stellar-sdk 17.4 + Wallets Kit is already proven in the
console. Pinned: react / react-dom 19.3.1 (14 Oct 2026), vite 6.4.1, @vitejs/plugin-react 5.2.0 (supports vite 6),
vitest 3.2.7, playwright 1.55.0. No router library (a ~30-line history router; fewer dependencies). Styling: plain
CSS with design tokens taken from the landing page (lavender, Plus Jakarta Sans, logo) - light and dark.

## Code reuse
`sandbox/src/{passkey,chain,wallet,recovery,ownvault}.ts` and their unit tests move into `app/src/lib/`
unchanged (passkey signing is byte-identical to the proven Python signer - test vectors move with it).
`sandbox/` is deleted at the end of step 10, after every flow passes in the new app.

## Slices (each: red test -> code -> verify in a browser -> commit + push)
| # | Slice | Done when |
|---|---|---|
| 10.1 | Scaffold `app/`, tokens + shell (nav, cards, buttons, gated badge), config loading, lib modules + tests moved | unit tests pass; shell renders; screenshot checked |
| 10.2 | Identity: welcome, create passkey (this device / phone), sign in with an existing passkey (factory, device registry, recovery registry), connect wallet; session | browser e2e: create + sign in on testnet |
| 10.3 | Portfolio: the user's vaults (own vaults from the factory + the shared Cash Reserve), NAV, weights vs bands, holdings, position, fee | live numbers match chain reads |
| 10.4 | New basket: thesis -> engine stages -> review (basket, simulation + Rule 10, thesis diff, exact on-chain terms) -> create with one signature | browser e2e creates a live vault |
| 10.5 | Vault: deposit / redeem / withdraw, claims, policy, keeper activity | browser e2e deposit + redeem |
| 10.6 | Account & security: devices (add / join with check code), guardians + recovery (setup, request, approve, finish, cancel) | recovery + pairing e2e ported and passing |
| 10.7 | Vision surfaces, gated: asset universe, Collateral Passport, gated vaults, partners | every gated item visible + disabled with its note |
| 10.8 | Deploy (new Vercel project, testnet config, no secrets), relayer CORS for its origin, all e2e on the hosted app, delete `sandbox/` | all flows pass on the hosted app |

Mobile: every screen works at 390 px (also folds in OPEN-ISSUES #4's lesson for the landing hero).
