/** Invitation links (step 10.6): ?pair (join an account as a new device),
 * ?guard (become someone's guardian), ?recover (approve a recovery). They work
 * signed in or out: a guardian may have no Constella account at all. */
import { useEffect, useState } from "react";
import { browserStore } from "../core/accounts";
import { approveRecovery, becomeGuardian, deviceApproved, joinDevice, readRequest } from "../core/security";
import type { LinkIntent } from "../router";
import { useSession } from "../session";
import { useAction, useAsync } from "../ui/hooks";
import { Badge, Logo } from "../ui/kit";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const done = () => { history.replaceState(null, "", "/"); dispatchEvent(new PopStateEvent("popstate")); };

function Frame({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <main className="c-welcome"><div className="c-sky" style={{ display: "block" }}>
      <div className="top"><Logo /><Badge kind="net">Stellar testnet</Badge></div>
      <section className="c-card c-invite"><h2>{title}</h2>{children}</section>
    </div></main>
  );
}

function JoinDevice({ code }: { code: string }) {
  const { chain, store, setIdentity } = useSession();
  const act = useAction();
  const [check, setCheck] = useState<string | null>(null);
  const go = () => act.run("join", async () => {
    if (store.load()?.address) throw new Error("This browser is already signed in to an account. Sign out first.");
    const { pk, check } = await joinDevice(chain, code);
    setCheck(check);
    for (let i = 0; i < 200; i++) {
      await sleep(3000);
      const a = await deviceApproved(chain, store, pk);
      if (a) { setIdentity({ kind: "passkey", account: a }); done(); return; }
    }
    throw new Error("It was not approved within 10 minutes. Start again from your first device.");
  });
  return (
    <Frame title="Join your account on this device">
      <p>Your other device asked to add this one. Create a passkey here, then approve it on the other device.</p>
      {check ? <div className="c-note" data-check>Check code <b className="mono">{check}</b>. It must match the code on your other device. Waiting for approval…</div>
             : <button className="c-btn c-btn-dark" disabled={!!act.busy} onClick={go} data-act="join">Create a passkey here and join</button>}
      {act.error && <div className="c-note c-note-bad" role="alert">{act.error}</div>}
    </Frame>
  );
}

function BecomeGuardian({ code }: { code: string }) {
  const { chain } = useSession();
  const act = useAction();
  const [check, setCheck] = useState<string | null>(null);
  return (
    <Frame title="Become a guardian">
      <p>Someone asked you to help recover their Constella account if they ever lose their passkeys. Create a guardian passkey on this device.
        It can only ever approve a new passkey for them, after a 48-hour wait they can cancel. It can never move money.</p>
      {check ? <div className="c-note c-note-ok" data-check>You are a guardian. Check code <b className="mono">{check}</b>: tell the account owner; it must match their screen.</div>
             : <button className="c-btn c-btn-dark" disabled={!!act.busy} data-act="become-guardian"
                 onClick={() => act.run("guard", async () => setCheck(await becomeGuardian(chain, browserStore(), code)))}>Create my guardian passkey</button>}
      {act.error && <div className="c-note c-note-bad" role="alert">{act.error}</div>}
    </Frame>
  );
}

function ApproveRecovery({ id }: { id: string }) {
  const { chain } = useSession();
  const q = useAsync(() => readRequest(chain, browserStore(), id), [id]);
  const act = useAction();
  const r = q.data;
  return (
    <Frame title="Approve a recovery">
      {q.error && <div className="c-note c-note-bad">This request is unknown or expired ({q.error}).</div>}
      {r && (
        <>
          <p>Account <span className="addr">{r.account}</span> asks to make a <b>new passkey</b> its owner.</p>
          <div className="c-note">New passkey check code <b className="mono" data-check>{r.check}</b>. Call the owner and confirm they see the same code before you approve.</div>
          <p className="small muted">Approvals: {r.approvals.length} of {r.threshold}. After approval the change waits 48 hours; the owner can cancel it.</p>
          {r.usable.length
            ? <button className="c-btn c-btn-dark" disabled={!!act.busy} data-act="approve-recovery"
                onClick={() => act.run("approve", async () => {
                  const out = await approveRecovery(chain, id, r.entry, r.ruleId, r.usable[0]);
                  return out.submitted ? "Approved. The request is on chain: it takes effect in 48 hours unless the owner cancels."
                                       : `Approved (${out.approvals} of ${out.threshold}). Waiting for the other guardians.`;
                })}>Approve with my guardian passkey</button>
            : <div className="c-note">This device has no guardian passkey for this account. Open the link on the device you used to become a guardian.</div>}
          {act.error && <div className="c-note c-note-bad" role="alert">{act.error}</div>}
          {act.done && <div className="c-note c-note-ok" role="status">{act.done}</div>}
        </>
      )}
    </Frame>
  );
}

export function Invitation({ intent }: { intent: NonNullable<LinkIntent> }) {
  useEffect(() => { document.title = "Constella"; }, []);
  if (intent.kind === "pair") return <JoinDevice code={intent.code} />;
  if (intent.kind === "guard") return <BecomeGuardian code={intent.code} />;
  return <ApproveRecovery id={intent.code} />;
}
