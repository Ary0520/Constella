/** Account & security (step 10.6): identity, devices, guardians, recovery. */
import { useEffect, useRef, useState } from "react";
import type { Chain } from "../core/chain";
import { approveDevice, cancelRecovery, pairingStatus, readSecurity, saveGuardians, securityView, startPairing } from "../core/security";
import { useSession } from "../session";
import { useAction, useAsync } from "../ui/hooks";
import { Badge } from "../ui/kit";
import { Page } from "../ui/Shell";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const linkFor = (kind: string, code: string) => `${location.origin}/?${kind}=${code}`;

/** Owner side of a pairing: show the link, wait for the other side, return what joined. */
function usePairing(chain: Chain) {
  const alive = useRef(true);
  useEffect(() => () => { alive.current = false; }, []);
  return async (code: string, onWaiting: () => void) => {
    onWaiting();
    for (let i = 0; i < 300 && alive.current; i++) {
      await sleep(2000);
      const st = await pairingStatus(chain, code);
      if (st.device_key) return st as { device_key: string; name: string; check: string };
    }
    throw new Error("Nobody joined within 10 minutes. Start again.");
  };
}

function Copy({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return <button className="c-btn c-btn-line c-btn-sm" onClick={() => navigator.clipboard?.writeText(text).then(() => { setDone(true); setTimeout(() => setDone(false), 1500); })}>
    {done ? "Copied" : "Copy"}</button>;
}

export function Account() {
  const { chain, identity, signOut } = useSession();
  const wait = usePairing(chain);
  const passkey = identity?.kind === "passkey" ? identity.account : null;
  const address = passkey?.address ?? (identity?.kind === "wallet" ? identity.signer.address : "");
  const view = securityView(identity?.kind === "passkey" ? { kind: "passkey", ruleId: passkey!.ruleId } : { kind: "wallet" });
  const q = useAsync(() => (view.passkey ? readSecurity(chain, address) : Promise.resolve(null)), [address]);
  const act = useAction();
  const [panel, setPanel] = useState<React.ReactNode>(null);
  const [joined, setJoined] = useState<{ key: string; check: string }[]>([]);
  const [threshold, setThreshold] = useState(1);

  const addDevice = () => act.run("device", async () => {
    const code = await startPairing(chain, address);
    const st = await wait(code, () => setPanel(<div className="c-note"><span><b>On your other device</b>, open <span className="addr">{linkFor("pair", code)}</span>
      {" "}<Copy text={linkFor("pair", code)} /> and create its passkey. Waiting for it (10 minutes)…</span></div>));
    const ok = await new Promise<boolean>((res) => setPanel(
      <div className="c-note" data-approve><span>A <b>{st.name}</b> wants to join. Check code <b className="mono">{st.check}</b> must match the other screen.</span>
        <button className="c-btn c-btn-dark c-btn-sm" data-act="approve-device" onClick={() => res(true)}>Approve</button>
        <button className="c-btn c-btn-line c-btn-sm" onClick={() => res(false)}>Not mine</button></div>));
    if (!ok) { setPanel(null); return "Rejected. Nothing was added."; }
    setPanel(<div className="c-note">Approving with your passkey…</div>);
    await approveDevice(chain, passkey!, st.device_key, st.name);
    setPanel(null); q.reload();
    return `The ${st.name} was added. It can use this account on its own.`;
  });

  const inviteGuardian = () => act.run("guardian", async () => {
    const code = await startPairing(chain, address);
    const st = await wait(code, () => setPanel(<div className="c-note" data-guard-link><span><b>Send this link to your guardian</b> (your other device or a trusted person):
      <span className="addr" style={{ display: "block" }}>{linkFor("guard", code)}</span> Waiting for them (10 minutes)…</span><Copy text={linkFor("guard", code)} /></div>));
    setJoined((j) => [...j, { key: st.device_key, check: st.check }]);
    setPanel(null);
    return `Guardian joined. Check code ${st.check} must match their screen.`;
  });

  const save = () => act.run("save", async () => {
    await saveGuardians(chain, passkey!, joined.map((g) => g.key), threshold);
    setJoined([]); q.reload();
    return "Guardians saved. If you ever lose every passkey, they can approve a new one.";
  });

  const s = q.data;
  return (
    <Page title="Account" sub="Who you are on Constella, the devices that can sign, and who can help you recover.">
      <section className="c-card" style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div className="c-section-title"><h3>{passkey ? "Passkey account" : "Stellar wallet"}</h3>
          {passkey ? <Badge kind="live">No seed phrase · fees paid for you</Badge> : <Badge kind="gated">You sign and pay fees in your wallet</Badge>}</div>
        <div className="c-row" style={{ alignItems: "center" }}><span className="addr" data-address>{address}</span><Copy text={address} /></div>
        {passkey && <p className="small muted">This device: {passkey.ruleId === undefined ? "the account's first device (it manages devices and guardians)" : "an added device (it can use the account, not manage it)"}.</p>}
        <div><button className="c-btn c-btn-line c-btn-sm" onClick={signOut} data-act="signout">Sign out of this browser</button></div>
      </section>

      {!view.passkey && <div className="c-note">Your wallet keeps its own keys. Devices and guardian recovery are for passkey accounts.</div>}

      {view.passkey && (
        <div className="c-grid c-grid-2">
          <section className="c-card" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <div className="c-section-title"><h3>Devices</h3></div>
            {s ? (s.devices.length ? <ul className="c-list">{s.devices.map((d) => <li key={d.rule_id}>{d.name}</li>)}</ul>
                               : <p className="small muted">Only this device so far.</p>) : <p className="small muted">…</p>}
            {view.manage && <div><button className="c-btn c-btn-dark c-btn-sm" disabled={!!act.busy} onClick={addDevice} data-act="add-device">Add a device</button></div>}
          </section>
          <section className="c-card" style={{ display: "flex", flexDirection: "column", gap: 12 }} data-guardians>
            <div className="c-section-title"><h3>Guardians</h3>{s?.guardians ? <Badge kind="live">Set</Badge> : <Badge kind="warn">Not set</Badge>}</div>
            {s?.guardians
              ? <p className="small">{s.guardians.guardians.length} guardian{s.guardians.guardians.length > 1 ? "s" : ""}; {s.guardians.threshold} must approve a new passkey.
                  A recovery takes effect only after 48 hours, and you can cancel it meanwhile. Guardians can never move money.</p>
              : <p className="small">If you lose every passkey, guardians (your other devices or people you trust) can approve a new one. Without them, a lost passkey is a lost account.</p>}
            {s?.pending && (
              <div className="c-note c-note-bad" data-pending><span><b>A recovery is pending.</b> A new passkey becomes the owner on {new Date(Number(s.pending.executable_at) * 1000).toLocaleString()} unless you cancel.</span>
                {view.manage && <button className="c-btn c-btn-dark c-btn-sm" disabled={!!act.busy} data-act="cancel-recovery"
                  onClick={() => act.run("cancel", async () => { await cancelRecovery(chain, passkey!); q.reload(); return "Recovery cancelled."; })}>Cancel it</button>}</div>
            )}
            {view.manage && !s?.guardians && (
              <>
                {joined.length > 0 && <p className="small">Ready to save: {joined.map((g) => g.check).join(", ")}</p>}
                <div className="c-row">
                  <button className="c-btn c-btn-line c-btn-sm" disabled={!!act.busy || joined.length >= 5} onClick={inviteGuardian} data-act="invite-guardian">Invite a guardian</button>
                  {joined.length > 0 && <>
                    <label className="small muted">approvals needed{" "}
                      <select className="c-input" style={{ width: 70, display: "inline-block", padding: "6px 8px" }} value={threshold} onChange={(e) => setThreshold(Number(e.target.value))}>
                        {joined.map((_, i) => <option key={i} value={i + 1}>{i + 1}</option>)}</select></label>
                    <button className="c-btn c-btn-dark c-btn-sm" disabled={!!act.busy} onClick={save} data-act="save-guardians">Save guardians</button></>}
                </div>
              </>
            )}
          </section>
        </div>
      )}
      {panel}
      {act.error && <div className="c-note c-note-bad" role="alert">{act.error}</div>}
      {act.done && <div className="c-note c-note-ok" role="status">{act.done}</div>}
      <p className="small muted">This browser remembers which account you use. Every action is signed in your device's own passkey prompt.</p>
    </Page>
  );
}
