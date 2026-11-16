# Constella sponsor relayer (audit F1 / S2)

Pays the Stellar network fee for users who hold **no XLM** — passkey smart accounts — so every vault
interaction is zero-gas for the user, within quotas enforced **on chain** by `contracts/fee-sponsor`.

## How it works

A passkey smart account is a contract (`C…`) and can never be a transaction source. The relayer's own
account is the source and pays the fee; the user signs only its Soroban **authorization entries** with
its passkey. Requests use the same `func` + `auth` shape as the OpenZeppelin Relayer Channels plugin.

```
client                      relayer (this service)                      chain
  │ POST /prepare {func} ──▶ recording simulation ──────────────────────▶ (read only)
  │ ◀── unsigned auth entries + latest ledger
  │ sign entries with passkey (passkey.py shows the exact format)
  │ POST /submit {func, auth} ──▶ 1. gateway.validate (pure)
  │                               2. fee-sponsor.is_whitelisted(target)   (on chain)
  │                               3. prepare user tx (seq S+2) with ENFORCING
  │                                  simulation → account's __check_auth runs
  │                               4. fee ≤ RELAYER_MAX_FEE_STROOPS
  │                               5. record_sponsored_op(user, target, fee) (seq S+1)
  │                                  → whitelist, pause, per-user quotas, daily budget
  │                               6. submit user tx (seq S+2)
  │ ◀── {hash, user, fee_charged, record_hash, record_fee_charged}
```

Because the user transaction carries sequence S+2, the network itself rejects it unless the record
transaction (S+1) landed first: quotas cannot be skipped even by a bug in step ordering.

## Security rules (gateway.py)

- Only `InvokeContract` on allow-listed contract **and** function (`SPONSOR_ALLOWED`); no deploys/uploads.
- **SourceAccount credentials are refused** — they would let the request spend the relayer's own funds.
- No relayer address as signer, CAP-71 delegate (any depth, tree ≤ 8) or anywhere in call arguments.
- `ADDRESS`, `ADDRESS_V2` (CAP-71, address-bound payload) and `ADDRESS_WITH_DELEGATES` accepted.
- `auth[0]` must authorize exactly this call; its address is the sponsored user. 1–4 entries; size limits.
- Errors are stable codes (`SOURCE_ACCOUNT_AUTH_FORBIDDEN`, `SIMULATION_FAILED`, `SPONSOR_REFUSED`, …).

The relayer key is the fee-sponsor's **relayer role** (F1b): it can record sponsored operations and
nothing else — it cannot change limits, the whitelist, the admin, or unpause.

## Run

```
CONSTELLA_RPC_URL=… CONSTELLA_NETWORK_PASSPHRASE=… RELAYER_SECRET=… \
SPONSOR_CONTRACT_ID=C… SPONSOR_ALLOWED="C<vault>:deposit,redeem,withdraw,redeem_frozen" \
RELAYER_MAX_FEE_STROOPS=1000000 constella-relayer        # binds 127.0.0.1:8787
```

Keep it behind your API gateway / rate limiter; never expose it directly. The secret is read from the
environment only and never logged. Production: hosted signer (KMS) is a follow-up.

## Tests

- `pytest` (23 unit tests on real XDR; gateway mutation-tested 9/9; passkey digest vectors cross-checked
  against OpenZeppelin's Rust `AuthDigestPreimage::digest`).
- `tests/localnet_e2e.sh` — the S2 proof on a real local network (quickstart, protocol 30, release
  WASMs). Result on 16 Nov 2026: a passkey account with 0 XLM deposited 500 USDC; 500 shares minted;
  user XLM 0; the relayer paid exactly the fees charged (deposit 328,466 + record 105,483 stroops);
  stranger passkey / source-account drain / non-allow-listed function / paused sponsor all refused
  before any quota or XLM was spent.

## Not yet

- OpenZeppelin hosted Channels as the submission backend (owner decision pending: external service).
- Channel accounts for parallel submission (one relayer account, serialized by a lock, today).
- Classic `G…` users signing their own inner transaction (fee-bump / CAP-15 path).
- Account-management sponsorship (each smart account is its own contract; needs a separate rule).
