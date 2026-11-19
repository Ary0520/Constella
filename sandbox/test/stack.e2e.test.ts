/**
 * The console's OWN code (src/chain.ts, src/passkey.ts) against the LIVE
 * local stack (localnet/up.sh). Opt-in: SANDBOX_E2E=1. A software WebAuthn
 * authenticator (P-256, same shape as the relayer's SoftwarePasskey) stands
 * in for the browser's navigator.credentials; everything else is unchanged.
 */
import { createHash, createSign, generateKeyPairSync } from "node:crypto";
import { readFileSync } from "node:fs";
import { xdr } from "@stellar/stellar-sdk";
import { Chain, Config, Passkey, sv, thesis } from "../src/chain";
import type { Assertion } from "../src/passkey";

const run = process.env.SANDBOX_E2E === "1" ? describe : describe.skip;
const ONE = 10_000_000n;
const sha = (b: Uint8Array | string) => new Uint8Array(createHash("sha256").update(b).digest());
const b64u = (b: Uint8Array) => Buffer.from(b).toString("base64url");
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function softwarePasskey() {
  const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const pub = new Uint8Array(publicKey.export({ type: "spki", format: "der" })).slice(-65);
  let counter = 0;
  const assert = async (_id: string, challenge: Uint8Array): Promise<Assertion> => {
    counter += 1;
    const authenticatorData = new Uint8Array([...sha("localhost"), 0x05, 0, 0, 0, counter]); // UP | UV
    const clientDataJSON = new TextEncoder().encode(JSON.stringify({ type: "webauthn.get", challenge: b64u(challenge), origin: "http://localhost:5173", crossOrigin: false }));
    const msg = new Uint8Array([...authenticatorData, ...sha(clientDataJSON)]);
    const signature = new Uint8Array(createSign("SHA256").update(msg).sign(privateKey)); // DER, like a browser
    return { authenticatorData, clientDataJSON, signature };
  };
  return { pub, assert };
}

async function until<T>(what: string, f: () => Promise<T>, ok: (v: T) => boolean, secs = 90): Promise<T> {
  for (let i = 0; i < secs / 3; i++) { const v = await f(); if (ok(v)) return v; await sleep(3000); }
  throw new Error(`timed out waiting for ${what}`);
}

run("console code on the live local stack", () => {
  const cfg: Config = JSON.parse(readFileSync("public/config.json", "utf8"));
  const chain = new Chain(cfg);
  const pk = softwarePasskey();
  let acct: Passkey;
  const c = cfg.contracts;

  it("the relayer creates a zero-XLM passkey account", async () => {
    const out = await chain.relayer("/accounts", { owner_key: Buffer.from(pk.pub).toString("hex") });
    acct = { credentialId: "software", publicKey: pk.pub, address: out.account };
    expect(out.account.startsWith("C")).toBe(true);
    await chain.invokeAs("issuer", cfg.assets.USDC, "mint", [sv.addr(acct.address), sv.i128(1_000n * ONE)]);
    expect(BigInt(await chain.view(cfg.assets.USDC, "balance", [sv.addr(acct.address)]))).toBe(1_000n * ONE);
  }, 120_000);

  it("a deposit signed by the passkey in 'the browser', paid by the relayer; the keeper rebalances", async () => {
    const out = await chain.sponsored(acct, c.vault, "deposit", [sv.i128(500n * ONE), sv.addr(acct.address), sv.addr(acct.address), sv.addr(acct.address)], pk.assert);
    expect(out.user).toBe(acct.address);
    expect(BigInt(await chain.view(c.vault, "balance", [sv.addr(acct.address)]))).toBeGreaterThan(0n);
    const sleeves = await until("the keeper to buy EURC", () => chain.view(c.vault, "get_sleeves"),
      (s: any[]) => BigInt(s.find((x) => x.asset_id === "EURC").current_balance) > 0n);
    expect(BigInt(sleeves.find((x: any) => x.asset_id === "EURC").current_balance)).toBeGreaterThan(0n);
  }, 180_000);

  it("frozen EURC -> redeem with a claim -> prices back -> the keeper settles it", async () => {
    await chain.invokeAs("admin", c.oracle, "trigger_circuit_breaker", [sv.addr(cfg.accounts.admin), sv.sym("CASH"), sv.sym("E2E")]);
    await chain.sponsored(acct, c.vault, "redeem_frozen", [sv.i128(50n * ONE), sv.addr(acct.address), sv.addr(acct.address), sv.addr(acct.address)], pk.assert);
    const owed = BigInt(await chain.view(c.vault, "get_claim", [sv.addr(acct.address), sv.addr(cfg.assets.EURC)]));
    expect(owed).toBeGreaterThan(0n);
    await chain.invokeAs("admin", c.oracle, "clear_circuit_breaker", [sv.sym("CASH")]);
    await until("the keeper to settle the claim", () => chain.view(c.vault, "get_claim", [sv.addr(acct.address), sv.addr(cfg.assets.EURC)]),
      (v: any) => BigInt(v) === 0n);
  }, 240_000);

  it("the Collateral Passport feed serves the share at the vault's conservative NAV", async () => {
    const share = await chain.view(c.collateral_oracle, "lastprice", [xdr.ScVal.scvVec([sv.sym("Stellar"), sv.addr(c.vault)])]);
    const nav = await chain.view(c.vault, "get_nav_conservative");
    expect(BigInt(share.price)).toBe(BigInt(nav.price));
  }, 60_000);

  it("the thesis API drafts a signed policy from the on-chain registry", async () => {
    const { status, out } = await thesis(cfg, { text: "Safe runway in dollars, keep at least 10% cash.", jurisdiction: "DE", investor_class: "retail" });
    expect(status).toBe(200);
    expect(out.bind_policy.sleeves.find((s: any) => s.asset_id === "USDC").sleeve_type).toBe("numeraire");
  }, 60_000);
});
