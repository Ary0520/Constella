# Step 3: a thesis becomes the user's own live vault

Plan row (docs/plans/NEXT.md #3): draft -> review -> passkey signs -> factory creates a private vault ->
`bind_policy` with the user as owner -> deposit. Founding doc 4.7 step 6 ("The user or multisig signs the
policy hash; the policy contract enforces it") and 4.3 ("Private vaults are created by a factory per
institution, with their own policy, admin accounts and share class").

## What exists (read 24 Nov 2026)
- `vault-factory::create_vault` deploys a vault; the creator is its admin (D-O: Constella's fee from the factory).
- The vault then needs, all admin-only: `bind_policy` (admin + owner sign; the policy contract records
  owner, hash and terms), `set_sleeves` (must equal the signed targets once bound), `set_keeper`,
  `propose_router` (first router atomic while supply is 0, D-K). Reference sequence: `testnet/blend.py`.
- AI engine `POST /thesis` with `owner` returns the canonical policy, its hash, and `bind_policy` terms
  derived from the hashed policy (OPEN-ISSUES #3 fix).
- Console signer signs one rule id per invocation in the tree (`passkey.ts signingInput`), so nested calls
  need no signer change.
- Gaps: relayer and fee-sponsor only sponsor statically whitelisted contracts (a new vault has a new
  address); the keeper only manages vaults listed in `KEEPER_VAULTS`.

## Research (24 Nov 2026)
- Soroban auth (developers.stellar.org, "Authorization"): an address may `require_auth` several times in
  one transaction; each call needs a node in the signed invocation tree, matched along the call path;
  `__check_auth` runs once per tree. Calls on behalf of the DIRECT invoker contract are authorized
  automatically. -> One signature can cover factory -> vault -> policy.
- Testnet limits (`stellar network settings`, 24 Nov 2026): tx 400M instructions, 40 MiB memory, 200 disk
  reads / 200 writes (400 footprint entries), 132,096 write bytes, 16,384 B events, 132,096 B tx size.
- Protocol fee pattern (Yearn v3 factory, D-O) already applied.

## Design
1. **One transaction, one signature**: `vault_factory.create_vault_with_policy(creator, params, policy)`
   deploys the vault (creator = admin = pause authority), then calls `bind_policy` (owner = creator),
   `set_sleeves` (built from the registry: address, sleeve type, liquidity tier - the creator cannot
   point a sleeve at an arbitrary contract), `set_keeper` (the factory's keeper) and, if configured,
   `propose_router` (the factory's Soroswap router). Every step re-uses the vault's own checks.
   Treasuries with a separate multisig admin keep the step-by-step path (`create_vault` + admin calls).
2. **Factory index**: `vault_count()` / `vault_at(i)` so services can find every factory vault.
3. **Sponsorship**: fee-sponsor trusts vaults proven by registered factories (`is_vault`), bounded list,
   admin-managed; the relayer allows the vault user functions on any `is_vault` contract and caps the
   setup call's fee (it pays the new contract's storage).
4. **Keeper**: discovers vaults from the factory index; still acts only where it is the vault's keeper.
5. **Console**: "Create my vault from this policy" after a signable draft; deposit / redeem / view for
   the user's own vault.

Done when: on testnet, a typed thesis produces a live vault the user owns, enforcing that thesis, with
sponsored deposit and keeper rebalancing.
