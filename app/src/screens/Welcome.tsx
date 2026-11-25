/** Signed out: the three ways in (step 10.2). Passkeys need no seed phrase and
 * no XLM (the relayer pays); a Stellar wallet signs and pays its own fees. */
import { useState } from "react";
import { createAccount, finishSetup, signIn } from "../core/accounts";
import { connectWallet } from "../core/wallet";
import { useSession } from "../session";
import { Badge, Icon, Logo } from "../ui/kit";

export function friendly(e: any): string {
  const m = String(e?.message ?? e);
  if (/NotAllowedError|timed out|not allowed/i.test(m)) return "The passkey prompt was closed or timed out. Nothing was created. Try again when you are ready.";
  if (/RATE_LIMITED/.test(m)) return "Too many new accounts from this network today. Try again tomorrow, or connect a Stellar wallet.";
  return m.replace(/^Error:\s*/, "");
}

export function Welcome() {
  const { chain, store, pending, setIdentity } = useSession();
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  async function run(label: string, f: () => Promise<void>) {
    setBusy(label); setErr(null);
    try { await f(); } catch (e) { setErr(friendly(e)); } finally { setBusy(null); }
  }
  const passkey = (where: "platform" | "cross-platform") => run(where, async () => {
    const a = await createAccount(chain, store, where);
    setIdentity({ kind: "passkey", account: a });
  });
  const ways = [
    { id: "platform", icon: "face", title: "Use this device", sub: "Face ID, Touch ID or Windows Hello. No seed phrase, no fees.", go: () => passkey("platform") },
    { id: "cross-platform", icon: "phone", title: "Use my phone", sub: "Scan a code with your phone; its passkey signs.", go: () => passkey("cross-platform") },
    { id: "wallet", icon: "wallet", title: "Connect a Stellar wallet", sub: "Freighter, xBull, LOBSTR, Albedo, Hana. You pay network fees.",
      go: () => run("wallet", async () => setIdentity({ kind: "wallet", signer: await connectWallet(chain.cfg.network.passphrase) })) },
  ];

  return (
    <main className="c-welcome">
      <div className="c-sky">
        <div className="top"><Logo /><Badge kind="net">Stellar testnet</Badge></div>
        <section>
          <h1>Your thesis.<br />One token.</h1>
          <p className="lead">Say what your money is for. Constella turns it into a rule-bound basket of Stellar's
            tokenized real-world assets: one token you hold, redeem any time, and use as collateral.</p>
        </section>
        <section className="c-ways" aria-label="Ways to sign in">
          {pending && (
            <div className="c-note">
              <span>A passkey on this device is saved but its account was not finished.</span>
              <button className="c-btn c-btn-dark c-btn-sm" disabled={!!busy}
                onClick={() => run("finish", async () => setIdentity({ kind: "passkey", account: await finishSetup(chain, store, store.load()!) }))}>
                Finish setup
              </button>
            </div>
          )}
          {ways.map((w) => (
            <button key={w.id} className="c-way" disabled={!!busy} onClick={w.go} data-way={w.id}>
              <span className="ic"><Icon name={w.icon} /></span>
              <span className="txt"><b>{busy === w.id ? "Waiting for you…" : w.title}</b><small>{w.sub}</small></span>
              <span className="go"><Icon name="arrow" /></span>
            </button>
          ))}
          <button className="c-btn c-btn-line" disabled={!!busy} data-way="signin"
            onClick={() => run("signin", async () => setIdentity({ kind: "passkey", account: await signIn(chain, store) }))}>
            <Icon name="key" /> {busy === "signin" ? "Waiting for your passkey…" : "I already have a passkey — sign in"}
          </button>
          {err && <div className="c-note c-note-bad" role="alert">{err}</div>}
        </section>
      </div>
    </main>
  );
}
