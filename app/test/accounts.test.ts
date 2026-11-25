/** Identity (step 10.2): stored passkey account, who acts (passkey -> relayer
 * sponsored; wallet -> the user's wallet), and the sign-in lookup order. */
import { describe, expect, it, vi } from "vitest";
import { actorFor, findAccount, memoryStore, PasskeyStore } from "../src/core/accounts";

const PK = new Uint8Array(65).fill(7);
const A = "CDAJDSXBYMHJMGL35WXL73MDRLPLFOSEM4MZ4VZRWNY6F7XMFGAP4QQS";

describe("stored passkey account", () => {
  it("round-trips, including the public key bytes and an added device's rule", () => {
    const s = new PasskeyStore(memoryStore());
    expect(s.load()).toBeNull();
    s.save({ credentialId: "id", publicKey: PK, address: A, ruleId: 3, attachment: "platform" });
    const back = s.load()!;
    expect(back.publicKey).toBeInstanceOf(Uint8Array);
    expect(Array.from(back.publicKey)).toEqual(Array.from(PK));
    expect(back.ruleId).toBe(3);
    s.forget();
    expect(s.load()).toBeNull();
  });
  it("a corrupt entry reads as signed out, never throws", () => {
    const m = memoryStore(); m.setItem("constella-app-account", "{oops");
    expect(new PasskeyStore(m).load()).toBeNull();
  });
});

describe("who acts", () => {
  it("a passkey account goes through the relayer (sponsored); a wallet signs itself", async () => {
    const chain: any = { sponsored: vi.fn(async () => ({ hash: "p" })) };
    const pass = actorFor({ kind: "passkey", account: { credentialId: "i", publicKey: PK, address: A } }, chain);
    expect(pass.address).toBe(A);
    await pass.call("CVAULT", "deposit", []);
    expect(chain.sponsored).toHaveBeenCalledWith(expect.objectContaining({ address: A }), "CVAULT", "deposit", []);
    const invoke = vi.fn(async () => "w");
    const w = actorFor({ kind: "wallet", signer: { address: "GABC", label: "x", sign: async (t: string) => t } }, chain, invoke);
    await w.call("CVAULT", "redeem", []);
    expect(invoke).toHaveBeenCalledWith(chain, expect.objectContaining({ address: "GABC" }), "CVAULT", "redeem", []);
    expect(chain.sponsored).toHaveBeenCalledTimes(1);
  });
  it("an unfinished passkey account cannot act", () => {
    expect(() => actorFor({ kind: "passkey", account: { credentialId: "i", publicKey: PK, address: "" } }, {} as any)).toThrow(/unfinished/);
  });
});

describe("sign-in lookup", () => {
  const cfg = { contracts: { device_registry: "CDEV", recovery_registry: "CREC", account_factory: "CFAC" } };
  const view = (answers: Record<string, any>) => vi.fn(async (c: string, fn: string) => answers[`${c}.${fn}`] ?? null);
  it("an added device signs in under its own rule", async () => {
    const chain: any = { cfg, view: view({ "CDEV.device_of": { account: A, rule_id: 4, name: "phone" } }) };
    expect(await findAccount(chain, PK)).toEqual({ address: A, ruleId: 4, via: "device" });
  });
  it("a recovered owner key is found through the recovery registry", async () => {
    const chain: any = { cfg, view: view({ "CREC.owner_account": A }) };
    expect(await findAccount(chain, PK)).toEqual({ address: A, via: "recovered" });
  });
  it("the original owner key is found through the factory; unknown keys give null", async () => {
    expect(await findAccount({ cfg, view: view({ "CFAC.account_of": A }) } as any, PK)).toEqual({ address: A, via: "owner" });
    expect(await findAccount({ cfg, view: view({}) } as any, PK)).toBeNull();
  });
});
