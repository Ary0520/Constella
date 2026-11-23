# Open issues (must fix before mainnet)

Found 23 Nov 2026 while fact-checking the landing page against the code. Each one is
open until a fix is merged **and** a test proves it. Keep this list current; delete an
entry only in the commit that fixes it.

Fixed: #1 (device rule could propose a recovery) - 24 Nov 2026, smart-account `RecoveryRequiresRecoveryRule`.

| # | Severity | Issue | Where | Done when |
|---|---|---|---|---|
| 2 | **High (users can lose funds)** | No recovery path if every passkey is lost. Guardian recovery exists in the contract (48 h, owner-cancellable) but nothing in the relayer or frontend sets up guardians or calls it, and the relayer does not sponsor those calls. | `contracts/smart-account`, `services/relayer`, frontend | Guardian setup + recovery flow works end to end on testnet; sponsored; documented in ACCOUNTS.md. |
| 3 | Medium (correctness) | Policy drafting: the policy hash is built with a placeholder admin (`GBADMIN1234567890`, `services/ai-engine/src/signer.py:31`, `api.py:106`), and the hashed JSON says `rebalance_band_bps` 500 (`signer.py:83`) while the term sent on-chain is 200 (`api.py:39`). The signed hash does not describe what is enforced. | `services/ai-engine/src/signer.py`, `api.py` | Hash built from the real owner and the exact on-chain terms; a test asserts hashed JSON == bind_policy terms. |
| 4 | Low (UI) | Landing page on phones: the hero coin overlaps the copy and the nav buttons wrap. | `prototypes/landing/index.html` (media query at max-width 900px) | Hero readable at 390 px wide, verified by screenshot. |

Also noted (not bugs, product gaps):
- (Resolved 23 Nov 2026) The AI model is now **on** for the hosted testnet services: `LLM_ENABLED=1`,
  `LLM_RUN_BUDGET_USD=1.00` per process run (falls back to the rules parser when spent). Verified live:
  `parse_source: llm+rules`.
- The draft-policy screen shows the on-chain terms but cannot yet bind them to a user's own vault.
