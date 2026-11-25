/**
 * Step 10.6: devices, guardians and recovery - ported from the console's flows
 * proven on testnet (pairing e2e, recovery e2e), without the DOM.
 *
 * Devices: the first device (owner passkey) opens a pairing code at the
 * relayer; the new device joins with its public key; both show the same check
 * code; the first device approves (device_registry.add_device).
 * Guardians (D-F / D-Q): invited the same way; installed as ONE M-of-N rule
 * (recovery_registry.set_guardians). Recovery: a new passkey requests
 * account.propose_recovery; guardians approve through the relayer mailbox;
 * after 48 h `/recovery/finish` makes it the owner; the owner can cancel.
 */
import { Address, xdr } from "@stellar/stellar-sdk";
import { Chain, funcB64, Passkey, sv } from "./chain";
import { createPasskey, createTestPasskey } from "./passkey";
import { fixExpiry, GuardianKey, guardianSign, proposalArgs } from "./recovery";
import type { KV, PasskeyStore } from "./accounts";

export const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
export const fromHex = (h: string) => Uint8Array.from(h.match(/../g) ?? [], (x) => parseInt(x, 16));

/** The relayer's pairing check code: SHA-256(public key), first 6 hex, upper. */
export async function checkCode(key: Uint8Array): Promise<string> {
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", key as BufferSource));
  return hex(d).slice(0, 6).toUpperCase();
}

const read = (kv: KV, k: string) => { try { return JSON.parse(kv.getItem(k) ?? "null"); } catch { return null; } };
const write = (kv: KV, k: string, v: unknown) => { try { kv.setItem(k, JSON.stringify(v)); } catch { /* blocked */ } };

/** Guardian passkeys created on this device (to approve others' recoveries). */
export function guardianKeys(kv: KV) {
  const K = "constella-app-guardian-keys";
  return {
    list(): GuardianKey[] {
      const raw = read(kv, K);
      return Array.isArray(raw) ? raw.map((g: any) => ({ ...g, publicKey: fromHex(g.publicKey) })) : [];
    },
    add(g: GuardianKey) {
      const raw = Array.isArray(read(kv, K)) ? read(kv, K) : [];
      raw.push({ ...g, publicKey: hex(g.publicKey) });
      write(kv, K, raw);
    },
  };
}

export interface PendingRecovery { account: string; id: string; credentialId: string; publicKey: Uint8Array; testKey?: JsonWebKey; transports?: string[] }

/** A recovery started on this device (finish it after 48 h). */
export function recoveryState(kv: KV) {
  const K = "constella-app-recovery";
  return {
    load(): PendingRecovery | null {
      const r = read(kv, K);
      return r && typeof r.account === "string" ? { ...r, publicKey: fromHex(r.publicKey) } : null;
    },
    save(r: PendingRecovery) { write(kv, K, { ...r, publicKey: hex(r.publicKey) }); },
    clear() { try { kv.removeItem(K); } catch { /* */ } },
  };
}

/** Only the owner passkey (first device) manages devices and guardians. */
export function securityView(id: { kind: "passkey" | "wallet"; ruleId?: number }) {
  return { manage: id.kind === "passkey" && id.ruleId === undefined, passkey: id.kind === "passkey" };
}

export async function readSecurity(chain: Chain, account: string) {
  const c = chain.cfg.contracts;
  const [devices, guardians, pending] = await Promise.all([
    c.device_registry ? chain.view(c.device_registry, "devices_of", [sv.addr(account)]).catch(() => []) : [],
    c.recovery_registry ? chain.view(c.recovery_registry, "guardians_of", [sv.addr(account)]).catch(() => null) : null,
    chain.view(account, "get_pending_recovery").catch(() => null),
  ]);
  return { devices: devices as { name: string; rule_id: number }[], guardians, pending };
}

// ------------------------------------------------------------ pairing
export const startPairing = async (chain: Chain, account: string): Promise<string> =>
  (await chain.relayer("/pair/start", { account })).code;

export const pairingStatus = (chain: Chain, code: string): Promise<{ device_key?: string; name?: string; check?: string }> =>
  chain.relayer("/pair/status", { code });

export const approveDevice = (chain: Chain, owner: Passkey, keyHex: string, name: string) =>
  chain.sponsored(owner, chain.cfg.contracts.device_registry, "add_device", [sv.addr(owner.address), sv.bytes(fromHex(keyHex)), sv.str(name)]);

async function newPasskey(label: string, test: boolean) {
  if (test) {
    const t = await createTestPasskey();
    return { id: t.id, publicKey: t.publicKey, jwk: t.jwk as JsonWebKey | undefined, transports: [] as string[], attachment: "test" };
  }
  const p = await createPasskey(label, "platform");
  return { ...p, jwk: undefined };
}

/** New device: create its passkey and join the pairing; returns its check code. */
export async function joinDevice(chain: Chain, code: string) {
  const pk = await newPasskey(`Constella ${new Date().toISOString().slice(0, 10)}`, false);
  const name = /Android|iPhone|iPad/i.test(navigator.userAgent) ? "phone" : "computer";
  const j = await chain.relayer("/pair/join", { code, device_key: hex(pk.publicKey), name });
  return { pk, check: String(j.check ?? await checkCode(pk.publicKey)) };
}

/** After the owner approves: this device signs in under its own rule. */
export async function deviceApproved(chain: Chain, store: PasskeyStore, pk: { id: string; publicKey: Uint8Array; attachment?: string; transports?: string[] }) {
  const dev = await chain.view(chain.cfg.contracts.device_registry, "device_of", [sv.bytes(pk.publicKey)]);
  if (!dev) return null;
  const a: Passkey = { credentialId: pk.id, publicKey: pk.publicKey, address: String(dev.account), ruleId: Number(dev.rule_id),
                       attachment: pk.attachment, transports: pk.transports };
  store.save(a);
  return a;
}

// ------------------------------------------------------------ guardians
export async function becomeGuardian(chain: Chain, kv: KV, code: string, test = false) {
  const g = await newPasskey(`Constella guardian ${new Date().toISOString().slice(0, 10)}`, test);
  guardianKeys(kv).add({ credentialId: g.id, publicKey: g.publicKey, testKey: g.jwk, transports: g.transports });
  const st = await chain.relayer("/pair/join", { code, device_key: hex(g.publicKey), name: "guardian" });
  return String(st.check ?? await checkCode(g.publicKey));
}

export const saveGuardians = (chain: Chain, owner: Passkey, keysHex: string[], threshold: number) =>
  chain.sponsored(owner, chain.cfg.contracts.recovery_registry, "set_guardians",
    [sv.addr(owner.address), xdr.ScVal.scvVec(keysHex.map((k) => sv.bytes(fromHex(k)))), sv.u32(threshold)]);

export const cancelRecovery = (chain: Chain, owner: Passkey) => chain.sponsored(owner, owner.address, "cancel_recovery", []);

// ------------------------------------------------------------ recovery
/** New device: a new passkey, then a request every guardian signs as is. */
export async function startRecovery(chain: Chain, kv: KV, account: string, test = false) {
  if (!/^C[A-Z2-7]{55}$/.test(account)) throw new Error("Enter the account's address (it starts with C).");
  const gs = await chain.view(chain.cfg.contracts.recovery_registry, "guardians_of", [sv.addr(account)]);
  if (!gs) throw new Error("This account has no guardians, so it cannot be recovered this way.");
  const pk = await newPasskey(`Constella ${new Date().toISOString().slice(0, 10)}`, test);
  const func = funcB64(account, "propose_recovery", proposalArgs(chain.cfg.contracts.verifier, pk.publicKey));
  const { auth, latest_ledger } = await chain.relayer("/prepare", { func });
  const mine = (auth as string[]).find((e) => {
    const c: any = (xdr.SorobanAuthorizationEntry.fromXDR(e, "base64") as any).credentials;
    const creds = c.address ?? c.addressV2;
    return creds && Address.fromScAddress(creds.address).toString() === account;
  });
  if (!mine) throw new Error("Could not prepare the recovery request.");
  const entry = fixExpiry(mine, latest_ledger + 120_960); // ~7 days for the guardians
  const req = await chain.relayer("/recovery/request", { account, new_key: hex(pk.publicKey), threshold: Number(gs.threshold), func, entry });
  recoveryState(kv).save({ account, id: req.id, credentialId: pk.id, publicKey: pk.publicKey, testKey: pk.jwk, transports: pk.transports });
  return { id: String(req.id), check: await checkCode(pk.publicKey), threshold: Number(gs.threshold), guardians: gs.guardians.length as number };
}

export async function readRequest(chain: Chain, kv: KV, id: string) {
  const r = await chain.relayer("/recovery/get", { id });
  const gs = await chain.view(chain.cfg.contracts.recovery_registry, "guardians_of", [sv.addr(r.account)]);
  const usable = guardianKeys(kv).list().filter((k) => gs?.guardians.some((g: any) => hex(new Uint8Array(g)) === hex(k.publicKey)));
  return { ...r, check: await checkCode(fromHex(r.new_key)), ruleId: Number(gs?.rule_id), usable };
}

export async function approveRecovery(chain: Chain, id: string, entry: string, ruleId: number, g: GuardianKey) {
  const signed = await guardianSign(entry, chain.cfg.network.passphrase, ruleId, chain.cfg.contracts.verifier, g);
  return chain.relayer("/recovery/approve", { id, entry: signed });
}

/** After 48 h: the relayer finishes it; this device becomes the owner. */
export async function finishRecovery(chain: Chain, kv: KV, store: PasskeyStore): Promise<Passkey> {
  const rec = recoveryState(kv).load();
  if (!rec) throw new Error("No recovery was started on this device.");
  await chain.relayer("/recovery/finish", { account: rec.account });
  const a: Passkey = { credentialId: rec.credentialId, publicKey: rec.publicKey, address: rec.account, attachment: "recovered",
                       transports: rec.transports, testKey: rec.testKey };
  store.save(a);
  recoveryState(kv).clear();
  return a;
}
