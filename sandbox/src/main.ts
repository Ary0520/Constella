/**
 * Constella sandbox - THROWAWAY feature console (local network only).
 * Plain DOM, raw numbers: a place to click through every implemented flow,
 * not a design. Delete sandbox/ to remove it.
 */
import { xdr } from "@stellar/stellar-sdk";
import { Chain, Config, Passkey, sv, thesis } from "./chain";
import { createPasskey } from "./passkey";
import { addUsdcTrustline, connectWallet, friendbotFund, WalletSigner, walletInvoke, walletStatus } from "./wallet";

const ONE = 10_000_000n;
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const fmt = (units: bigint | number | string, dec = 7) => {
  const n = BigInt(units);
  const neg = n < 0n;
  const a = neg ? -n : n;
  const s = (a / 10n ** BigInt(dec)).toString() + "." + (a % 10n ** BigInt(dec)).toString().padStart(dec, "0").slice(0, 4);
  return (neg ? "-" : "") + s;
};
const toUnits = (s: string) => BigInt(Math.round(Number(s) * 1e7));
const short = (a: string) => `${a.slice(0, 6)}...${a.slice(-4)}`;

function log(msg: string, data?: unknown) {
  const line = `[${new Date().toLocaleTimeString()}] ${msg}${data === undefined ? "" : " " + JSON.stringify(data, (_, v) => (typeof v === "bigint" ? v.toString() : v))}`;
  $("log").textContent = line + "\n" + $("log").textContent;
}

// Visible status bar: every action's progress and result (the activity log
// at the bottom was too easy to miss - first real-browser test).
function status(text: string, kind: "busy" | "ok" | "err") {
  const el = $("status");
  el.textContent = text;
  el.className = kind;
}

async function act(label: string, fn: () => Promise<unknown>) {
  log(`${label}...`);
  status(mode() === "wallet"
    ? `${label}... (approve it in your wallet; you pay the XLM network fee)`
    : `${label}... (a passkey prompt may appear; the relayer then needs a few seconds)`, "busy");
  try {
    const out = await fn();
    log(`${label}: OK`, out);
    // Refresh BEFORE reporting done; the RPC may still serve the previous
    // ledger for a moment after confirmation, so refresh once more shortly.
    await refresh();
    status(`${label}: done`, "ok");
    setTimeout(() => void refresh(), 3_000);
  } catch (e: any) {
    const msg = e?.message ?? String(e);
    log(`${label}: FAILED - ${msg}`);
    status(`${label} failed: ${msg}`, "err");
  }
}

// ---------------------------------------------------------------- state
let cfg: Config;
let chain: Chain;
let wallet: WalletSigner | null = null; // in memory: reconnect after a reload
const mode = (): "passkey" | "wallet" => ((document.querySelector('input[name="who"]:checked') as HTMLInputElement)?.value as any) ?? "passkey";

/** Who acts: the passkey account (relayer pays) or the wallet (user pays). */
function actor(): { address: string; call: (fn: string, args: xdr.ScVal[]) => Promise<unknown> } {
  if (mode() === "wallet") {
    if (!wallet) throw new Error("connect a Stellar wallet first (section 2b)");
    const w = wallet;
    return { address: w.address, call: (fn, args) => walletInvoke(chain, w, cfg.contracts.vault, fn, args) };
  }
  const a = need();
  return { address: a.address, call: (fn, args) => chain.sponsored(a, cfg.contracts.vault, fn, args) };
}
const userArgs = (amount: bigint, who: string) => [sv.i128(amount), sv.addr(who), sv.addr(who), sv.addr(who)];
const ACCOUNT_KEY = "constella-sandbox-account";

function loadAccount(): Passkey | null {
  try {
    const raw = localStorage.getItem(ACCOUNT_KEY);
    if (!raw) return null;
    const a = JSON.parse(raw);
    return { ...a, publicKey: Uint8Array.from(a.publicKey) };
  } catch { return null; }
}
function saveAccount(a: Passkey) {
  try { localStorage.setItem(ACCOUNT_KEY, JSON.stringify({ ...a, publicKey: Array.from(a.publicKey) })); } catch { /* private mode */ }
}

// ---------------------------------------------------------------- render
function layout() {
  const c = cfg.contracts;
  $("app").innerHTML = `
  <section><h2>1. Vault overview <button id="refresh">Refresh</button></h2>
    <div class="mono">vault ${c.vault} · policy ${c.policy} · oracle ${c.oracle}</div>
    <div id="overview">...</div></section>

  <section><h2>2. Passkey account (zero XLM; created by the relayer, A2)</h2>
    <div id="acct"></div>
    <button id="create">Create with this device (Windows Hello / Touch ID)</button>
    <button id="createPhone">Create with phone / security key</button>
    <button id="faucet">Get 1,000 test USDC</button>
    <button id="forget">Start over with a new passkey</button></section>

  <section><h2>2b. Stellar wallet (Freighter/Varial, xBull, LOBSTR, Albedo, Hana, ...)</h2>
    <p>A classic Stellar account: you sign in your wallet and pay the XLM network fee yourself. Same vault, same shares.</p>
    <button id="connect">Connect a Stellar wallet</button>
    <button id="wfund">Fund with friendbot (local/testnet)</button>
    <button id="wtrust">Add USDC trustline</button>
    <button id="wfaucet">Get 1,000 test USDC</button>
    <div id="wallet">not connected</div></section>

  <section><h2>3. Deposit / redeem</h2>
    Sign in with: <label><input type="radio" name="who" value="passkey" checked/> passkey (relayer pays, zero XLM)</label>
    <label><input type="radio" name="who" value="wallet"/> Stellar wallet (you pay the XLM fee)</label><br/>
    USDC <input id="dep" value="100" size="8"/> <button id="deposit">Deposit</button>
    shares <input id="red" value="10" size="8"/> <button id="redeem">Redeem</button>
    <p>The keeper rebalances to the signed targets within ~20 s (watch the overview).</p></section>

  <section><h2>4. AI thesis -> signed policy (thesis API)</h2>
    <textarea id="thesis">We are a small studio. Keep a safe runway in dollars with at least 10% instant cash.</textarea>
    jurisdiction <input id="jur" value="DE" size="4"/>
    class <select id="cls"><option>retail</option><option>professional</option><option>institutional</option></select>
    <label><input type="checkbox" id="llm"/> use the LLM (only if the server enables it)</label>
    <button id="run">Draft policy</button>
    <div id="thesisOut"></div></section>

  <section><h2>5. Frozen asset -> claim -> keeper settles (V6 / A5)</h2>
    <button id="freeze">Freeze EURC prices (oracle circuit breaker)</button>
    <button id="unfreeze">Restore prices</button>
    shares <input id="fz" value="10" size="8"/> <button id="redeemFrozen">Redeem while frozen</button>
    <div id="claims"></div></section>

  <section><h2>6. Collateral Passport feed (D-N)</h2>
    <div id="cp"></div>
    <p>Borrowing against shares on a live Blend pool is proven in the contract tests
    (<code>collateral_passport::*</code>); a local Blend pool for this console is a later step.</p></section>

  <section><h2>7. Admin (local throwaway admin key)</h2>
    <button id="pause">Pause vault</button> <button id="unpause">Unpause vault</button>
    <button id="vaults">List factory vaults</button> <div id="vaultList"></div></section>`;

  $("refresh").onclick = () => refresh();
  $("create").onclick = () => act("create passkey account", () => createAccount("platform"));
  $("createPhone").onclick = () => act("create passkey account", () => createAccount("cross-platform"));
  $("faucet").onclick = () => act("faucet", async () => {
    const a = need();
    return chain.invokeAs("issuer", cfg.assets.USDC, "mint", [sv.addr(a.address), sv.i128(1_000n * ONE)]);
  });
  $("forget").onclick = () => { try { localStorage.removeItem(ACCOUNT_KEY); } catch { /* */ } refresh(); };
  $("deposit").onclick = () => act("deposit", () => {
    const u = actor(); return u.call("deposit", userArgs(toUnits($<HTMLInputElement>("dep").value), u.address));
  });
  $("redeem").onclick = () => act("redeem", () => {
    const u = actor(); return u.call("redeem", userArgs(toUnits($<HTMLInputElement>("red").value), u.address));
  });
  $("redeemFrozen").onclick = () => act("redeem while frozen", () => {
    const u = actor(); return u.call("redeem_frozen", userArgs(toUnits($<HTMLInputElement>("fz").value), u.address));
  });
  $("connect").onclick = () => act("connect wallet", async () => { wallet = await connectWallet(cfg.network.passphrase); return wallet.address; });
  const needWallet = () => { if (!wallet) throw new Error("connect a Stellar wallet first"); return wallet; };
  $("wfund").onclick = () => act("friendbot", () => friendbotFund(chain, needWallet().address));
  $("wtrust").onclick = () => act("add USDC trustline", () => addUsdcTrustline(chain, needWallet()));
  $("wfaucet").onclick = () => act("faucet (wallet)", () =>
    chain.invokeAs("issuer", cfg.assets.USDC, "mint", [sv.addr(needWallet().address), sv.i128(1_000n * ONE)]));
  $("freeze").onclick = () => act("freeze EURC", () =>
    chain.invokeAs("admin", cfg.contracts.oracle, "trigger_circuit_breaker", [sv.addr(cfg.accounts.admin), sv.sym("CASH"), sv.sym("SANDBOX")]));
  $("unfreeze").onclick = () => act("restore prices", () =>
    chain.invokeAs("admin", cfg.contracts.oracle, "clear_circuit_breaker", [sv.sym("CASH")]));
  $("pause").onclick = () => act("pause", () => chain.invokeAs("admin", cfg.contracts.vault, "pause"));
  $("unpause").onclick = () => act("unpause", () => chain.invokeAs("admin", cfg.contracts.vault, "unpause"));
  $("vaults").onclick = () => act("factory vaults", async () => {
    const list = await chain.view(cfg.contracts.vault_factory, "vaults_of", [sv.addr(cfg.accounts.admin)]);
    $("vaultList").innerHTML = list.length ? list.map((v: string) => `<div class="mono">${v}</div>`).join("") : "none yet";
    return list.length;
  });
  $("run").onclick = () => act("draft policy", runThesis);
}

function need(): Passkey {
  const a = loadAccount();
  if (!a) throw new Error("create a passkey account first");
  if (!a.address) throw new Error("account setup is unfinished - click 'Create passkey account' to finish it");
  return a;
}

async function createAccount(where: "platform" | "cross-platform") {
  const existing = loadAccount();
  if (existing && !existing.address) return finishSetup(existing);
  if (existing) throw new Error("this browser already has an account - use 'Start over with a new passkey' first");
  const pk = await createPasskey(`constella-${Date.now()}`, where);
  // Save the passkey BEFORE asking the relayer: if the page reloads or the
  // request is interrupted, "Finish setup" retries (creation is idempotent:
  // the address is derived from the key). First real-browser test lost it.
  const pending: Passkey = { credentialId: pk.id, publicKey: pk.publicKey, address: "", attachment: pk.attachment, transports: pk.transports };
  saveAccount(pending);
  return finishSetup(pending);
}

async function finishSetup(a: Passkey) {
  const hex = Array.from(a.publicKey, (b) => b.toString(16).padStart(2, "0")).join("");
  const out = await chain.relayer("/accounts", { owner_key: hex });
  saveAccount({ ...a, address: out.account });
  return out;
}

async function runThesis() {
  const body = { text: $<HTMLTextAreaElement>("thesis").value, jurisdiction: $<HTMLInputElement>("jur").value,
                 investor_class: $<HTMLSelectElement>("cls").value, use_llm: $<HTMLInputElement>("llm").checked };
  const { status, out } = await thesis(cfg, body);
  if (status !== 200) {
    $("thesisOut").innerHTML = `<p><b>${out.error}</b></p><pre>${JSON.stringify(out.reasons ?? out, null, 1)}</pre>`;
    return status;
  }
  const rows = Object.entries(out.allocation.weights_bps as Record<string, number>)
    .filter(([, w]) => w > 0).map(([a, w]) => `<tr><td>${a}</td><td>${(w / 100).toFixed(2)} %</td><td>${out.explanation.asset_rationales[a] ?? ""}</td></tr>`).join("");
  $("thesisOut").innerHTML = `
    <p>profile <b>${out.allocation.profile}</b>, vault currency <b>${out.allocation.numeraire}</b>, parsed by <b>${out.parse_source}</b>
      ${out.ai_note ? `<br><i>${out.ai_note}</i>` : ""}</p>
    <table><tr><th>asset</th><th>weight</th><th>why</th></tr>${rows}</table>
    ${out.ai_summary ? `<p><b>AI summary:</b> ${out.ai_summary.summary}</p>` : ""}
    <p><b>Warnings:</b> ${(out.explanation.issuer_concentration_warnings as string[]).join("<br>") || "none"}</p>
    <p><b>Needs your confirmation:</b> ${(out.explanation.ambiguity_clarifications as string[]).join("<br>") || "nothing"}</p>
    <p>policy hash <span class="mono">${out.policy_hash}</span></p>
    <details><summary>exact on-chain bind_policy terms</summary><pre>${JSON.stringify(out.bind_policy, null, 1)}</pre></details>
    <details><summary>not eligible for this user</summary><pre>${JSON.stringify(out.ineligible, null, 1)}</pre></details>`;
  return out.policy_hash;
}

async function refresh() {
  const c = cfg.contracts;
  try {
    const [cons, dep, total, supply, sleeves, paused, hwm, history, eurc] = await Promise.all([
      chain.view(c.vault, "get_nav_conservative"), chain.view(c.vault, "get_nav_deposit"), chain.view(c.vault, "total_assets"),
      chain.view(c.vault, "total_supply"), chain.view(c.vault, "get_sleeves"), chain.view(c.vault, "is_paused"),
      chain.view(c.vault, "get_high_water_mark"), chain.view(c.vault, "get_nav_history"),
      chain.view(c.oracle, "asset_price", [sv.addr(cfg.assets.EURC)]),
    ]);
    const realized = history.length >= 2
      ? (((Number(history.at(-1).nav) / Number(history[0].nav)) - 1) * 31_536_000 / Number(history.at(-1).timestamp - history[0].timestamp) * 100).toFixed(3) + " %/yr"
      : "not enough history yet (one point a day)";
    const rows = sleeves.map((s: any) => `<tr><td>${s.asset_id}</td><td>${(s.target_weight_bps / 100).toFixed(1)} %</td>
      <td>${fmt(s.current_balance)}</td><td class="mono">${short(s.contract_address)}</td></tr>`).join("");
    $("overview").innerHTML = `
      <table>
        <tr><td>NAV / share (conservative · deposit)</td><td>${cons ? fmt(cons.price) : "UNPRICED"} · ${dep ? fmt(dep.price) : "UNPRICED"}</td></tr>
        <tr><td>total assets · shares</td><td>${fmt(total)} USDC · ${fmt(supply)}</td></tr>
        <tr><td>high-water mark (D-D fee)</td><td>${fmt(hwm)}</td></tr>
        <tr><td>EURC price (oracle)</td><td>${eurc ? fmt(eurc.price) : "NONE (frozen or stale)"}</td></tr>
        <tr><td>realised return (on-chain NAV history, A3)</td><td>${realized}</td></tr>
        <tr><td>vault</td><td>${paused ? "PAUSED" : "running"}</td></tr>
      </table>
      <table><tr><th>sleeve</th><th>target</th><th>held (units)</th><th>contract</th></tr>${rows}</table>`;

    const a = loadAccount();
    if (a && !a.address) {
      $("acct").innerHTML = `<b>Passkey saved, account setup unfinished.</b> Click either Create button to finish (no new passkey is created), or "Start over with a new passkey".`;
      $("claims").textContent = "";
    } else if (a) {
      const [usdc, shares, claim] = await Promise.all([
        chain.view(cfg.assets.USDC, "balance", [sv.addr(a.address)]), chain.view(c.vault, "balance", [sv.addr(a.address)]),
        chain.view(c.vault, "get_claim", [sv.addr(a.address), sv.addr(cfg.assets.EURC)]),
      ]);
      const where = a.attachment === "platform" ? "this device (Windows Hello / Touch ID)" : a.attachment === "cross-platform" ? "a phone or security key" : "unknown (created before this was recorded)";
      $("acct").innerHTML = `<div class="mono">account ${a.address}</div><div>USDC ${fmt(usdc)} · vault shares ${fmt(shares)} · XLM: none needed</div>
        <div>passkey stored on: <b>${where}</b>${a.transports?.length ? ` (${a.transports.join(", ")})` : ""} - signing needs that same passkey</div>`;
      $("claims").innerHTML = `open EURC claim for this account: <b>${fmt(claim)}</b> units ${BigInt(claim) > 0n ? "(the keeper settles it once EURC is priced again)" : ""}`;
    } else {
      $("acct").textContent = "no account in this browser yet";
      $("claims").textContent = "";
    }
    if (wallet) {
      const ws = await walletStatus(chain, wallet.address);
      const shares = ws.exists ? await chain.view(c.vault, "balance", [sv.addr(wallet.address)]) : 0;
      const claim = ws.exists ? await chain.view(c.vault, "get_claim", [sv.addr(wallet.address), sv.addr(cfg.assets.EURC)]) : 0;
      const todo = !ws.exists ? "Next: fund the account (friendbot)." : !ws.hasTrustline ? "Next: add the USDC trustline." : ws.usdc === 0n ? "Next: get test USDC." : "Ready.";
      $("wallet").innerHTML = `<div class="mono">${wallet.label} ${wallet.address}</div>
        <div>account ${ws.exists ? "exists" : "NOT FUNDED"} · XLM ${fmt(ws.xlm)} · USDC trustline ${ws.hasTrustline ? "yes" : "NO"} · USDC ${fmt(ws.usdc)}
        · vault shares ${fmt(shares)} · open EURC claim ${fmt(claim)}</div><b>${todo}</b>`;
    }
    const share = await chain.view(c.collateral_oracle, "lastprice", [xdr.ScVal.scvVec([sv.sym("Stellar"), sv.addr(c.vault)])]);
    $("cp").innerHTML = `share price served to lenders: <b>${share ? fmt(share.price) : "NONE - the vault cannot value itself, lending halts"}</b>
      (vault conservative NAV ${cons ? fmt(cons.price) : "none"})`;
  } catch (e: any) {
    log(`refresh failed - ${e?.message ?? e}`);
  }
}

async function main() {
  try {
    cfg = await (await fetch("/config.json")).json();
  } catch {
    return; // index.html already tells the user to run localnet/up.sh
  }
  chain = new Chain(cfg);
  layout();
  await refresh();
  setInterval(refresh, 10_000);
}


main();
