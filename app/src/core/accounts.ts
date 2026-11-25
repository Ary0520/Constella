/**
 * Identity (step 10.2). Ported from the console's proven flows, without the
 * DOM: storage and chain are passed in, so every rule here is unit-tested.
 *
 * - A passkey account (smart account; the relayer pays, no XLM) is stored in
 *   this browser: credential id, public key, address, and the rule an ADDED
 *   device signs under (the first device signs under the owner rule).
 * - A Stellar wallet (classic account) lives in memory only: reconnect after a
 *   reload; it signs and pays its own fees.
 */
import { xdr } from "@stellar/stellar-sdk";
import { Chain, Passkey, sv } from "./chain";
import { createPasskey, discoverPasskey, getAssertion, recoverPublicKeys, Where } from "./passkey";
import { WalletSigner, walletInvoke } from "./wallet";

export interface KV { getItem(k: string): string | null; setItem(k: string, v: string): void; removeItem(k: string): void }

export function memoryStore(): KV {
  const m = new Map<string, string>();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v), removeItem: (k) => void m.delete(k) };
}

/** localStorage when the browser allows it (private modes may throw). */
export function browserStore(): KV {
  try {
    const t = "__constella_probe";
    localStorage.setItem(t, "1");
    localStorage.removeItem(t);
    return localStorage;
  } catch {
    return memoryStore();
  }
}

const KEY = "constella-app-account";

export class PasskeyStore {
  constructor(private kv: KV) {}
  load(): Passkey | null {
    try {
      const raw = this.kv.getItem(KEY);
      if (!raw) return null;
      const a = JSON.parse(raw);
      if (!a || typeof a.credentialId !== "string" || !Array.isArray(a.publicKey)) return null;
      return { ...a, publicKey: Uint8Array.from(a.publicKey) };
    } catch {
      return null;
    }
  }
  save(a: Passkey) {
    try { this.kv.setItem(KEY, JSON.stringify({ ...a, publicKey: Array.from(a.publicKey) })); } catch { /* storage full / blocked */ }
  }
  forget() {
    try { this.kv.removeItem(KEY); } catch { /* */ }
  }
}

export type Identity = { kind: "passkey"; account: Passkey } | { kind: "wallet"; signer: WalletSigner };

export interface Actor { address: string; kind: Identity["kind"]; call(contract: string, fn: string, args: xdr.ScVal[]): Promise<any> }

/** Who acts: the passkey account through the relayer (sponsored), or the
 * wallet signing and paying itself. */
export function actorFor(id: Identity, chain: Chain, invoke = walletInvoke): Actor {
  if (id.kind === "passkey") {
    const a = id.account;
    if (!a.address) throw new Error("account setup is unfinished");
    return { address: a.address, kind: "passkey", call: (c, fn, args) => chain.sponsored(a, c, fn, args) };
  }
  const w = id.signer;
  return { address: w.address, kind: "wallet", call: (c, fn, args) => invoke(chain, w, c, fn, args) };
}

export type Found = { address: string; ruleId?: number; via: "device" | "recovered" | "owner" };

/** Which account a public key controls: an added device (its own rule), a
 * key that became owner through recovery, or an original owner key. */
export async function findAccount(chain: Chain, pk: Uint8Array): Promise<Found | null> {
  const c = chain.cfg.contracts;
  if (c.device_registry) {
    const dev = await chain.view(c.device_registry, "device_of", [sv.bytes(pk)]);
    if (dev) return { address: String(dev.account), ruleId: Number(dev.rule_id), via: "device" };
  }
  if (c.recovery_registry) {
    const rec = await chain.view(c.recovery_registry, "owner_account", [sv.bytes(pk)]);
    if (rec) return { address: String(rec), via: "recovered" };
  }
  const acct = await chain.view(c.account_factory, "account_of", [sv.bytes(pk)]);
  return acct ? { address: String(acct), via: "owner" } : null;
}

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

/** Ask the relayer to deploy the smart account for this passkey (idempotent:
 * the address is derived from the key). */
export async function finishSetup(chain: Chain, store: PasskeyStore, a: Passkey): Promise<Passkey> {
  const out = await chain.relayer("/accounts", { owner_key: hex(a.publicKey) });
  const done = { ...a, address: String(out.account) };
  store.save(done);
  return done;
}

/** New passkey on this device or a phone, then the account. The passkey is
 * saved BEFORE the relayer call, so an interrupted setup can be finished. */
export async function createAccount(chain: Chain, store: PasskeyStore, where: Where): Promise<Passkey> {
  const existing = store.load();
  if (existing && !existing.address) return finishSetup(chain, store, existing);
  if (existing) throw new Error("This browser is already signed in. Sign out first.");
  const pk = await createPasskey(`Constella ${new Date().toISOString().slice(0, 10)}`, where);
  const pending: Passkey = { credentialId: pk.id, publicKey: pk.publicKey, address: "", attachment: pk.attachment, transports: pk.transports };
  store.save(pending);
  return finishSetup(chain, store, pending);
}

/** Sign in with any passkey this device holds for the site. */
export async function signIn(chain: Chain, store: PasskeyStore): Promise<Passkey & { via: Found["via"] | "new" }> {
  let found;
  try {
    found = await discoverPasskey();
  } catch (e: any) {
    throw new Error(`This device has no Constella passkey for ${location.hostname}. Create one, or sign in on the device that has it.`);
  }
  for (const pk of found.candidates) {
    const f = await findAccount(chain, pk);
    if (f) {
      const a: Passkey = { credentialId: found.id, publicKey: pk, address: f.address, ruleId: f.ruleId, attachment: "existing" };
      store.save(a);
      return { ...a, via: f.via };
    }
  }
  // A passkey without an account (an interrupted setup): a second signature
  // pins down the exact public key, then setup finishes.
  const challenge = crypto.getRandomValues(new Uint8Array(32));
  const a2 = await getAssertion(found.id, challenge);
  const clientHash = new Uint8Array(await crypto.subtle.digest("SHA-256", a2.clientDataJSON as BufferSource));
  const second = (await recoverPublicKeys(new Uint8Array([...a2.authenticatorData, ...clientHash]), a2.signature)).map(hex);
  const key = found.candidates.find((k) => second.includes(hex(k)));
  if (!key) throw new Error("Could not read this passkey's public key. Try again.");
  const done = await finishSetup(chain, store, { credentialId: found.id, publicKey: key, address: "", attachment: "existing" });
  return { ...done, via: "new" };
}
