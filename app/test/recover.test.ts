/** Public-key recovery from a WebAuthn-style P-256 signature finds the real key. */
import { createSign, generateKeyPairSync } from "node:crypto";
import { recoverPublicKeys } from "../src/core/passkey";

it("one of the (at most two) recovered keys is the signer's, for many signatures", async () => {
  for (let i = 0; i < 10; i++) {
    const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
    const pub = Buffer.from(new Uint8Array(publicKey.export({ type: "spki", format: "der" })).slice(-65)).toString("hex");
    const msg = new Uint8Array(Buffer.from(`authData+clientHash ${i}`));
    const der = new Uint8Array(createSign("SHA256").update(msg).sign(privateKey));
    const cands = (await recoverPublicKeys(msg, der)).map((k) => Buffer.from(k).toString("hex"));
    expect(cands.length).toBeGreaterThanOrEqual(1);
    expect(cands.length).toBeLessThanOrEqual(2);
    expect(cands).toContain(pub);
  }
});
