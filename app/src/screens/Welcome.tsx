/** Signed out: the three ways in (step 10.2). Passkeys need no seed phrase and
 * no XLM (the relayer pays); a Stellar wallet signs and pays its own fees. */
import { useState } from "react";
import { browserStore, createAccount, finishSetup, signIn } from "../core/accounts";
import { finishRecovery, recoveryState, startRecovery } from "../core/security";
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
  const [lost, setLost] = useState(false);
  const [acct, setAcct] = useState("");
  const [req, setReq] = useState<{ id: string; check: string; threshold: number; guardians: number } | null>(null);
  const started = recoveryState(browserStore()).load();

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
          {started && (
            <div className="c-note"><span>You started recovering <span className="addr">{started.account}</span> on this device.
              Once your guardians approved and 48 hours passed, finish it.</span>
              <button className="c-btn c-btn-dark c-btn-sm" disabled={!!busy} data-act="finish-recovery"
                onClick={() => run("finish", async () => {
                  try { setIdentity({ kind: "passkey", account: await finishRecovery(chain, browserStore(), store) }); }
                  catch (e: any) { throw new Error(/NoPendingRecovery|#9|not.*due|Error\(Contract/i.test(String(e?.message)) ? "Not ready yet: the guardians have not approved, or 48 hours have not passed." : e?.message); }
                })}>Finish recovery</button></div>
          )}
          {!lost ? (
            <button className="c-btn c-btn-line c-btn-sm" style={{ alignSelf: "center", border: 0 }} onClick={() => setLost(true)} data-way="lost">Lost every passkey? Recover your account</button>
          ) : (
            <div className="c-card" style={{ display: "flex", flexDirection: "column", gap: 10 }} data-lost>
              <h3>Recover with your guardians</h3>
              <p className="small">Create a new passkey on this device. Your guardians approve it; it becomes your account's owner 48 hours later.</p>
              <input className="c-input addr" placeholder="Your account address (C…)" value={acct} onChange={(e) => setAcct(e.target.value.trim())} id="recAccount" />
              {req ? (
                <div className="c-note c-note-ok" data-request><span>Send this link to your guardians ({req.threshold} of {req.guardians} must approve):
                  <span className="addr" style={{ display: "block" }}>{`${location.origin}/?recover=${req.id}`}</span>
                  Tell them your check code <b className="mono">{req.check}</b>.</span></div>
              ) : (
                <button className="c-btn c-btn-dark" disabled={!!busy || !acct} data-act="start-recovery"
                  onClick={() => run("recover", async () => setReq(await startRecovery(chain, browserStore(), acct)))}>Create a new passkey and ask my guardians</button>
              )}
            </div>
          )}
          {err && <div className="c-note c-note-bad" role="alert">{err}</div>}
        </section>
      </div>
    </main>
  );
}
