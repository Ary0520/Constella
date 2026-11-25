/** Portfolio (step 10.3): every basket the user can hold - the shared Cash
 * Reserve and their own - with their position and live composition. */
import { useSession } from "../session";
import { formatUnits, listBaskets, readPosition, readVault } from "../core/vaults";
import { sv } from "../core/chain";
import { navigate } from "../router";
import { useAsync } from "../ui/hooks";
import { Page } from "../ui/Shell";
import { Badge, Icon } from "../ui/kit";
import { Composition } from "../ui/Composition";

export function userAddress(id: ReturnType<typeof useSession>["identity"]): string {
  return id?.kind === "passkey" ? id.account.address : id?.signer.address ?? "";
}

const usd = (u: bigint) => "$" + formatUnits(u);

export function Portfolio() {
  const { chain, identity, cfg } = useSession();
  const me = userAddress(identity);
  const q = useAsync(async () => {
    const baskets = await listBaskets(chain, me);
    const rows = await Promise.all(baskets.map(async (b) => ({
      ...b, vault: await readVault(chain, b.address), pos: await readPosition(chain, b.address, me),
    })));
    const usdc = BigInt(await chain.view(cfg.assets.USDC, "balance", [sv.addr(me)]).catch(() => 0));
    return { rows, usdc };
  }, [me]);

  const total = q.data?.rows.reduce((n, r) => n + r.pos.value, 0n) ?? 0n;
  const open = (to: string) => (e: React.MouseEvent) => { e.preventDefault(); navigate(to); };
  return (
    <Page title="Portfolio" sub="Your baskets, what they hold, and what your shares are worth right now."
      actions={<button className="c-btn c-btn-dark" onClick={() => navigate("/new")}><Icon name="new" /> New basket</button>}>
      <div className="c-grid c-grid-3">
        <div className="c-card c-stat"><span className="muted small">Your baskets are worth</span><b className="num">{q.data ? usd(total) : "…"}</b></div>
        <div className="c-card c-stat"><span className="muted small">USDC ready to invest</span><b className="num">{q.data ? usd(q.data.usdc) : "…"}</b></div>
        <div className="c-card c-stat"><span className="muted small">Baskets</span><b className="num">{q.data ? q.data.rows.length : "…"}</b></div>
      </div>
      {q.error && (
        <div className="c-note c-note-bad">Could not read the network: {q.error}
          <button className="c-btn c-btn-line c-btn-sm" onClick={q.reload}>Retry</button></div>
      )}
      {!q.data && !q.error && <div className="c-grid c-grid-2"><div className="c-skel" /><div className="c-skel" /></div>}
      {q.data && (
        <div className="c-grid c-grid-2">
          {q.data.rows.map((r) => (
            <a key={r.address} href={`/vault/${r.address}`} className="c-card c-basket" data-basket={r.address} onClick={open(`/vault/${r.address}`)}>
              <div className="row">
                <div><h3>{r.vault.name}</h3><span className="addr">{r.vault.symbol}</span></div>
                {r.shared ? <Badge kind="live">Shared basket</Badge> : <Badge kind="soon">Yours</Badge>}
              </div>
              <div className="row">
                <div className="c-kv"><span>Your position</span><b className="num">{usd(r.pos.value)}</b></div>
                <div className="c-kv"><span>Value per share</span>
                  <b className="num">{r.vault.navConservative !== null ? formatUnits(r.vault.navConservative, 4) : "unpriced"}</b></div>
                <div className="c-kv"><span>Basket size</span><b className="num">{usd(r.vault.totalAssets)}</b></div>
              </div>
              <Composition sleeves={r.vault.sleeves} compact />
              {r.vault.paused && <Badge kind="warn">Paused by its admin</Badge>}
            </a>
          ))}
          <a href="/new" className="c-card-flat c-basket c-build" onClick={open("/new")}>
            <div><h3>Build your own basket</h3>
              <p className="muted small">Write your thesis in plain words. One signature creates it.</p></div>
          </a>
        </div>
      )}
    </Page>
  );
}
