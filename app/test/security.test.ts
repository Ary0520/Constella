/** Step 10.6: devices, guardians, recovery - the parts that must be exact. */
import { checkCode, guardianKeys, recoveryState, securityView } from "../src/core/security";
import { memoryStore } from "../src/core/accounts";

const hexKey = (b: string) => Uint8Array.from(("04" + b.repeat(64)).match(/../g)!.map((h) => parseInt(h, 16)));

describe("check codes", () => {
  it("match the relayer's (Python pairing.check_code)", async () => {
    expect(await checkCode(hexKey("11"))).toBe("F0B931");
    expect(await checkCode(hexKey("ab"))).toBe("8A2C85");
  });
});

describe("guardian keys and pending recoveries kept on this device", () => {
  it("guardian passkeys accumulate and round-trip their public keys", () => {
    const g = guardianKeys(memoryStore());
    expect(g.list()).toEqual([]);
    g.add({ credentialId: "a", publicKey: hexKey("11") });
    g.add({ credentialId: "b", publicKey: hexKey("ab") });
    expect(g.list().map((k) => k.credentialId)).toEqual(["a", "b"]);
    expect(g.list()[1].publicKey).toEqual(hexKey("ab"));
  });
  it("a started recovery is remembered until finished", () => {
    const r = recoveryState(memoryStore());
    expect(r.load()).toBeNull();
    r.save({ account: "CX", id: "ABCD2345", credentialId: "n", publicKey: hexKey("11") });
    expect(r.load()!.publicKey).toEqual(hexKey("11"));
    r.clear();
    expect(r.load()).toBeNull();
  });
});

describe("what the account screen may offer", () => {
  it("only the first device (owner passkey) manages devices and guardians; a wallet has neither", () => {
    expect(securityView({ kind: "passkey", ruleId: undefined })).toEqual({ manage: true, passkey: true });
    expect(securityView({ kind: "passkey", ruleId: 3 })).toEqual({ manage: false, passkey: true });
    expect(securityView({ kind: "wallet" })).toEqual({ manage: false, passkey: false });
  });
});
