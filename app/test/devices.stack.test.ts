/**
 * One account, several devices - on the LIVE local stack with REAL
 * signatures (two in-browser WebCrypto passkeys, exactly the WebAuthn format
 * the smart account verifies). Opt-in: STACK_E2E=1 (config from localnet/render.py).
 */
import { readFileSync } from "node:fs";
import { Chain, Config, Passkey, sv } from "../src/core/chain";
import { createTestPasskey, testAssertion } from "../src/core/passkey";

const run = process.env.STACK_E2E === "1" ? describe : describe.skip;
const ONE = 10_000_000n;
const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");

run("one account, several devices (real signatures, live local stack)", () => {
  const cfg: Config = JSON.parse(readFileSync("config.local.json", "utf8"));
  const chain = new Chain(cfg); const c = cfg.contracts;
  let A: Passkey, B: Passkey;
  let keyA: JsonWebKey, keyB: JsonWebKey;
  const signer = (jwk: JsonWebKey) => (_id: string, ch: Uint8Array) => testAssertion(jwk, ch, "localhost", "http://localhost:5173");

  it("device B pairs and the account (device A's passkey) approves it", async () => {
    const ta = await createTestPasskey(); keyA = ta.jwk;
    const out = await chain.relayer("/accounts", { owner_key: hex(ta.publicKey) });
    A = { credentialId: "a", publicKey: ta.publicKey, address: out.account, testKey: ta.jwk };
    const tb = await createTestPasskey(); keyB = tb.jwk;
    const { code } = await chain.relayer("/pair/start", { account: A.address });
    const j = await chain.relayer("/pair/join", { code, device_key: hex(tb.publicKey), name: "phone" });
    const st = await chain.relayer("/pair/status", { code });
    expect(st.check).toBe(j.check);                                  // same code on both screens
    await chain.sponsored(A, c.device_registry, "add_device", [sv.addr(A.address), sv.bytes(tb.publicKey), sv.str("phone")], signer(keyA));
    const link = await chain.view(c.device_registry, "device_of", [sv.bytes(tb.publicKey)]);
    expect(String(link.account)).toBe(A.address);
    B = { credentialId: "b", publicKey: tb.publicKey, address: A.address, ruleId: Number(link.rule_id), testKey: tb.jwk };
  }, 180_000);

  it("device B uses the account ON ITS OWN (signs alone under its rule)", async () => {
    await chain.invokeAs("issuer", cfg.assets.USDC, "mint", [sv.addr(A.address), sv.i128(100n * ONE)]);
    await chain.sponsored(B, c.vault, "deposit", [sv.i128(30n * ONE), sv.addr(A.address), sv.addr(A.address), sv.addr(A.address)], signer(keyB));
    expect(BigInt(await chain.view(c.vault, "balance", [sv.addr(A.address)]))).toBeGreaterThan(0n);
    // ...and device A still works too.
    await chain.sponsored(A, c.vault, "deposit", [sv.i128(10n * ONE), sv.addr(A.address), sv.addr(A.address), sv.addr(A.address)], signer(keyA));
  }, 180_000);

  it("an added device cannot add further devices (only the first device manages the account)", async () => {
    const tc = await createTestPasskey();
    await expect(chain.sponsored(B, c.device_registry, "add_device", [sv.addr(A.address), sv.bytes(tc.publicKey), sv.str("x")], signer(keyB)))
      .rejects.toThrow();
    expect(await chain.view(c.device_registry, "device_of", [sv.bytes(tc.publicKey)])).toBeNull();
  }, 120_000);
});
