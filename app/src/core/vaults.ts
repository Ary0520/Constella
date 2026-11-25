/**
 * Vault reads and the numbers the app shows (step 10.3). Valuation mirrors the
 * vault and the keeper planner exactly: the cash sleeve (the vault currency)
 * at face value, any other sleeve at units x oracle `asset_price` / 10^7,
 * rounded down. Nothing is estimated: an unpriced holding is shown as such.
 */
import { Chain, sv } from "./chain";

export const DECIMALS = 7;
const ONE = 10n ** BigInt(DECIMALS);

export function formatUnits(u: bigint | number | string, dp = 2): string {
  let n = BigInt(u);
  const neg = n < 0n;
  if (neg) n = -n;
  const step = 10n ** BigInt(DECIMALS - dp);
  let rounded = (n + step / 2n) / step;
  if (rounded === 0n && n > 0n) { // too small for dp: show it exactly
    const frac = (n % ONE).toString().padStart(DECIMALS, "0").replace(/0+$/, "");
    return `${neg ? "-" : ""}${n / ONE}.${frac}`;
  }
  const scale = 10n ** BigInt(dp);
  const whole = (rounded / scale).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const frac = dp > 0 ? "." + (rounded % scale).toString().padStart(dp, "0") : "";
  return `${neg ? "-" : ""}${whole}${frac}`;
}

/** Largest-remainder rounding to exactly 10000 (all zeros if nothing). */
export function toBps(values: bigint[]): number[] {
  const total = values.reduce((a, b) => a + b, 0n);
  if (total <= 0n) return values.map(() => 0);
  const raw = values.map((v) => v * 10000n);
  const out = raw.map((r) => Number(r / total));
  let rest = 10000 - out.reduce((a, b) => a + b, 0);
  const order = raw.map((r, i) => [r % total, i] as const).sort((a, b) => (b[0] > a[0] ? 1 : b[0] < a[0] ? -1 : a[1] - b[1]));
  for (const [, i] of order) { if (rest-- <= 0) break; out[i] += 1; }
  return out;
}

export interface Sleeve { asset_id: string; target_weight_bps: number; current_balance: bigint; contract_address: string; sleeve_type: number; band_bps: number }
export interface SleeveView extends Sleeve { value: bigint | null; weight_bps: number | null; priced: boolean; is_cash: boolean }

export function sleeveView(sleeves: Sleeve[], prices: Record<string, bigint | undefined>, cash: string): SleeveView[] {
  const rows = sleeves.map((s) => {
    const is_cash = s.contract_address === cash;
    const held = BigInt(s.current_balance);
    const p = prices[s.contract_address];
    const priced = is_cash || held === 0n || (p !== undefined && p > 0n);
    const value = is_cash ? held : held === 0n ? 0n : priced ? (held * (p as bigint)) / ONE : null;
    return { ...s, current_balance: held, is_cash, priced, value, weight_bps: null as number | null };
  });
  if (rows.every((r) => r.value !== null)) {
    const w = toBps(rows.map((r) => r.value as bigint));
    rows.forEach((r, i) => (r.weight_bps = w[i]));
  }
  return rows;
}

export interface NavPoint { nav: bigint | number; timestamp: bigint | number }

/** Annualised NAV change between the first and last on-chain points (%/yr). */
export function realisedApr(history: NavPoint[]): number | null {
  if (history.length < 2) return null;
  const a = history[0], b = history[history.length - 1];
  const dt = Number(BigInt(b.timestamp) - BigInt(a.timestamp));
  if (dt < 86_400 || Number(a.nav) <= 0) return null;
  return (Number(b.nav) / Number(a.nav) - 1) * (31_536_000 / dt) * 100;
}

// ------------------------------------------------------------------ reads
export interface Basket { address: string; shared: boolean }

/** The shared Cash Reserve plus the user's own vaults (factory). */
export async function listBaskets(chain: Chain, user: string | null): Promise<Basket[]> {
  const c = chain.cfg.contracts;
  const own: string[] = user && c.vault_factory
    ? (await chain.view(c.vault_factory, "vaults_of", [sv.addr(user)]).catch(() => [])).map(String) : [];
  return [{ address: c.vault, shared: true }, ...own.filter((v) => v !== c.vault).map((address) => ({ address, shared: false }))];
}

export interface VaultData {
  address: string; name: string; symbol: string; paused: boolean;
  navConservative: bigint | null; navDeposit: bigint | null; totalAssets: bigint; totalSupply: bigint; highWaterMark: bigint;
  sleeves: SleeveView[]; apr: number | null; cash: string;
  feeBps: number; admin: string; policy: string; policyHash: string | null;
}

export async function readVault(chain: Chain, address: string): Promise<VaultData> {
  const v = (fn: string) => chain.view(address, fn);
  const [name, symbol, paused, cons, dep, total, supply, hwm, sleeves, history, config, hash] = await Promise.all([
    v("name"), v("symbol"), v("is_paused"), v("get_nav_conservative"), v("get_nav_deposit"), v("total_assets"),
    v("total_supply"), v("get_high_water_mark"), v("get_sleeves"), v("get_nav_history").catch(() => []), v("get_config"),
    v("get_policy_hash").catch(() => null),
  ]);
  const cash = String(config.numeraire);
  const oracle = String(config.oracle_contract);
  const prices: Record<string, bigint> = {};
  await Promise.all((sleeves as any[]).map(async (s) => {
    const a = String(s.contract_address);
    if (a === cash) return;
    const p = await chain.view(oracle, "asset_price", [sv.addr(a)]).catch(() => null);
    if (p) prices[a] = BigInt(p.price);
  }));
  const rows: Sleeve[] = (sleeves as any[]).map((s) => ({
    asset_id: String(s.asset_id), target_weight_bps: Number(s.target_weight_bps), current_balance: BigInt(s.current_balance),
    contract_address: String(s.contract_address), sleeve_type: Number(s.sleeve_type), band_bps: Number(s.band_bps),
  }));
  return {
    address, name: String(name), symbol: String(symbol), paused: Boolean(paused),
    navConservative: cons ? BigInt(cons.price) : null, navDeposit: dep ? BigInt(dep.price) : null,
    totalAssets: BigInt(total), totalSupply: BigInt(supply), highWaterMark: BigInt(hwm),
    sleeves: sleeveView(rows, prices, cash), apr: realisedApr(history as NavPoint[]), cash,
    feeBps: Number(config.performance_fee_bps), admin: String(config.admin), policy: String(config.policy_contract),
    policyHash: hash ? Array.from(hash as Uint8Array, (b: number) => b.toString(16).padStart(2, "0")).join("") : null,
  };
}

export interface Position { shares: bigint; value: bigint }

export async function readPosition(chain: Chain, vault: string, user: string): Promise<Position> {
  const shares = BigInt(await chain.view(vault, "balance", [sv.addr(user)]).catch(() => 0));
  const value = shares > 0n ? BigInt(await chain.view(vault, "convert_to_assets", [sv.i128(shares)])) : 0n;
  return { shares, value };
}

/** What a person typed -> exact units. Positive, at most 7 decimals, commas allowed. */
export function parseUnits(text: string): bigint {
  const s = text.trim().replace(/,/g, "");
  const m = /^(\d*)(?:\.(\d*))?$/.exec(s);
  if (!s || !m || (m[1] === "" && !m[2])) throw new Error("Enter an amount, like 100 or 12.5.");
  const frac = m[2] ?? "";
  if (frac.length > DECIMALS) throw new Error(`At most ${DECIMALS} decimal places.`);
  const units = BigInt(m[1] || "0") * ONE + BigInt((frac + "0".repeat(DECIMALS)).slice(0, DECIMALS));
  if (units <= 0n) throw new Error("Enter an amount above zero.");
  return units;
}
