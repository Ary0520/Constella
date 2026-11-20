/**
 * Chain + service access for the THROWAWAY console. Reads via RPC
 * simulation; user actions go through the relayer (zero XLM, passkey-signed
 * in the browser); demo/admin actions are signed with the LOCAL throwaway
 * keys from config.json (local network only).
 */
import { Address, Keypair, nativeToScVal, Operation, rpc, scValToNative, TransactionBuilder, xdr } from "@stellar/stellar-sdk";
import { Assertion, attachSignature, getAssertion, signingInput, testAssertion } from "./passkey";

const X = xdr as any;

export interface Config {
  network: { rpc_url: string; passphrase: string; friendbot: string };
  assets: { USDC: string; EURC: string; issuer: string };
  contracts: Record<string, string>;
  accounts: Record<string, string>;
  LOCAL_ONLY_secrets: Record<string, string>;
  services: { relayer: string; thesis_api: string };
}

export const sv = {
  addr: (a: string) => new Address(a).toScVal(),
  i128: (n: bigint) => nativeToScVal(n, { type: "i128" }),
  u32: (n: number) => nativeToScVal(n, { type: "u32" }),
  sym: (s: string) => nativeToScVal(s, { type: "symbol" }),
  str: (s: string) => nativeToScVal(s, { type: "string" }),
  bytes: (b: Uint8Array) => xdr.ScVal.scvBytes(b as any),
};

/** InvokeContract host function, base64 XDR (what the relayer accepts);
 * byte-identical to the relayer's Python func_b64 (test/chain.test.ts). */
export function funcB64(contract: string, fn: string, args: xdr.ScVal[]): string {
  const hf = X.HostFunction.hostFunctionTypeInvokeContract(
    new X.InvokeContractArgs({ contractAddress: new Address(contract).toScAddress(), functionName: fn, args }));
  return hf.toXDR("base64");
}

export class Chain {
  server: rpc.Server;
  constructor(public cfg: Config) { this.server = new rpc.Server(cfg.network.rpc_url, { allowHttp: true }); }

  async view(contract: string, fn: string, args: xdr.ScVal[] = []): Promise<any> {
    const src = await this.server.getAccount(this.cfg.accounts.admin);
    const tx = new TransactionBuilder(src, { fee: "100", networkPassphrase: this.cfg.network.passphrase })
      .addOperation(contractCall(contract, fn, args)).setTimeout(30).build();
    const sim: any = await this.server.simulateTransaction(tx);
    if (sim.error) throw new Error(`${fn}: ${sim.error}`);
    return scValToNative(sim.result.retval);
  }

  /** A demo/admin action signed by a LOCAL throwaway key. */
  async invokeAs(who: string, contract: string, fn: string, args: xdr.ScVal[] = []): Promise<string> {
    const kp = Keypair.fromSecret(this.cfg.LOCAL_ONLY_secrets[who]);
    // One pending transaction per account: TRY_AGAIN_LATER (e.g. two buttons
    // at once) is retried with a fresh sequence number.
    let sent: any;
    for (let attempt = 0; attempt < 5; attempt++) {
      const src = await this.server.getAccount(kp.publicKey());
      let tx = new TransactionBuilder(src, { fee: "1000", networkPassphrase: this.cfg.network.passphrase })
        .addOperation(contractCall(contract, fn, args)).setTimeout(60).build();
      tx = await this.server.prepareTransaction(tx);
      tx.sign(kp);
      sent = await this.server.sendTransaction(tx);
      if (sent.status !== "TRY_AGAIN_LATER") break;
      await new Promise((r) => setTimeout(r, 1500 * (attempt + 1)));
    }
    if (sent.status !== "PENDING" && sent.status !== "DUPLICATE") throw new Error(`${fn}: ${sent.status}`);
    const res: any = await this.server.pollTransaction(sent.hash, { attempts: 30 });
    if (res.status !== "SUCCESS") throw new Error(`${fn}: ${res.status}`);
    return sent.hash;
  }

  async relayer(path: string, body: unknown): Promise<any> {
    const r = await fetch(this.cfg.services.relayer + path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    const out = await r.json();
    if (!r.ok) throw new Error(`${out.error}: ${out.detail ?? ""}`);
    return out;
  }

  /** Zero-XLM user call: relayer prepares the auth entries, the PASSKEY signs
   * them in this browser, the relayer pays and submits. */
  async sponsored(account: Passkey, contract: string, fn: string, args: xdr.ScVal[],
                  assert: (credentialId: string, challenge: Uint8Array) => Promise<Assertion> =
                    (id, ch) => account.testKey
                      ? testAssertion(account.testKey, ch, globalThis.location?.hostname ?? "localhost", globalThis.location?.origin ?? "http://localhost:5173")
                      : getAssertion(id, ch, account.transports ?? [])): Promise<any> {
    const func = funcB64(contract, fn, args);
    const { auth, latest_ledger } = await this.relayer("/prepare", { func });
    const rule = Number(await this.view(account.address, "get_owner_rule"));
    const validUntil = latest_ledger + 60;
    const signed: string[] = [];
    for (const entryXdr of auth) {
      const input = signingInput(entryXdr, validUntil, this.cfg.network.passphrase, rule);
      if (input.account !== account.address) { signed.push(entryXdr); continue; }
      const assertion = await assert(account.credentialId, input.challenge);
      signed.push(attachSignature(input, this.cfg.contracts.verifier, account.publicKey, assertion));
    }
    return this.relayer("/submit", { func, auth: signed });
  }
}

export interface Passkey { credentialId: string; publicKey: Uint8Array; address: string; attachment?: string; transports?: string[];
  /** LOCAL testing only: an in-browser test passkey's private key. */
  testKey?: JsonWebKey }

function contractCall(contract: string, fn: string, args: xdr.ScVal[]) {
  // The same host function the relayer receives.
  return Operation.invokeHostFunction({ func: X.HostFunction.fromXDR(funcB64(contract, fn, args), "base64"), auth: [] });
}

export async function thesis(cfg: Config, body: unknown): Promise<{ status: number; out: any }> {
  const r = await fetch(cfg.services.thesis_api + "/thesis", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  return { status: r.status, out: await r.json() };
}
