# Constella sandbox - manual user-flow test suite

Run these in a real browser on the local stack before the real product frontend is built. Each test
lists the steps and what you should see. **The status bar at the top shows every result or error**;
if a step fails, copy the status bar text and the Activity log (bottom of the page).

## 0. Setup (once per session)

| # | Step | Expected |
|---|---|---|
| 0.1 | `cd contracts && stellar contract build` (only after contract changes) | 11 contracts built |
| 0.2 | `localnet/up.sh` | ends with "stack up: RPC :8000 relayer :8787 thesis API :8790" |
| 0.3 | `cd sandbox && npm install && npm run dev` | Vite prints `http://localhost:5173` |
| 0.4 | Open **http://localhost:5173** in Chrome or Edge (Windows) | Status bar "Ready."; overview shows NAV ~1.0000, EURC price 1.0800 |

For the wallet tests (section W), add the local network to your wallet once. In Freighter/Varial:
Settings -> Network -> **Add custom network**:

| Field | Value |
|---|---|
| Name | Constella local |
| Horizon URL | `http://localhost:8000` |
| Soroban RPC URL | `http://localhost:8000/soroban/rpc` |
| Passphrase | `Standalone Network ; February 2017` |
| Friendbot URL | `http://localhost:8000/friendbot` |

If the wallet refuses an `http://` URL, enable its "allow HTTP / custom networks" option (label varies by
version). Then select **Constella local** as the active network. xBull also supports custom networks.

## P. Passkey account (zero XLM, the relayer pays)

| # | Step | Expected |
|---|---|---|
| P1 | Click **Create with this device (Windows Hello)** (recommended locally) - or **Create with phone / security key** | Status "create passkey account: done" within ~10 s; section 2 shows `account C...`, USDC 0, "XLM: none needed" |
| P2 | Reload the page | The same account is still shown |
| P3 | (Recovery) Start P1 in a fresh browser profile, and while the status says "...relayer needs a few seconds", reload the page | Section 2 says "Passkey saved, account setup unfinished"; clicking **Create passkey account** finishes it **without** a new passkey prompt-to-create |
| P4 | Click **Get 1,000 test USDC** | USDC 1000.0000 |
| P5 | Section 3: **Sign in with: passkey**; Deposit **200** | Passkey prompt; then USDC 800, vault shares ~200; no XLM asked of you at any point |
| P6 | Wait ~20-30 s, watch the overview | EURC sleeve "held" goes from 0 to a positive amount (keeper rebalanced to 85/15) |
| P7 | Redeem **50** shares | Passkey prompt; shares drop by 50, USDC rises by ~50 |
| P8 | Click **Start over with a new passkey** | Section 2 says "no account in this browser yet" (the account still exists on chain) |

## W. Stellar wallet (you sign in your wallet and pay the XLM fee)

| # | Step | Expected |
|---|---|---|
| W1 | Click **Connect a Stellar wallet** | A picker lists Varial/Freighter, xBull, LOBSTR, Albedo, Hana...; choose yours and approve the connection |
| W2 | Look at section 2b | Your `G...` address; with a fresh account: "NOT FUNDED", "Next: fund the account" |
| W3 | Click **Fund with friendbot** | account exists, XLM ~10,000; "Next: add the USDC trustline" |
| W4 | Click **Add USDC trustline** | Wallet popup to sign; then "USDC trustline yes"; "Next: get test USDC" |
| W5 | Click **Get 1,000 test USDC** (section 2b) | USDC 1000; "Ready." |
| W6 | Section 3: **Sign in with: Stellar wallet**; Deposit **300** | Wallet popup shows the transaction; after signing: USDC 700, vault shares ~300; your **XLM balance drops slightly** (you paid the fee) |
| W7 | Redeem **50** | Wallet popup; shares -50, USDC +~50, XLM drops again slightly |
| W8 | Reject the wallet popup on any action | Status bar shows a "failed" message; nothing changes on chain |
| W9 | Reload the page | Wallet panel says "not connected" (reconnect with W1 - by design in this console) |

## F. Frozen asset -> claim -> keeper settles (both identities)

| # | Step | Expected |
|---|---|---|
| F1 | Click **Freeze EURC prices** | Overview: EURC price "NONE"; NAV "UNPRICED"; Collateral Passport: "NONE - ... lending halts" |
| F2 | Deposit or normal Redeem now | Fails (status bar shows an error): no fair price while frozen |
| F3 | **Redeem while frozen 10** (either identity) | Succeeds: you get USDC for the priced part, and "open EURC claim" shows a positive amount |
| F4 | Click **Restore prices**, wait ~20-40 s | NAV priced again; the open claim returns to **0** and your USDC rises (the keeper paid the claim) |

## A. AI thesis -> signed policy

| # | Step | Expected |
|---|---|---|
| A1 | Keep the default thesis, jurisdiction DE, class retail; **Draft policy** | profile cash_reserve, vault currency USDC, parsed by rules; a table with USDC 100 %; policy hash shown; "exact on-chain bind_policy terms" has USDC as `numeraire` |
| A2 | Jurisdiction **IR** | "INFEASIBLE" with a reason (sanctioned jurisdiction) |
| A3 | Thesis "No more than 40% per issuer, rest in dollar t-bills" | "Needs your confirmation" mentions the cap looser than the 30 % default |
| A4 | Tick "use the LLM" | Still works; a note says the AI reading is disabled on this server (LLM off locally by design) |

## C. Collateral Passport and admin

| # | Step | Expected |
|---|---|---|
| C1 | Section 6 with prices normal | "share price served to lenders" equals the vault conservative NAV |
| C2 | Section 7: **Pause vault**, then try a deposit | Overview "PAUSED"; deposit fails |
| C3 | **Unpause vault** | Overview "running"; deposits work again |
| C4 | **List factory vaults** | "none yet" (factory vaults are created later in the product) |

## Finish

`localnet/down.sh` stops everything and forgets the local chain (the next `up.sh` starts fresh: create a
new passkey account then - your old one only existed on the old chain).

Automated equivalents (run by the engineer): `npm test`, `npm run test:stack` (both identities on the
live stack), `npm run test:browser` (headless Chromium with a virtual passkey).

## Troubleshooting

- **Phone says "no passkeys" when signing a deposit:** the account was created with a passkey that is not on
  that phone (e.g. from an earlier unfinished attempt). Section 2 shows where the passkey is stored. Click
  **Start over with a new passkey**, then create with **this device (Windows Hello)** - signing then happens
  locally, no QR code.
