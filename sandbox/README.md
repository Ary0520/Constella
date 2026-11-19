# Constella sandbox (THROWAWAY)

A plain feature console for the **local network only**, to click through every implemented flow while the
real product UI is designed. Not the product, no design. **Delete this folder to remove it entirely** -
nothing else in the repo imports it, and it talks only to the real interfaces (Stellar RPC, relayer HTTP API,
thesis API).

```
stellar contract build          # release WASMs (contracts/)
localnet/up.sh                  # local chain + every contract + relayer, keeper, watchdog, thesis API
cd sandbox && npm install && npm run dev   # http://localhost:5173
localnet/down.sh                # stop and forget the chain
```

Flows: vault overview (conservative / deposit NAV, holdings, high-water mark, realised return from on-chain
history), passkey account created by the relayer (zero XLM), deposit / redeem signed by the passkey in the
browser and paid by the relayer, AI thesis -> signed policy (+ exact on-chain terms), frozen asset -> claim ->
keeper settles, Collateral Passport share price, admin pause.

Passkeys need a browser with a platform authenticator (Touch ID / Windows Hello / phone / security key);
`localhost` counts as a secure context. The signer (`src/passkey.ts`) is byte-identical to the relayer's
network-proven Python signer (`npm test`, vectors in `test/`). Demo/admin buttons use the throwaway local keys
written by `localnet/deploy.py` to `public/config.json` (gitignored).
