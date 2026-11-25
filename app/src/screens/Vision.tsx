/** Step 10.7: the founding-doc vision, honestly gated. Assets (the closed
 * universe every basket is built from) and the Collateral Passport (borrow
 * against a basket token). Live parts read the chain / registry; everything
 * else is visible but disabled with the reason (feature availability list). */
import { xdr } from "@stellar/stellar-sdk";
import { sv } from "../core/chain";
import { formatUnits, listBaskets, readVault } from "../core/vaults";
import { useSession } from "../session";
import { useAsync } from "../ui/hooks";
import { Badge } from "../ui/kit";
import { Page } from "../ui/Shell";
import { userAddress } from "./Portfolio";

const TIER = ["Instant", "Within 24 hours", "Within a week", "Longer"];
const GATED = [
  { name: "Franklin OnChain US Government Money Fund", id: "BENJI", kind: "US T-bill fund", note: "Available when the issuer partnership is live (allow-listed token)." },
  { name: "Ondo US Dollar Yield", id: "USDY", kind: "Yield dollar", note: "Available when the issuer partnership is live (allow-listed token)." },
  { name: "Spiko US T-Bills", id: "USTBL", kind: "US T-bill fund", note: "Available when the issuer partnership is live (allow-listed token)." },
  { name: "Spiko Euro T-Bills", id: "EUTBL", kind: "Euro T-bill fund", note: "Available when the issuer partnership is live (allow-listed token)." },
  { name: "Janus Henderson / Centrifuge Nasdaq-100 tracker", id: "JQQQ", kind: "Tokenized equities, professional investors ($100k minimum)", note: "Gated equities, for institutional treasuries through a licensed partner." },
  { name: "DTCC Tokenization Service securities", id: "DTS", kind: "Tokenized equities", note: "Later phase, once DTC-tokenized securities reach Stellar." },
];

export function Assets() {
  const { cfg } = useSession();
  const q = useAsync(async () => (await (await fetch(cfg.services.thesis_api + "/registry")).json()).assets as any[], []);
  return (
    <Page title="Assets" sub="The closed universe every basket is built from. The engine never picks anything outside this list.">
      <section className="c-card">
        <div className="c-section-title"><h3>Live on testnet</h3><Badge kind="live">Eligible now</Badge></div>
        {q.error && <div className="c-note c-note-bad">Could not load the registry: {q.error}</div>}
        <table className="c-table">
          <thead><tr><th>Asset</th><th className="hide-sm">Issuer</th><th>Type</th><th className="r">Redeemable</th></tr></thead>
          <tbody>{(q.data ?? []).filter((a) => a.is_active).map((a) => (
            <tr key={a.asset_id}><td>{a.display_name}<span className="addr" style={{ display: "block" }}>{a.asset_id}</span></td>
              <td className="hide-sm">{a.issuer}</td><td>{a.sleeve_type} · {a.currency}</td><td className="r">{TIER[a.liquidity_tier] ?? "—"}</td></tr>
          ))}</tbody>
        </table>
      </section>
      <section className="c-card">
        <div className="c-section-title"><h3>Coming with issuer partners</h3><Badge kind="gated">Not available yet</Badge></div>
        <div className="c-grid c-grid-2">
          {GATED.map((g) => (
            <div key={g.id} className="c-card-flat c-gated" aria-disabled="true">
              <div className="c-section-title" style={{ marginBottom: 6 }}><b>{g.name}</b><Badge kind="gated">{g.id}</Badge></div>
              <p className="small muted">{g.kind}. {g.note}</p>
            </div>
          ))}
        </div>
        <p className="small muted" style={{ marginTop: 14 }}>Professional and institutional investor classes and allow-listed (gated) baskets come through a licensed partner.</p>
      </section>
    </Page>
  );
}

export function Passport() {
  const { chain, identity } = useSession();
  const me = userAddress(identity);
  const co = chain.cfg.contracts.collateral_oracle;
  const q = useAsync(async () => {
    const baskets = await listBaskets(chain, me);
    return Promise.all(baskets.map(async (b) => {
      const v = await readVault(chain, b.address);
      const asset = xdr.ScVal.scvVec([sv.sym("Stellar"), sv.addr(b.address)]);
      const [listed, price] = await Promise.all([
        chain.view(co, "is_listed", [sv.addr(b.address)]).catch(() => false),
        chain.view(co, "lastprice", [asset]).catch(() => null),
      ]);
      return { ...b, name: v.name, listed: Boolean(listed), price: price ? BigInt(price.price) : null, nav: v.navConservative };
    }));
  }, [me]);
  return (
    <Page title="Collateral Passport" sub="Your basket token is built to be borrowed against: lenders read one conservative price for it, and stop lending if it cannot be valued.">
      <section className="c-card">
        <div className="c-section-title"><h3>What lenders see for your baskets</h3></div>
        <table className="c-table">
          <thead><tr><th>Basket</th><th>Listed for lenders</th><th className="r">Price served</th><th className="r hide-sm">Basket value / share</th></tr></thead>
          <tbody>{(q.data ?? []).map((b) => (
            <tr key={b.address}><td>{b.name}</td>
              <td>{b.listed ? <Badge kind="live">Listed</Badge> : <Badge kind="gated">Not listed yet</Badge>}</td>
              <td className="r num">{b.price !== null ? formatUnits(b.price, 4) : b.listed ? "none (lending halts)" : "—"}</td>
              <td className="r num hide-sm">{b.nav !== null ? formatUnits(b.nav, 4) : "unpriced"}</td></tr>
          ))}</tbody>
        </table>
        {!q.data && !q.error && <div className="c-skel" style={{ height: 60, marginTop: 10 }} />}
      </section>
      <div className="c-grid c-grid-2">
        <section className="c-card c-gated" aria-disabled="true">
          <div className="c-section-title"><h3>Borrow USDC against a basket</h3><Badge kind="soon">Coming online</Badge></div>
          <p className="small">Supply your basket token to Constella's Blend pool and borrow USDC without selling. The pool is deployed; borrowing
            opens once its backstop is funded. Until then this stays off.</p>
          <button className="c-btn c-btn-dark" disabled>Borrow</button>
        </section>
        <section className="c-card c-gated" aria-disabled="true">
          <div className="c-section-title"><h3>Other lending markets</h3><Badge kind="gated">Partners</Badge></div>
          <p className="small">Any lender can read the same price feed (SEP-40). Listing on third-party markets comes with lending partners.</p>
        </section>
      </div>
    </Page>
  );
}
