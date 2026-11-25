# Open issues (must fix before mainnet)

Found 23 Nov 2026 while fact-checking the landing page against the code. Each one is
open until a fix is merged **and** a test proves it. Keep this list current; delete an
entry only in the commit that fixes it.

Fixed: #1 (device rule could propose a recovery) - 24 Nov 2026, smart-account `RecoveryRequiresRecoveryRule`.
Fixed: #3 (policy hash placeholder admin, band 500 vs 200) - 24 Nov 2026, `signer.onchain_terms` + real `owner`.
**Pending rollout:** testnet account factory has the fixed WASM `284b9a9e…` proposed; run
`python testnet/account_wasm.py activate` after 2026-11-25 19:01 UTC. Accounts created before then keep the old
WASM (no upgrade path by design) - on mainnet every account will be created with the fixed code.

| # | Severity | Issue | Where | Done when |
|---|---|---|---|---|
| 2 | **High (users can lose funds)** | No recovery path if every passkey is lost. **25 Nov 2026: built and live on testnet** (recovery registry, guardian mailbox, sponsored propose/cancel, relayer-built finish, console screens; ACCOUNTS.md). Proven live: guardian setup, M-of-N approval, proposal on chain, owner cancel. **Still open until** the 48-hour finish is proven: `python testnet/e2e_recovery.py --finish` after 2026-11-27 13:00 UTC. | `contracts/smart-account`, `services/relayer`, frontend | Guardian setup + recovery flow works end to end on testnet; sponsored; documented in ACCOUNTS.md. |
| 4 | Low (UI) | Landing page on phones: the hero coin overlaps the copy and the nav buttons wrap. | `prototypes/landing/index.html` (media query at max-width 900px) | Hero readable at 390 px wide, verified by screenshot. |

Also noted (not bugs, product gaps):
- Euro-currency vaults need a EUR-based price feed on chain (the NAV oracle is USDC-based; the factory refuses
  other vault currencies). Until then a mostly-euro thesis stays in a USDC vault with the reason shown.
- Testnet Soroswap USDC/EURC depth is demo-scale (~177 EURC after repricing on 25 Nov 2026): the keeper refuses
  larger EURC legs (correctly). More testnet EURC is needed for bigger baskets.
- (Resolved 23 Nov 2026) The AI model is now **on** for the hosted testnet services: `LLM_ENABLED=1`,
  `LLM_RUN_BUDGET_USD=1.00` per process run (falls back to the rules parser when spent). Verified live:
  `parse_source: llm+rules`.
- The draft-policy screen shows the on-chain terms but cannot yet bind them to a user's own vault.
