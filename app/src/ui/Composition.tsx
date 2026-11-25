import type { SleeveView } from "../core/vaults";
import { assetName, COLORS } from "../core/labels";

/** Stacked bar of current weights (or targets when nothing is held yet). */
export function Composition({ sleeves, compact = false }: { sleeves: SleeveView[]; compact?: boolean }) {
  const held = sleeves.some((s) => (s.weight_bps ?? 0) > 0);
  const w = (s: SleeveView) => (held ? s.weight_bps ?? 0 : s.target_weight_bps);
  return (
    <div className="c-comp">
      <div className="c-bar" role="img" aria-label={sleeves.map((s) => `${assetName(s.asset_id)} ${(w(s) / 100).toFixed(1)}%`).join(", ")}>
        {sleeves.map((s, i) => <span key={s.contract_address} style={{ width: `${w(s) / 100}%`, background: COLORS[i % COLORS.length] }} />)}
      </div>
      {!compact && (
        <ul className="c-legend">
          {sleeves.map((s, i) => (
            <li key={s.contract_address}><i style={{ background: COLORS[i % COLORS.length] }} />{assetName(s.asset_id)}
              <b className="num">{(w(s) / 100).toFixed(1)}%</b></li>
          ))}
        </ul>
      )}
      {!held && <p className="muted small">Nothing held yet: showing the signed targets.</p>}
    </div>
  );
}
