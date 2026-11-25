/** Step 8: a recovery request fixes the entry's expiry up front, so every
 * guardian signs the SAME entry (the relayer mailbox refuses anything else);
 * each guardian signs under the account's recovery rule with its own key. */
import { describe, expect, it } from "vitest";
import { Address, StrKey, xdr } from "@stellar/stellar-sdk";
import { fixExpiry, guardianSign, proposalArgs } from "../src/recovery";
import { createTestPasskey } from "../src/passkey";
import vectors from "./passkey_vectors.json";

const X = (v: any) => v as any;
const VERIFIER = StrKey.encodeContract(new Uint8Array(32).fill(5) as any);

describe("recovery requests", () => {
  const v = (vectors as any[]).find((x) => x.credentials === "address") ?? (vectors as any[])[0];

  it("fixes the expiry and leaves everything else as prepared", () => {
    const base = fixExpiry(v.entry_xdr, 123_456);
    const e = X(xdr.SorobanAuthorizationEntry.fromXDR(base, "base64"));
    const o = X(xdr.SorobanAuthorizationEntry.fromXDR(v.entry_xdr, "base64"));
    const c = (x: any) => x.credentials.address ?? x.credentials.addressV2;
    expect(c(e).signatureExpirationLedger).toBe(123_456);
    expect(c(e).signature.type).toBe("scvVoid");
    expect(c(e).nonce.toString()).toBe(c(o).nonce.toString());
    expect(e.rootInvocation.toXDR("base64")).toBe(o.rootInvocation.toXDR("base64"));
  });

  it("a guardian signs that same entry under the recovery rule with its own key", async () => {
    const base = fixExpiry(v.entry_xdr, 123_456);
    const g = await createTestPasskey();
    const signed = await guardianSign(base, v.passphrase, 7, VERIFIER, { credentialId: g.id, publicKey: g.publicKey, testKey: g.jwk },
                                      "localhost", "http://localhost:5173");
    const e = X(xdr.SorobanAuthorizationEntry.fromXDR(signed, "base64"));
    const c = e.credentials.address ?? e.credentials.addressV2;
    expect(c.signatureExpirationLedger).toBe(123_456);
    const top = c.signature.map;
    const ids = top.find((m: any) => new TextDecoder().decode(m.key.sym.bytes) === "context_rule_ids").val.vec;
    expect(ids.every((i: any) => i.u32 === 7)).toBe(true);
    const signers = top.find((m: any) => new TextDecoder().decode(m.key.sym.bytes) === "signers").val.map;
    expect(signers).toHaveLength(1);
    expect(Buffer.from(signers[0].key.vec[2].bytes.value).toString("hex")).toBe(Buffer.from(g.publicKey).toString("hex"));
  });

  it("the proposal names the account's new owner passkey", () => {
    const key = new Uint8Array(65).fill(9); key[0] = 4;
    const [signer] = proposalArgs(VERIFIER, key);
    const s = X(signer);
    expect(new TextDecoder().decode(s.vec[0].sym.bytes)).toBe("External");
    expect(Address.fromScVal(s.vec[1]).toString()).toBe(VERIFIER);
    expect(Buffer.from(s.vec[2].bytes.value)).toEqual(Buffer.from(key));
  });
});
