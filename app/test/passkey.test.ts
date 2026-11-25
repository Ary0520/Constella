/** The TypeScript signer must compute byte-identical payloads and passkey
 * challenges to the Python signer proven on a real network (vectors generated
 * by services/relayer passkey.py: legacy ADDRESS and CAP-71 ADDRESS_V2). */
import { createSign, generateKeyPairSync } from "node:crypto";
import { scValToNative, xdr } from "@stellar/stellar-sdk";
import { attachSignature, derToLowS, signingInput } from "../src/core/passkey";
import vectors from "./passkey_vectors.json";

const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");

describe("passkey signing matches the proven Python implementation", () => {
  for (const v of vectors) {
    it(`payload and challenge are identical (${v.credentials})`, () => {
      const s = signingInput(v.entry_xdr, v.valid_until, v.passphrase, v.rule_ids[0]);
      expect(s.account).toBe(v.account);
      expect(s.ruleIds).toEqual(v.rule_ids);
      expect(hex(s.payload)).toBe(v.payload_hex);
      expect(hex(s.challenge)).toBe(v.challenge_hex);
    });
  }
});

describe("signature encoding", () => {
  it("DER -> low-S r||s, and the entry carries the OZ AuthPayload", () => {
    const N = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551n;
    const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
    for (let i = 0; i < 20; i++) {
      const der = createSign("SHA256").update(`m${i}`).sign(privateKey);
      const rs = derToLowS(new Uint8Array(der));
      expect(rs.length).toBe(64);
      expect(BigInt("0x" + hex(rs.slice(32))) <= N / 2n).toBe(true);
    }
    const spki = publicKey.export({ type: "spki", format: "der" });
    const pub = new Uint8Array(spki).slice(-65);
    const v = vectors[1];
    const input = signingInput(v.entry_xdr, v.valid_until, v.passphrase, 7);
    const der = new Uint8Array(createSign("SHA256").update("x").sign(privateKey));
    const out = attachSignature(input, v.account, pub, { authenticatorData: new Uint8Array(37), clientDataJSON: new Uint8Array(10), signature: der });
    const entry: any = xdr.SorobanAuthorizationEntry.fromXDR(out, "base64");
    const creds = entry.credentials.addressV2;
    expect(creds.signatureExpirationLedger).toBe(v.valid_until);
    const sig = scValToNative(creds.signature);
    expect(Object.keys(sig).sort()).toEqual(["context_rule_ids", "signers"]);
    expect(sig.context_rule_ids).toEqual([7, 7]);
    // Re-deriving the signing input from the SIGNED entry gives the same challenge.
    expect(hex(signingInput(out, v.valid_until, v.passphrase, 7).challenge)).toBe(v.challenge_hex);
  });
});
