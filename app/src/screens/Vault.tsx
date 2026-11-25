/** One basket (steps 10.3 + 10.5): what it holds against its signed rules,
 * your position, deposit / redeem, test USDC, open claims, policy. */
import { useState } from "react";
import { sv } from "../core/chain";
import { assetName } from "../core/labels";
import { formatUnits, parseUnits, readActivity, readPosition, readVault } from "../core/vaults";
import { addUsdcTrustline, friendbotFund, walletStatus } from "../core/wallet";
import { navigate } from "../router";
import { useSession } from "../session";
import { useAction, useAsync } from "../ui/hooks";
import { Badge } from "../ui/kit";
import { Page } from "../ui/Shell";
import { Composition } from "../ui/Composition";
import { userAddress } from "./Portfolio";

const usd = (u: bigint) => "$" + formatUnits(u);
const pct = (bps: number) => (bps / 100).toFixed(1) + "%";

/** Vault contract errors (VaultError codes) and service errors, in plain words. */
const VAULT_ERRORS: Record<string, string> = {
  "4": "This basket is paused by its admin, so it cannot take this action right now.",
  "8": "The basket does not have enough cash to pay this out right now. Redeem less, or wait for the keeper to raise cash.",
  "10": "A price is unavailable right now, so a normal redemption cannot value your shares. Use \"Redeem while a price is unavailable\" below.",
  "18": "This basket is full (its total limit).",
  "19": "That is above this basket's per-person limit.",
};
export function friendlyChainError(m: string): string {
  const code = /Error\(Contract, #(\d+)\)/.exec(m)?.[1];
  if (code && VAULT_ERRORS[code]) return VAULT_ERRORS[code];
  if (/RATE_LIMITED|once a day/i.test(m)) return "The test-USDC faucet gives each account one top-up a day. Try again tomorrow.";
  if (/faucet_refused|trustline/i.test(m)) return "This wallet needs to accept USDC first (button above).";
  if (/NotAllowedError|timed out/i.test(m)) return "The signature prompt was closed. Nothing was sent.";
  return m;
}

export function Vault({ id }: { id: string }) {
  const { chain, cfg, identity, actor } = useSession();
  const me = userAddress(identity);
  const q = useAsync(async () => {
    const [vault, pos, usdc, claim, wallet] = await Promise.all([
      readVault(chain, id), readPosition(chain, id, me),
      chain.view(cfg.assets.USDC, "balance", [sv.addr(me)]).catch(() => 0),
      chain.view(id, "get_claim", [sv.addr(me), sv.addr(cfg.assets.EURC)]).catch(() => 0),
      identity?.kind === "wallet" ? walletStatus(chain, me) : Promise.resolve(null),
    ]);
    return { vault, pos, usdc: BigInt(usdc), claim: BigInt(claim), wallet };
  }, [id, me]);
  const act = useAction();
  const activity = useAsync(() => readActivity(chain, id), [id]);
  const [frozenPreview, setFrozenPreview] = useState<{ cash: bigint; claims: [string, bigint][] } | null>(null);
  const [dep, setDep] = useState("");
  const [red, setRed] = useState("");

  const run = (label: string, f: () => Promise<string>) => act.run(label, async () => {
    try { const m = await f(); q.reload(); return m; } catch (e: any) { throw new Error(friendlyChainError(String(e?.message ?? e))); }
  });
  const user = (n: bigint) => [sv.i128(n), sv.addr(me), sv.addr(me), sv.addr(me)];

  const d = q.data;
  const v = d?.vault;
  return (
    <Page title={v?.name ?? "Basket"} sub={<span className="addr">{id}</span>}
      actions={<button className="c-btn c-btn-line c-btn-sm" onClick={() => navigate("/")}>All baskets</button>}>
      {q.error && <div className="c-note c-note-bad">Could not read this basket: {q.error}</div>}
      {!d && !q.error && <div className="c-skel" />}
      {d && v && (
        <>
          <div className="c-grid c-grid-3">
            <div className="c-card c-stat"><span className="muted small">Value per share (conservative)</span>
              <b className="num">{v.navConservative !== null ? formatUnits(v.navConservative, 4) : "unpriced"}</b></div>
            <div className="c-card c-stat"><span className="muted small">Basket size</span><b className="num">{usd(v.totalAssets)}</b></div>
            <div className="c-card c-stat"><span className="muted small">Realised return (on-chain history)</span>
              <b className="num">{v.apr !== null ? `${v.apr.toFixed(2)}%/yr` : "Not enough history"}</b></div>
          </div>
          {v.paused && <div className="c-note"><Badge kind="warn">Paused</Badge> Its admin paused this basket.</div>}

          <div className="c-grid c-grid-2">
            <section className="c-card">
              <div className="c-section-title"><h3>Your position</h3>{identity?.kind === "passkey" ? <Badge kind="live">No fees to sign</Badge> : <Badge kind="gated">You pay network fees</Badge>}</div>
              <div className="c-kvs">
                <div className="c-kv"><span>Worth</span><b className="num" data-pos>{usd(d.pos.value)}</b></div>
                <div className="c-kv"><span>Shares</span><b className="num">{formatUnits(d.pos.shares, 4)}</b></div>
                <div className="c-kv"><span>Your USDC</span><b className="num">{usd(d.usdc)}</b></div>
              </div>
              {d.wallet && !d.wallet.exists && (
                <div className="c-note" style={{ marginBottom: 12 }}>This wallet account is not on testnet yet.
                  <button className="c-btn c-btn-dark c-btn-sm" disabled={!!act.busy}
                    onClick={() => run("fund", async () => { await friendbotFund(chain, me); return "Funded with test XLM."; })}>Fund it</button></div>
              )}
              {d.wallet?.exists && !d.wallet.hasTrustline && (
                <div className="c-note" style={{ marginBottom: 12 }}>Your wallet needs to accept USDC first.
                  <button className="c-btn c-btn-dark c-btn-sm" disabled={!!act.busy}
                    onClick={() => run("trust", async () => {
                      if (identity?.kind !== "wallet") throw new Error("Connect your wallet again.");
                      await addUsdcTrustline(chain, identity.signer); return "USDC enabled.";
                    })}>Enable USDC</button></div>
              )}
              <div className="c-row">
                <div className="c-field"><label htmlFor="dep">Deposit USDC</label>
                  <input id="dep" className="c-input num" inputMode="decimal" placeholder="100" value={dep} onChange={(e) => setDep(e.target.value)} /></div>
                <button className="c-btn c-btn-dark" disabled={!!act.busy || v.paused} data-act="deposit"
                  onClick={() => run("deposit", async () => {
                    const n = parseUnits(dep);
                    if (n > d.usdc) throw new Error(`You have ${usd(d.usdc)} USDC. Get test USDC below or deposit less.`);
                    await actor().call(id, "deposit", user(n)); setDep(""); return `Deposited ${usd(n)}.`;
                  })}>{act.busy === "deposit" ? "Confirm with your passkey…" : "Deposit"}</button>
              </div>
              <div className="c-row" style={{ marginTop: 12 }}>
                <div className="c-field"><label htmlFor="red">Redeem shares</label>
                  <input id="red" className="c-input num" inputMode="decimal" placeholder="10" value={red} onChange={(e) => setRed(e.target.value)} /></div>
                <button className="c-btn c-btn-line c-btn-sm" data-act="redeem-all" disabled={d.pos.shares === 0n} onClick={() => setRed(formatUnits(d.pos.shares, 7).replace(/,/g, ""))}>All</button>
                <button className="c-btn c-btn-line" disabled={!!act.busy} data-act="redeem"
                  onClick={() => run("redeem", async () => {
                    const n = parseUnits(red);
                    if (n > d.pos.shares) throw new Error(`You hold ${formatUnits(d.pos.shares, 4)} shares.`);
                    await actor().call(id, "redeem", user(n)); setRed(""); return "Redeemed. USDC is back in your account.";
                  })}>{act.busy === "redeem" ? "Confirm…" : "Redeem"}</button>
              </div>
              <div style={{ marginTop: 16 }}>
                <button className="c-btn c-btn-soft c-btn-sm" disabled={!!act.busy} data-act="faucet"
                  onClick={() => run("faucet", async () => { await chain.relayer("/faucet", { address: me }); return "100 test USDC is on its way."; })}>
                  Get 100 test USDC (once a day)</button>
              </div>
              {act.error && <div className="c-note c-note-bad" role="alert" style={{ marginTop: 12 }}>{act.error}</div>}
              {act.done && <div className="c-note c-note-ok" role="status" style={{ marginTop: 12 }}>{act.done}</div>}
              {v.sleeves.some((s) => !s.priced) && d.pos.shares > 0n && (
                <div className="c-note" style={{ marginTop: 12, flexDirection: "column" }} data-frozen>
                  <span>A holding has no valid price right now. You can still redeem: you get its cash part now and a claim for the rest, which the keeper pays in USDC once the price is back.</span>
                  {frozenPreview && <span className="small">For {red || "these"} shares: {usd(frozenPreview.cash)} now
                    {frozenPreview.claims.map(([a, n]) => ` + a claim of ${formatUnits(n)} units of ${a === cfg.assets.EURC ? "EURC" : a.slice(0, 6)}`).join("")}.</span>}
                  <div className="c-row">
                    <button className="c-btn c-btn-line c-btn-sm" disabled={!!act.busy} onClick={() => act.run("preview", async () => {
                      const p = await chain.view(id, "preview_redeem_frozen", [sv.i128(parseUnits(red))]);
                      setFrozenPreview({ cash: BigInt(p.cash), claims: (p.claims as any[]).map(([a, n]: any) => [String(a), BigInt(n)]) });
                    })}>Preview</button>
                    <button className="c-btn c-btn-dark c-btn-sm" disabled={!!act.busy} data-act="redeem-frozen" onClick={() => run("frozen", async () => {
                      const n = parseUnits(red);
                      if (n > d.pos.shares) throw new Error(`You hold ${formatUnits(d.pos.shares, 4)} shares.`);
                      await actor().call(id, "redeem_frozen", user(n)); setRed(""); setFrozenPreview(null);
                      return "Redeemed: cash is in your account; the rest is a claim the keeper pays once the price is back.";
                    })}>Redeem while a price is unavailable</button>
                  </div>
                </div>
              )}
              {d.claim > 0n && (
                <div className="c-note" style={{ marginTop: 12 }}>You have an open EURC claim of {formatUnits(d.claim)} units. It was set aside
                  while the euro price was unavailable; the keeper pays it in USDC once EURC is priced again.</div>
              )}
            </section>
            <section className="c-card">
              <div className="c-section-title"><h3>What it holds</h3></div>
              <Composition sleeves={v.sleeves} />
            </section>
          </div>

          <section className="c-card">
            <div className="c-section-title"><h3>Against its signed rules</h3><Badge kind="live">Enforced on chain</Badge></div>
            <table className="c-table">
              <thead><tr><th>Asset</th><th className="r">Target</th><th className="r hide-sm">Allowed band</th><th className="r">Now</th><th className="r hide-sm">Value</th></tr></thead>
              <tbody>
                {v.sleeves.map((s) => {
                  const out = s.weight_bps !== null && Math.abs(s.weight_bps - s.target_weight_bps) > s.band_bps && v.totalAssets > 0n;
                  return (
                    <tr key={s.contract_address}>
                      <td>{assetName(s.asset_id)}</td>
                      <td className="r num">{pct(s.target_weight_bps)}</td>
                      <td className="r num hide-sm">{pct(Math.max(0, s.target_weight_bps - s.band_bps))} – {pct(Math.min(10000, s.target_weight_bps + s.band_bps))}</td>
                      <td className="r num">{s.weight_bps === null ? <Badge kind="warn">unpriced</Badge> : <>{pct(s.weight_bps)} {out && <Badge kind="warn">rebalancing</Badge>}</>}</td>
                      <td className="r num hide-sm">{s.value === null ? "—" : usd(s.value)}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <p className="muted small" style={{ marginTop: 14 }}>
              The keeper rebalances only when a holding leaves its band, and only the difference (deposits and withdrawals net out first).
              Fee: {v.feeBps / 100}% of profit above the high-water mark, never more than 0.20% a year; nothing when there is no profit.
            </p>
            {v.policyHash && <p className="addr" style={{ marginTop: 8 }}>Policy hash {v.policyHash}</p>}
          </section>
          <section className="c-card">
            <div className="c-section-title"><h3>Recent activity</h3><span className="muted small">last week, from the network</span></div>
            {activity.error && <p className="small muted">Activity is unavailable right now.</p>}
            {!activity.data && !activity.error && <p className="small muted">Loading…</p>}
            {activity.data?.length === 0 && <p className="small muted">Nothing in the last week.</p>}
            {activity.data && activity.data.length > 0 && (
              <table className="c-table" data-activity><tbody>{activity.data.map((a) => (
                <tr key={a.tx + a.label}><td>{a.label}</td><td className="r muted small">{new Date(a.at).toLocaleString()}</td>
                  <td className="r hide-sm"><a className="addr" href={`https://stellar.expert/explorer/testnet/tx/${a.tx}`} target="_blank" rel="noreferrer">{a.tx.slice(0, 10)}…</a></td></tr>
              ))}</tbody></table>
            )}
          </section>
        </>
      )}
    </Page>
  );
}
