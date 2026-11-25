/** New basket (step 10.4): thesis -> engine draft -> review -> one signature.
 * Spec: the signed policy is always shown and diffed against the thesis; the
 * simulation carries the Rule 10 disclaimer; nothing is auto-submitted. */
import { useState } from "react";
import { createBasket, draftThesis } from "../core/basket";
import { assetName, COLORS } from "../core/labels";
import { navigate } from "../router";
import { useSession } from "../session";
import { useAction } from "../ui/hooks";
import { Badge } from "../ui/kit";
import { Page } from "../ui/Shell";
import { userAddress } from "./Portfolio";

const EXAMPLES = [
  "Keep 18 months of runway safe, hold a third in euro, at least 10% instant cash. Up to 40% per issuer and 40% per asset.",
  "Keep a safe runway in dollars with at least 10% instant cash; lend a little.",
  "Mostly dollars I can pull out within a day, some lending for yield, nothing risky.",
];
const STAGES = ["Reading your thesis", "Finding eligible assets", "Building the basket", "Stress-testing it", "Explaining every choice"];
const pct = (bps: number) => (bps / 100).toFixed(1) + "%";
const hours = (h: number) => (h >= 48 ? `${Math.round(h / 24)} days` : h >= 24 ? "1 day" : h >= 1 ? `${Math.round(h)} hours` : "under an hour");

function Weights({ w, why }: { w: Record<string, number>; why?: Record<string, string> }) {
  const rows = Object.entries(w).filter(([, x]) => x > 0);
  return (
    <div className="c-comp">
      <div className="c-bar">{rows.map(([a, x], i) => <span key={a} style={{ width: `${x / 100}%`, background: COLORS[i % COLORS.length] }} />)}</div>
      <ul className="c-legend">
        {rows.map(([a, x], i) => (
          <li key={a} style={{ alignItems: "flex-start" }}><i style={{ background: COLORS[i % COLORS.length], marginTop: 6 }} />
            <span>{assetName(a)}{why?.[a] && <span className="muted small" style={{ display: "block" }}>{why[a]}</span>}</span>
            <b className="num">{pct(x)}</b></li>
        ))}
      </ul>
    </div>
  );
}

function Simulation({ sim }: { sim: any }) {
  const l = sim.redemption_ladder, m = sim.redeemable_during_freeze;
  const p = (x: number) => (x * 100).toFixed(1) + "%";
  return (
    <section className="c-card">
      <div className="c-section-title"><h3>Simulation, before any money moves</h3><Badge kind="warn">Simulation</Badge></div>
      <div className="c-note" style={{ marginBottom: 14 }}><span><b>Worst case: a full exit could take {hours(sim.worst_case_exit_hours)}</b> ({sim.worst_case_scenario}).
        Normal full exit: {hours(sim.normal_exit_hours)}.</span></div>
      <table className="c-table">
        <tbody>
          <tr><td>Issuer freeze</td><td>{sim.issuer_freeze_stress.description}</td></tr>
          <tr><td>Depeg</td><td>{sim.depeg_stress.description}</td></tr>
          <tr><td>Redeemable</td><td className="num">instant {p(l.instant_pct)} · within 24 h {p(l.within_24h_pct)} · within a week {p(l.within_1w_pct)} · longer {p(l.illiquid_pct)}
            {m && <span className="muted"> (during the freeze: instant {p(m.instant_pct)}, 24 h {p(m.within_24h_pct)})</span>}</td></tr>
          <tr><td>Estimated yield</td><td className="num">{sim.weighted_yield_pct.toFixed(2)}%/yr <span className="muted">(an unknown rate counts as 0)</span></td></tr>
          <tr><td>History</td><td>{sim.history_note}</td></tr>
        </tbody>
      </table>
      <p className="small" style={{ marginTop: 14 }} data-disclaimer><i>{sim.disclaimer}</i></p>
    </section>
  );
}

export function NewBasket() {
  const { cfg, chain, identity, actor } = useSession();
  const me = userAddress(identity);
  const [text, setText] = useState("");
  const [jur, setJur] = useState("DE");
  const [d, setD] = useState<any>(null);
  const [refused, setRefused] = useState<any>(null);
  const [ok, setOk] = useState(false);
  const draft = useAction();
  const make = useAction();

  const run = () => draft.run("draft", async () => {
    setD(null); setRefused(null); setOk(false);
    try { setD(await draftThesis(cfg, text, me, { jurisdiction: jur })); }
    catch (e: any) {
      // An infeasible thesis comes back with reasons and the nearest basket.
      if (e?.body?.reasons) setRefused(e.body); else throw e;
    }
  });

  return (
    <Page title="New basket" sub="Say what the money is for. The engine drafts a basket of eligible assets and the rules that will bind it. You review; nothing happens until you sign.">
      <section className="c-card" style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        <div className="c-field"><label htmlFor="thesis">Your thesis</label>
          <textarea id="thesis" className="c-input" placeholder="e.g. Keep 18 months of runway safe, hold a third in euro, at least 10% instant cash."
            value={text} onChange={(e) => setText(e.target.value)} /></div>
        <div className="c-row" style={{ gap: 8 }}>
          {EXAMPLES.map((x) => <button key={x} className="c-btn c-btn-soft c-btn-sm" style={{ whiteSpace: "normal", textAlign: "left" }} onClick={() => setText(x)}>{x.split(/[,;.]/)[0]}…</button>)}
        </div>
        <div className="c-row">
          <div className="c-field" style={{ maxWidth: 220 }}><label htmlFor="jur">Where you are based</label>
            <select id="jur" className="c-input" value={jur} onChange={(e) => setJur(e.target.value)}>
              {[["DE", "Germany"], ["FR", "France"], ["NL", "Netherlands"], ["ES", "Spain"], ["IT", "Italy"], ["GB", "United Kingdom"], ["SG", "Singapore"], ["AE", "UAE"], ["IN", "India"], ["BR", "Brazil"]]
                .map(([c, n]) => <option key={c} value={c}>{n}</option>)}
            </select></div>
          <div className="c-field" style={{ maxWidth: 260 }}><label>Investor type</label>
            <div className="c-row" style={{ gap: 6 }}><Badge kind="live">Retail</Badge><Badge kind="gated">Professional · via licensed partner</Badge></div></div>
          <button className="c-btn c-btn-dark" disabled={!!draft.busy || text.trim().length < 8} onClick={run} data-act="draft">
            {draft.busy ? "Drafting…" : "Draft my basket"}</button>
        </div>
        {draft.busy && <ol className="c-stages">{STAGES.map((s) => <li key={s}>{s}</li>)}</ol>}
        {draft.error && <div className="c-note c-note-bad" role="alert">{draft.error}</div>}
      </section>

      {refused && (
        <section className="c-card" data-refused>
          <div className="c-section-title"><h3>Your thesis can't be met as written</h3><Badge kind="warn">Not feasible</Badge></div>
          <ul className="c-list">{(refused.reasons as string[]).map((r) => <li key={r}>{r}</li>)}</ul>
          {refused.relaxed?.length > 0 && <><p className="small muted" style={{ marginTop: 12 }}>To get closest, the engine had to relax:</p>
            <ul className="c-list">{(refused.relaxed as string[]).map((r) => <li key={r}>{r}</li>)}</ul></>}
          {refused.nearest && <><p className="small muted" style={{ margin: "14px 0 8px" }}>Nearest basket that fits the rules (not signable as is):</p><Weights w={refused.nearest.weights_bps} /></>}
          <p className="small" style={{ marginTop: 12 }}>Adjust the thesis (for example allow a higher share per asset or issuer) and draft again.</p>
        </section>
      )}

      {d && (
        <>
          <div className="c-grid c-grid-2">
            <section className="c-card">
              <div className="c-section-title"><h3>{d.policy?.basket_name ?? "Proposed basket"}</h3>
                <span className="muted small">vault currency {d.allocation.numeraire} · parsed by {d.parse_source === "rules" ? "rules" : "AI + rules"}</span></div>
              <Weights w={d.allocation.weights_bps} why={d.explanation.asset_rationales} />
              {d.ai_summary?.summary && <p className="small" style={{ marginTop: 14 }}>{d.ai_summary.summary}</p>}
              {d.ai_note && <p className="small muted" style={{ marginTop: 8 }}>{d.ai_note}</p>}
            </section>
            <section className="c-card">
              <div className="c-section-title"><h3>Your thesis vs this basket</h3></div>
              <table className="c-table"><thead><tr><th>You asked</th><th>Proposed</th></tr></thead>
                <tbody>{(d.explanation.thesis_diff as any[]).map((x, i) => (
                  <tr key={i}><td>{x.requested}</td><td>{x.proposed}<span className="muted small" style={{ display: "block" }}>{x.explanation}</span></td></tr>
                ))}</tbody></table>
            </section>
          </div>
          {(d.explanation.issuer_concentration_warnings.length > 0 || d.explanation.ambiguity_clarifications.length > 0) && (
            <section className="c-card">
              {d.explanation.issuer_concentration_warnings.length > 0 && <><h3>Warnings</h3>
                <ul className="c-list">{d.explanation.issuer_concentration_warnings.map((w: string) => <li key={w}>{w}</li>)}</ul></>}
              {d.explanation.ambiguity_clarifications.length > 0 && <><h3 style={{ marginTop: 12 }}>Needs your confirmation</h3>
                <ul className="c-list">{d.explanation.ambiguity_clarifications.map((w: string) => <li key={w}>{w}</li>)}</ul></>}
            </section>
          )}
          <Simulation sim={d.simulation} />
          <section className="c-card" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <div className="c-section-title"><h3>The rules you will sign</h3><Badge kind="live">Enforced on chain</Badge></div>
            <table className="c-table"><tbody>
              <tr><td>Instant cash at least</td><td className="r num">{pct(d.bind_policy.min_liquidity_sleeve_bps)}</td></tr>
              <tr><td>Any single asset at most</td><td className="r num">{pct(d.bind_policy.max_single_asset_bps)}</td></tr>
              <tr><td>Any single issuer at most</td><td className="r num">{pct(d.bind_policy.max_issuer_bps)}</td></tr>
              <tr><td>Rebalance when a holding drifts by</td><td className="r num">{pct(d.bind_policy.rebalance_band_bps)}</td></tr>
              <tr><td>Maximum slippage per trade</td><td className="r num">{pct(d.bind_policy.max_slippage_bps)}</td></tr>
            </tbody></table>
            {d.signable ? <p className="addr">Policy hash {d.policy_hash}</p> : <p className="muted small">Preview only.</p>}
            <details><summary className="small muted">Exact on-chain terms</summary><pre className="addr" style={{ whiteSpace: "pre-wrap" }}>{JSON.stringify(d.bind_policy, null, 1)}</pre></details>
            {d.ineligible && Object.keys(d.ineligible).length > 0 && (
              <details><summary className="small muted">Not available to you, and why</summary><pre className="addr" style={{ whiteSpace: "pre-wrap" }}>{JSON.stringify(d.ineligible, null, 1)}</pre></details>
            )}
            <label className="c-check"><input type="checkbox" checked={ok} onChange={(e) => setOk(e.target.checked)} data-act="confirm" />
              I reviewed the basket, the simulation and these rules. Create my basket with them.</label>
            <p className="muted small">One signature: deploys your private basket, binds exactly these rules with you as owner, and sets Constella's keeper.
              Fee: 10% of profit, never more than 0.20% a year. Testnet limit: 25,000 USDC.</p>
            <div><button className="c-btn c-btn-dark" disabled={!ok || !d.signable || !!make.busy} data-act="create"
              onClick={() => make.run("create", async () => { const v = await createBasket(chain, actor(), d); navigate(`/vault/${v}`); })}>
              {make.busy ? "Confirm with your passkey…" : "Create my basket"}</button></div>
            {make.error && <div className="c-note c-note-bad" role="alert">{make.error}</div>}
          </section>
        </>
      )}
    </Page>
  );
}
