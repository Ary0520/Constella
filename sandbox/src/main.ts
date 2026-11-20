/**
 * Constella sandbox - THROWAWAY feature console (local network only).
 * Plain DOM, raw numbers: a place to click through every implemented flow,
 * not a design. Delete sandbox/ to remove it.
 */
import { xdr } from "@stellar/stellar-sdk";
import { Chain, Config, Passkey, sv, thesis } from "./chain";
import { createPasskey, createTestPasskey, discoverPasskey, getAssertion, recoverPublicKeys } from "./passkey";
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
    status(`${label} failed: ${explain(msg)}`, "err");
  }
}

/** Plain-language explanations for the errors users actually meet (the raw
 * code stays in the activity log). */
function explain(msg: string): string {
  const m = msg.toLowerCase();
  if (m.includes("rate_limited")) return "daily limit reached for this action (testnet faucet / new accounts). Try again tomorrow or from another network.";
  if (m.includes("notallowederror") || m.includes("the operation either timed out or was not allowed")) return "the passkey prompt was cancelled or failed on the device. Try again; on a phone, make sure you approve the prompt for this website.";
  if (m.includes("failed to fetch")) return "could not reach the Constella service (network). Check your connection and try again.";
  if (m.includes("faucet_refused")) return "the faucet could not send to this address (a Stellar wallet account needs a USDC trustline first).";
  if (m.includes("error(contract, #10)") || m.includes("balanceerror")) return "not enough balance for this transaction (USDC or shares). Get test USDC first, or use a smaller amount.";
  if (m.includes("error(contract, #8)")) return "the vault does not hold enough cash for this redemption right now - try a smaller amount.";
  if (m.includes("simulation_failed")) return "the network rejected the transaction in a dry run, before anything was signed or spent. Details are in the activity log.";
  return msg;
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
/** Hosted (testnet) console: no secrets at all - faucet via the relayer,
 * operator actions hidden. Local console: throwaway local keys. */
const hosted = () => !cfg.LOCAL_ONLY_secrets;

async function testUsdc(address: string) {
  if (hosted()) return chain.relayer("/faucet", { address });
  return chain.invokeAs("issuer", cfg.assets.USDC, "mint", [sv.addr(address), sv.i128(1_000n * ONE)]);
}

function layout() {
  const c = cfg.contracts;
  $("app").innerHTML = `
  <section><h2>1. Vault overview <button id="refresh">Refresh</button></h2>
    <div class="mono">vault ${c.vault} · policy ${c.policy} · oracle ${c.oracle}</div>
    <div id="overview">...</div></section>

  <section><h2>2. Passkey account (zero XLM; created by the relayer, A2)</h2>
    <div id="acct"></div>
    <button id="create">Create - use THIS device (fingerprint / face / screen lock)</button>
    <button id="createPhone">Create - use ANOTHER device (scan a QR code with your phone)</button>
    <button id="signin">Sign in with an existing passkey</button>
    <button id="createTest">Create a test passkey (in this browser - local testing only)</button>
    <button id="faucet">Get 1,000 test USDC</button>
    <button id="forget">Start over with a new passkey</button>
    <h3>Your devices</h3>
    <div id="devices"></div>
    <button id="addDevice">Add another device (phone, laptop, ...)</button>
    <div id="pairBox"></div>
    <div id="joinBox"></div></section>

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
  $("signin").onclick = () => act("sign in with an existing passkey", signIn);
  $("addDevice").onclick = () => act("add another device", addDevice);
  const pairCode = new URLSearchParams(location.search).get("pair");
  if (pairCode) showJoin(pairCode);
  $("createTest").onclick = () => act("create test passkey account", async () => {
    if (loadAccount()) throw new Error("this browser already has an account - use 'Start over with a new passkey' first");
    const t = await createTestPasskey();
    const pending: Passkey = { credentialId: t.id, publicKey: t.publicKey, address: "", attachment: "test", testKey: t.jwk };
    saveAccount(pending);
    return finishSetup(pending);
  });
  $("faucet").onclick = () => act("faucet", async () => {
    const a = need();
    return testUsdc(a.address);
  });
  $("forget").onclick = () => { try { localStorage.removeItem(ACCOUNT_KEY); } catch { /* */ } refresh(); };
  $("deposit").onclick = () => act("deposit", async () => {
    const u = actor(); const amt = toUnits($<HTMLInputElement>("dep").value);
    const bal = BigInt(await chain.view(cfg.assets.USDC, "balance", [sv.addr(u.address)]).catch(() => 0));
    if (amt <= 0n) throw new Error("enter an amount above 0");
    if (bal < amt) throw new Error(`you have ${fmt(bal)} USDC, so you cannot deposit ${fmt(amt)}. Click "Get 100 test USDC" first, or deposit less.`);
    return u.call("deposit", userArgs(amt, u.address));
  });
  $("redeem").onclick = () => act("redeem", async () => {
    const u = actor(); const sh = toUnits($<HTMLInputElement>("red").value);
    const have = BigInt(await chain.view(cfg.contracts.vault, "balance", [sv.addr(u.address)]).catch(() => 0));
    if (have < sh) throw new Error(`you hold ${fmt(have)} vault shares, so you cannot redeem ${fmt(sh)}. Deposit first, or redeem less.`);
    return u.call("redeem", userArgs(sh, u.address));
  });
  $("redeemFrozen").onclick = () => act("redeem while frozen", () => {
    const u = actor(); return u.call("redeem_frozen", userArgs(toUnits($<HTMLInputElement>("fz").value), u.address));
  });
  $("connect").onclick = () => act("connect wallet", async () => { wallet = await connectWallet(cfg.network.passphrase); return wallet.address; });
  const needWallet = () => { if (!wallet) throw new Error("connect a Stellar wallet first"); return wallet; };
  $("wfund").onclick = () => act("friendbot", () => friendbotFund(chain, needWallet().address));
  $("wtrust").onclick = () => act("add USDC trustline", () => addUsdcTrustline(chain, needWallet()));
  $("wfaucet").onclick = () => act("faucet (wallet)", () => testUsdc(needWallet().address));
  if (hosted()) {
    // Operator-only on the hosted testnet console (no keys in the browser).
    for (const id of ["freeze", "unfreeze", "pause", "unpause"]) $(id).style.display = "none";
    $("freeze").insertAdjacentHTML("beforebegin", `<p><i>On testnet, freezing prices and pausing are operator actions (not available from this public page). Redeem-while-frozen works whenever a price is genuinely unavailable.</i></p>`);
    for (const id of ["faucet", "wfaucet"]) $(id).textContent = "Get 100 test USDC (testnet faucet, once a day)";
    document.querySelector("h1")!.textContent = "Constella sandbox - Stellar TESTNET";
  }
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

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

/** Find the account of ANY passkey this device holds for this site: the
 * device lists its passkeys; the public key is recovered from the signature
 * and the account factory says which key owns an account. Also a diagnostic:
 * "no passkey" means this device is not storing one for this site. */
async function signIn() {
  if (loadAccount()?.address) throw new Error("this browser already has an account - use 'Start over with a new passkey' first");
  let found;
  try {
    found = await discoverPasskey();
  } catch (e: any) {
    throw new Error(`this device returned no passkey for ${location.hostname} (${e?.name ?? e}). If you created one on a phone, open this page ON that phone and use "Sign in", or create a new one with "use THIS device".`);
  }
  for (const pk of found.candidates) {
    const dev = cfg.contracts.device_registry ? await chain.view(cfg.contracts.device_registry, "device_of", [sv.bytes(pk)]) : null;
    if (dev) {
      saveAccount({ credentialId: found.id, publicKey: pk, address: String(dev.account), ruleId: Number(dev.rule_id), attachment: "existing" });
      return { signed_in: String(dev.account), as_device: dev.name };
    }
    const acct = await chain.view(cfg.contracts.account_factory, "account_of", [sv.bytes(pk)]);
    if (acct) {
      const address = typeof acct === "string" ? acct : String(acct);
      saveAccount({ credentialId: found.id, publicKey: pk, address, attachment: "existing" });
      return { signed_in: address };
    }
  }
  // A passkey with no account yet (an unfinished earlier attempt): a second
  // signature pins down the exact key, then setup finishes with it.
  const challenge = crypto.getRandomValues(new Uint8Array(32));
  const a2 = await getAssertion(found.id, challenge);
  const clientHash = new Uint8Array(await crypto.subtle.digest("SHA-256", a2.clientDataJSON as BufferSource));
  const second = (await recoverPublicKeys(new Uint8Array([...a2.authenticatorData, ...clientHash]), a2.signature)).map(hex);
  const key = found.candidates.find((k) => second.includes(hex(k)));
  if (!key) throw new Error("could not determine this passkey's public key");
  const pending: Passkey = { credentialId: found.id, publicKey: key, address: "", attachment: "existing" };
  saveAccount(pending);
  return finishSetup(pending);
}

// ------------------------------------------------ one account, several devices
/** On the EXISTING device: open a pairing code, wait for the new device,
 * show the shared check code, and approve with this device's passkey. */
async function addDevice() {
  const a = need();
  if (a.ruleId !== undefined) throw new Error("only the first device of this account can add devices (security: a lost extra device cannot add others)");
  if (!cfg.contracts.device_registry) throw new Error("this deployment has no device registry");
  const { code } = await chain.relayer("/pair/start", { account: a.address });
  const link = `${location.origin}${location.pathname}?pair=${code}`;
  $("pairBox").innerHTML = `<p><b>On your other device</b>, open this link (send it to yourself, e.g. by message):<br>
    <span class="mono">${link}</span><br>or open the site there and add <span class="mono">?pair=${code}</span>. Waiting for it (10 minutes)...</p>`;
  for (let i = 0; i < 300; i++) {
    await new Promise((r) => setTimeout(r, 2000));
    const st = await chain.relayer("/pair/status", { code });
    if (st.device_key) {
      $("pairBox").innerHTML = `<p><b>"${st.name}" wants to join this account.</b> Check code: <b class="mono">${st.check}</b> - it must match the code on the other device.</p>
        <button id="approveDevice">Approve this device</button> <button id="rejectDevice">Not my device</button>`;
      const ok = await new Promise<boolean>((res) => { $("approveDevice").onclick = () => res(true); $("rejectDevice").onclick = () => res(false); });
      if (!ok) { $("pairBox").textContent = "Rejected - nothing was added."; return "rejected"; }
      status("approving the device (your passkey prompt)...", "busy");
      const key = Uint8Array.from(st.device_key.match(/../g).map((h: string) => parseInt(h, 16)));
      const out = await chain.sponsored(a, cfg.contracts.device_registry, "add_device", [sv.addr(a.address), sv.bytes(key), sv.str(st.name)]);
      $("pairBox").textContent = `"${st.name}" added. It can now use this account on its own.`;
      return out;
    }
  }
  throw new Error("pairing expired - start again");
}

/** On the NEW device (opened with ?pair=CODE): create a passkey here, send
 * its public key, show the check code, and sign in once approved. */
function showJoin(code: string) {
  $("joinBox").innerHTML = `<h3>Join an existing account</h3>
    <p>Your other device asked to add this one. Create a passkey on THIS device, then approve it on the other device.</p>
    <button id="joinBtn">Create a passkey on this device and join</button> <span id="joinInfo"></span>`;
  $("joinBtn").onclick = () => act("join account", async () => {
    if (loadAccount()?.address) throw new Error("this browser already has an account - use 'Start over with a new passkey' first");
    const pk = await createPasskey(`constella-device-${Date.now()}`, "platform");
    const keyHex = Array.from(pk.publicKey, (b) => b.toString(16).padStart(2, "0")).join("");
    const name = /Android|iPhone|iPad/i.test(navigator.userAgent) ? "phone" : "computer";
    const j = await chain.relayer("/pair/join", { code, device_key: keyHex, name });
    $("joinInfo").innerHTML = `Check code <b class="mono">${j.check}</b> - approve it on your other device. Waiting...`;
    for (let i = 0; i < 300; i++) {
      await new Promise((r) => setTimeout(r, 3000));
      const dev = await chain.view(cfg.contracts.device_registry, "device_of", [sv.bytes(pk.publicKey)]);
      if (dev) {
        saveAccount({ credentialId: pk.id, publicKey: pk.publicKey, address: String(dev.account), ruleId: Number(dev.rule_id),
                      attachment: pk.attachment, transports: pk.transports });
        history.replaceState(null, "", location.pathname);
        $("joinBox").innerHTML = "<p><b>This device now uses your account.</b></p>";
        return { joined: String(dev.account) };
      }
    }
    throw new Error("not approved within the time limit");
  });
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
      const where = a.testKey ? "this browser (TEST passkey - local testing only, no prompt)" : a.attachment === "existing" ? "the device you signed in with" : a.attachment === "platform" ? "this device (Windows Hello / Touch ID)" : a.attachment === "cross-platform" ? "a phone or security key" : "unknown (created before this was recorded)";
      $("acct").innerHTML = `<div class="mono">account ${a.address}</div><div>USDC ${fmt(usdc)} · vault shares ${fmt(shares)} · XLM: none needed</div>
        <div>passkey stored on: <b>${where}</b>${a.transports?.length ? ` (${a.transports.join(", ")})` : ""} - signing needs that same passkey</div>`;
      if (cfg.contracts.device_registry) {
        const devs = await chain.view(cfg.contracts.device_registry, "devices_of", [sv.addr(a.address)]).catch(() => []);
        $("devices").innerHTML = `this device: <b>${a.ruleId === undefined ? "first device (can add others)" : `added device (rule ${a.ruleId})`}</b>`
          + (devs.length ? `<br>added devices: ${devs.map((d: any) => `${d.name} (rule ${d.rule_id})`).join(", ")}` : "<br>no other devices yet");
      }
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
