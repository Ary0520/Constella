/**
 * Step 8 (decision D-F): account recovery in the console.
 * A proposal is `account.propose_recovery(Signer::External(verifier, new key))`,
 * authorized by the account's guardians under its recovery rule. Every
 * guardian signs the SAME entry (expiry fixed up front); the relayer mailbox
 * merges the signatures and submits once the threshold is met. The account
 * enforces the guardians, the threshold, and the 48-hour delay the owner can
 * cancel.
 */
import { Address, xdr } from "@stellar/stellar-sdk";
import { attachSignature, fixExpiry, getAssertion, signingInput, testAssertion } from "./passkey";

export { fixExpiry };

export interface GuardianKey { credentialId: string; publicKey: Uint8Array; testKey?: JsonWebKey; transports?: string[] }

/** propose_recovery arguments: the new owner passkey as an External signer. */
export function proposalArgs(verifier: string, newKey: Uint8Array): xdr.ScVal[] {
  return [xdr.ScVal.scvVec([xdr.ScVal.scvSymbol("External"), new Address(verifier).toScVal(), xdr.ScVal.scvBytes(newKey as any)])];
}

/** The expiry a request's entry carries (all guardians sign this one). */
export function entryExpiry(entryXdr: string): number {
  const e: any = xdr.SorobanAuthorizationEntry.fromXDR(entryXdr, "base64");
  return (e.credentials.address ?? e.credentials.addressV2).signatureExpirationLedger;
}

/** A guardian signs the request's entry under the account's recovery rule. */
export async function guardianSign(entryXdr: string, passphrase: string, recoveryRule: number, verifier: string,
                                   g: GuardianKey, rpId = globalThis.location?.hostname ?? "localhost",
                                   origin = globalThis.location?.origin ?? "http://localhost:5173"): Promise<string> {
  const input = signingInput(entryXdr, entryExpiry(entryXdr), passphrase, recoveryRule);
  const assertion = g.testKey ? await testAssertion(g.testKey, input.challenge, rpId, origin)
                              : await getAssertion(g.credentialId, input.challenge, g.transports ?? []);
  return attachSignature(input, verifier, g.publicKey, assertion);
}
