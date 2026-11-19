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

`npm run test:stack` runs the console's own code against the live stack (software passkey in place of the
browser): account, sponsored deposit, keeper rebalance, frozen claim settled, Collateral Passport, thesis API.

`npm run test:browser` (after `npx playwright install chromium`) clicks through the page in headless Chromium
with a virtual WebAuthn authenticator: interrupted setup -> Finish setup, faucet, passkey-signed deposit, keeper.

If an action seems to do nothing, read the status bar at the top (every result and error appears there).
No wallet extension is involved: the passkey is the account and the relayer pays the fees.

Passkeys need a browser with a platform authenticator (Touch ID / Windows Hello / phone / security key);
`localhost` counts as a secure context. The signer (`src/passkey.ts`) is byte-identical to the relayer's
network-proven Python signer (`npm test`, vectors in `test/`). Demo/admin buttons use the throwaway local keys
written by `localnet/deploy.py` to `public/config.json` (gitignored).
