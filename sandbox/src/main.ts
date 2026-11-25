/**
 * Constella sandbox - THROWAWAY feature console (local network only).
 * Plain DOM, raw numbers: a place to click through every implemented flow,
 * not a design. Delete sandbox/ to remove it.
 */
import { Address, xdr } from "@stellar/stellar-sdk";
import { Chain, Config, funcB64, Passkey, sv, thesis } from "./chain";
import { createPasskey, createTestPasskey, discoverPasskey, getAssertion, recoverPublicKeys } from "./passkey";
import { addUsdcTrustline, connectWallet, friendbotFund, WalletSigner, walletInvoke, walletStatus } from "./wallet";
import { createVaultArgs } from "./ownvault";
import { fixExpiry, GuardianKey, guardianSign, proposalArgs } from "./recovery";

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

// Step 3: which vault section 3 acts on - the shared Cash Reserve or one of
// the user's own vaults (created from a signed policy, section 4).
const TARGET_KEY = "constella-sandbox-target";
function target(): string {
  const sel = document.getElementById("target") as HTMLSelectElement | null;
  if (sel?.value) return sel.value;
  try { return localStorage.getItem(TARGET_KEY) || cfg.contracts.vault; } catch { return cfg.contracts.vault; }
}
function setTarget(v: string) { try { localStorage.setItem(TARGET_KEY, v); } catch { /* private mode */ } }
/** The signable draft from section 4 (policy, hash, bind_policy terms). */
let lastDraft: any = null;

/** Who acts: the passkey account (relayer pays) or the wallet (user pays). */
function actor(contract: string = target()): { address: string; call: (fn: string, args: xdr.ScVal[]) => Promise<unknown> } {
  if (mode() === "wallet") {
    if (!wallet) throw new Error("connect a Stellar wallet first (section 2b)");
    const w = wallet;
    return { address: w.address, call: (fn, args) => walletInvoke(chain, w, contract, fn, args) };
  }
  const a = need();
  return { address: a.address, call: (fn, args) => chain.sponsored(a, contract, fn, args) };
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
    Vault: <select id="target"></select><br/>
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

  <section><h2>8. Account recovery (guardians, D-F)</h2>
    <p>If every passkey of an account is lost, its guardians (your other devices or trusted people) can approve a NEW
    passkey. It takes effect after 48 hours; the owner can cancel meanwhile. Guardians can never move funds.</p>
    <h3>Your guardians (owner)</h3>
    <div id="guardians"></div>
    <button id="addGuardian">Add a guardian (another device or a trusted person)</button>
    approvals needed <input id="threshold" value="1" size="2"/> <button id="saveGuardians">Save guardians</button>
    <button id="cancelRecovery">Cancel a pending recovery</button>
    <div id="guardBox"></div><div id="guardJoin"></div>
    <h3>Lost every passkey? Recover your account (on the new device)</h3>
    account <input id="recAccount" size="58" placeholder="C... (ask a guardian if you do not know it)"/>
    <button id="recover">Recover - new passkey on THIS device</button>
    <button id="recoverTest">Recover - test passkey (local testing only)</button>
    <button id="finishRecovery">Finish recovery (after 48 hours)</button>
    <div id="recBox"></div><div id="approveBox"></div></section>

  <section><h2>7. Admin (local throwaway admin key)</h2>
    <button id="pause">Pause vault</button> <button id="unpause">Unpause vault</button>
    <button id="vaults">List factory vaults</button> <div id="vaultList"></div></section>`;

  $("refresh").onclick = () => refresh();
  $("target").onchange = () => { setTarget(target()); refresh(); };
  $("create").onclick = () => act("create passkey account", () => createAccount("platform"));
  $("createPhone").onclick = () => act("create passkey account", () => createAccount("cross-platform"));
  $("signin").onclick = () => act("sign in with an existing passkey", signIn);
  $("addDevice").onclick = () => act("add another device", addDevice);
  const pairCode = new URLSearchParams(location.search).get("pair");
  if (pairCode) showJoin(pairCode);
  const guardCode = new URLSearchParams(location.search).get("guard");
  if (guardCode) showGuardJoin(guardCode);
  const recId = new URLSearchParams(location.search).get("recover");
  if (recId) showApprove(recId);
  $("addGuardian").onclick = () => act("add a guardian", addGuardian);
  $("saveGuardians").onclick = () => act("save guardians", saveGuardians);
  $("cancelRecovery").onclick = () => act("cancel recovery", async () => { const a = need(); return chain.sponsored(a, a.address, "cancel_recovery", []); });
  $("recover").onclick = () => act("start recovery", () => startRecovery("platform"));
  $("recoverTest").onclick = () => act("start recovery", () => startRecovery("test"));
  $("finishRecovery").onclick = () => act("finish recovery", finishRecovery);
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
    const have = BigInt(await chain.view(target(), "balance", [sv.addr(u.address)]).catch(() => 0));
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
    // Step 8: a key that became the owner through a recovery.
    const recovered = cfg.contracts.recovery_registry ? await chain.view(cfg.contracts.recovery_registry, "owner_account", [sv.bytes(pk)]) : null;
    if (recovered) {
      saveAccount({ credentialId: found.id, publicKey: pk, address: String(recovered), attachment: "existing" });
      return { signed_in: String(recovered), recovered: true };
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

/** Step 7 (spec THESIS-ENGINE Step 4): the simulation, shown before any money
 * moves - labelled "Simulation", worst case first, Rule 10 disclaimer. */
function simulationHtml(sim: any): string {
  if (!sim) return "";
  const pct = (x: number) => `${(x * 100).toFixed(1)} %`;
  const hours = (h: number) => (h >= 48 ? `${Math.round(h / 24)} days` : h >= 24 ? "1 day" : "under 1 hour");
  const l = sim.redemption_ladder, m = sim.redeemable_during_freeze;
  return `<h4>Simulation (before any money moves)</h4>
    <p><b>Worst case: a full exit could take ${hours(sim.worst_case_exit_hours)}</b> - ${sim.worst_case_scenario}.
      Normal full exit: ${hours(sim.normal_exit_hours)}.</p>
    <p><b>Issuer freeze:</b> ${sim.issuer_freeze_stress.description}</p>
    <p><b>Depeg:</b> ${sim.depeg_stress.description}</p>
    <p><b>Redeemable:</b> instant ${pct(l.instant_pct)} · within 24 h ${pct(l.within_24h_pct)} · within 1 week ${pct(l.within_1w_pct)}
      · longer ${pct(l.illiquid_pct)}${m ? ` (during the freeze: instant ${pct(m.instant_pct)}, 24 h ${pct(m.within_24h_pct)})` : ""}</p>
    <p><b>Estimated yield:</b> ${sim.weighted_yield_pct.toFixed(2)} %/yr (an unknown rate counts as 0)</p>
    <p>${sim.history_note}</p>
    <p><i>${sim.disclaimer}</i></p>`;
}

async function runThesis() {
  // OPEN-ISSUES #3: the policy hash commits to the account that will sign it
  // (passkey smart account, else the connected wallet); without one the API
  // returns a preview with no signable hash.
  const owner = loadAccount()?.address ?? wallet?.address;
  const body = { text: $<HTMLTextAreaElement>("thesis").value, jurisdiction: $<HTMLInputElement>("jur").value,
                 investor_class: $<HTMLSelectElement>("cls").value, use_llm: $<HTMLInputElement>("llm").checked,
                 ...(owner ? { owner } : {}) };
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
    ${simulationHtml(out.simulation)}
    <h4>Your thesis vs this basket</h4>
    <table><tr><th>you asked</th><th>proposed</th><th>why</th></tr>${(out.explanation.thesis_diff as any[])
      .map((d) => `<tr><td>${d.requested}</td><td>${d.proposed}</td><td>${d.explanation}</td></tr>`).join("")}</table>
    <p><b>Warnings:</b> ${(out.explanation.issuer_concentration_warnings as string[]).join("<br>") || "none"}</p>
    <p><b>Needs your confirmation:</b> ${(out.explanation.ambiguity_clarifications as string[]).join("<br>") || "nothing"}</p>
    <p>${out.signable ? `policy hash for <span class="mono">${out.policy.admin.admin_address}</span>: <span class="mono">${out.policy_hash}</span>`
                      : "<i>Preview only: create or connect an account to get a policy you can sign.</i>"}</p>
    <details><summary>exact on-chain bind_policy terms</summary><pre>${JSON.stringify(out.bind_policy, null, 1)}</pre></details>
    <details><summary>not eligible for this user</summary><pre>${JSON.stringify(out.ineligible, null, 1)}</pre></details>`;
  lastDraft = out;
  if (out.signable && cfg.contracts.vault_factory) {
    $("thesisOut").insertAdjacentHTML("beforeend", `<p><button id="mkVault">Create my vault from this policy</button>
      <i>one signature: deploys your private vault, binds this exact policy with you as owner, sets Constella's keeper.
      Fee: Constella's default (10 % of profit, never above 0.20 %/yr). Cap 25,000 USDC.</i></p>`);
    $("mkVault").onclick = () => act("create my vault", createOwnVault);
  }
  return out.policy_hash;
}

/** Step 3: fill the vault selector - the shared vault plus the acting
 * identity's own factory vaults (vault_factory.vaults_of). */
async function refreshTargets(prefer?: string) {
  const sel = document.getElementById("target") as HTMLSelectElement | null;
  if (!sel || !cfg.contracts.vault_factory) return;
  const who = mode() === "wallet" ? wallet?.address : loadAccount()?.address;
  const own: string[] = who ? (await chain.view(cfg.contracts.vault_factory, "vaults_of", [sv.addr(who)]).catch(() => [])).map(String) : [];
  const want = prefer ?? target();
  sel.innerHTML = [`<option value="${cfg.contracts.vault}">Constella Cash Reserve (shared)</option>`,
                   ...own.map((v, i) => `<option value="${v}">My vault ${i + 1}: ${short(v)}</option>`)].join("");
  sel.value = [cfg.contracts.vault, ...own].includes(want) ? want : cfg.contracts.vault;
  setTarget(sel.value);
}

/** Step 3: the signed draft becomes the user's own live vault - one call,
 * one signature (vault_factory.create_vault_with_policy). */
async function createOwnVault() {
  const d = lastDraft;
  if (!d?.signable) throw new Error("draft a policy first (create or connect an account so the draft is signable)");
  const u = actor(cfg.contracts.vault_factory);
  if (d.policy.admin.admin_address !== u.address) {
    throw new Error("this draft was made for another account - draft the policy again with the account selected in section 3");
  }
  const settings = await chain.view(cfg.contracts.vault_factory, "get_settings");
  const salt = crypto.getRandomValues(new Uint8Array(32));
  const name = String(d.policy.basket_name || "My basket");
  const before: string[] = (await chain.view(cfg.contracts.vault_factory, "vaults_of", [sv.addr(u.address)]).catch(() => [])).map(String);
  await u.call("create_vault_with_policy", createVaultArgs({
    owner: u.address, numeraire: d.numeraire_contract ?? cfg.assets.USDC, feeBps: Number(settings.default_fee_bps),
    capUnits: 25_000n * 10_000_000n, name, symbol: name.replace(/[^A-Za-z]/g, "").slice(0, 8).toUpperCase() || "BASKET",
    salt, bind: d.bind_policy, policyHashHex: d.policy_hash,
  }));
  const after: string[] = (await chain.view(cfg.contracts.vault_factory, "vaults_of", [sv.addr(u.address)])).map(String);
  const created = after.find((v) => !before.includes(v));
  if (!created) throw new Error("the vault was not created");
  setTarget(created);
  await refreshTargets(created);
  return { vault: created, policy_hash: d.policy_hash };
}

// ------------------------------------------------------------ step 8: recovery
const GUARDIAN_KEYS = "constella-sandbox-guardian-keys";
const RECOVERY_KEY = "constella-sandbox-recovery";
const fromHexStr = (h: string) => Uint8Array.from(h.match(/../g)!.map((x) => parseInt(x, 16)));
const readJson = (k: string, d: any) => { try { return JSON.parse(localStorage.getItem(k) || "") ?? d; } catch { return d; } };
const writeJson = (k: string, v: any) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* private mode */ } };
/** Same 6-character check code as the pairing mailbox (SHA-256 of the key). */
async function checkCode(key: Uint8Array): Promise<string> {
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", key as BufferSource));
  return hex(d).slice(0, 6).toUpperCase();
}
let pendingGuardians: { key: string; check: string }[] = [];

/** Owner: invite a guardian through the pairing mailbox (public key + check code). */
async function addGuardian() {
  const a = need();
  if (a.ruleId !== undefined) throw new Error("only the account's first device (its owner passkey) can set guardians");
  if (!cfg.contracts.recovery_registry) throw new Error("this deployment has no recovery registry");
  const { code } = await chain.relayer("/pair/start", { account: a.address });
  const link = `${location.origin}${location.pathname}?guard=${code}`;
  $("guardBox").innerHTML = `<p><b>Send this link to your guardian</b> (your other device or a trusted person):<br>
    <span class="mono">${link}</span><br>Waiting for them to create their guardian passkey (10 minutes)...</p>`;
  for (let i = 0; i < 300; i++) {
    await new Promise((r) => setTimeout(r, 2000));
    const st = await chain.relayer("/pair/status", { code });
    if (st.device_key) {
      pendingGuardians.push({ key: st.device_key, check: st.check });
      $("guardBox").innerHTML = `<p>Guardian joined - check code <b class="mono">${st.check}</b> must match their screen.
        Pending: ${pendingGuardians.length}. Add more, then "Save guardians".</p>`;
      return { guardian_check: st.check, pending: pendingGuardians.length };
    }
  }
  throw new Error("the guardian did not join in time - start again");
}

/** Owner: install the guardians as one M-of-N rule (one passkey signature). */
async function saveGuardians() {
  const a = need();
  if (!pendingGuardians.length) throw new Error("add at least one guardian first");
  const t = Number($<HTMLInputElement>("threshold").value);
  if (!(t >= 1 && t <= pendingGuardians.length)) throw new Error(`approvals needed must be 1..${pendingGuardians.length}`);
  const keys = xdr.ScVal.scvVec(pendingGuardians.map((g) => sv.bytes(fromHexStr(g.key))));
  const out = await chain.sponsored(a, cfg.contracts.recovery_registry, "set_guardians", [sv.addr(a.address), keys, sv.u32(t)]);
  pendingGuardians = [];
  return out;
}

/** Guardian device (opened with ?guard=CODE): create a guardian passkey here. */
function showGuardJoin(code: string) {
  $("guardJoin").innerHTML = `<h3>Become a guardian</h3>
    <p>Someone asked you to help recover their Constella account if they ever lose their passkeys. Create a guardian
    passkey on this device. It can only ever approve a new passkey for them - never move money.</p>
    <button id="guardHere">Create my guardian passkey (this device)</button>
    <button id="guardTest">Create a test guardian passkey (local testing only)</button>`;
  const go = (test: boolean) => act("become a guardian", async () => {
    const g = test ? await createTestPasskey().then((t) => ({ id: t.id, publicKey: t.publicKey, jwk: t.jwk, transports: [] as string[] }))
                   : await createPasskey(`constella-guardian-${Date.now()}`, "platform").then((p) => ({ ...p, jwk: undefined }));
    const keys = readJson(GUARDIAN_KEYS, []);
    keys.push({ credentialId: g.id, publicKey: hex(g.publicKey), testKey: g.jwk, transports: g.transports });
    writeJson(GUARDIAN_KEYS, keys);
    const st = await chain.relayer("/pair/join", { code, device_key: hex(g.publicKey), name: "guardian" });
    $("guardJoin").innerHTML = `<p><b>You are a guardian.</b> Check code <b class="mono">${st.check ?? await checkCode(g.publicKey)}</b> - tell the
      account owner; it must match their screen.</p>`;
    return { guardian: true };
  });
  $("guardHere").onclick = () => go(false);
  $("guardTest").onclick = () => go(true);
}

/** New device: a new passkey and a recovery request for the guardians. */
async function startRecovery(kind: "platform" | "test") {
  const account = $<HTMLInputElement>("recAccount").value.trim();
  if (!/^C[A-Z2-7]{55}$/.test(account)) throw new Error("enter the account's C... address");
  const gs = await chain.view(cfg.contracts.recovery_registry, "guardians_of", [sv.addr(account)]);
  if (!gs) throw new Error("this account has no guardians - it cannot be recovered");
  const pk = kind === "test" ? await createTestPasskey().then((t) => ({ id: t.id, publicKey: t.publicKey, jwk: t.jwk as JsonWebKey | undefined, transports: [] as string[], attachment: "test" }))
                             : await createPasskey(`constella-${Date.now()}`, "platform").then((p) => ({ ...p, jwk: undefined }));
  const func = funcB64(account, "propose_recovery", proposalArgs(cfg.contracts.verifier, pk.publicKey));
  const { auth, latest_ledger } = await chain.relayer("/prepare", { func });
  // The account's own entry (the guardians sign as the account, under its recovery rule).
  const mine = (auth as string[]).find((e) => {
    const c: any = (xdr.SorobanAuthorizationEntry.fromXDR(e, "base64") as any).credentials;
    const creds = c.address ?? c.addressV2;
    return creds && Address.fromScAddress(creds.address).toString() === account;
  });
  if (!mine) throw new Error("no authorization to sign");
  const entry = fixExpiry(mine, latest_ledger + 120_960); // guardians have about 7 days
  const req = await chain.relayer("/recovery/request", { account, new_key: hex(pk.publicKey), threshold: Number(gs.threshold), func, entry });
  writeJson(RECOVERY_KEY, { account, id: req.id, credentialId: pk.id, publicKey: hex(pk.publicKey), testKey: pk.jwk,
                            transports: pk.transports, attachment: pk.attachment });
  const link = `${location.origin}${location.pathname}?recover=${req.id}`;
  $("recBox").innerHTML = `<p><b>Send this link to your guardians</b> (${gs.threshold} of ${gs.guardians.length} must approve):<br>
    <span class="mono">${link}</span><br>Tell them this check code: <b class="mono">${await checkCode(pk.publicKey)}</b>.
    After approval, the new passkey takes effect in 48 hours - then click "Finish recovery" here.</p>`;
  return { request: req.id };
}

/** Guardian device (opened with ?recover=ID): approve with a guardian passkey. */
async function showApprove(id: string) {
  try {
    const r = await chain.relayer("/recovery/get", { id });
    const gs = await chain.view(cfg.contracts.recovery_registry, "guardians_of", [sv.addr(r.account)]);
    const mine: GuardianKey[] = readJson(GUARDIAN_KEYS, []).map((k: any) => ({ ...k, publicKey: fromHexStr(k.publicKey) }));
    const usable = mine.filter((k) => gs?.guardians.some((g: any) => hex(new Uint8Array(g)) === hex(k.publicKey)));
    $("approveBox").innerHTML = `<h3>Approve a recovery</h3>
      <p>Account <span class="mono">${r.account}</span> asks to make a NEW passkey its owner. Check code of the new passkey:
      <b class="mono">${await checkCode(fromHexStr(r.new_key))}</b> - confirm it with the owner (call them) before approving.
      Approvals: ${r.approvals.length} of ${r.threshold}.</p>
      ${usable.length ? `<button id="approveRec">Approve with my guardian passkey</button>` : "<p><i>No guardian passkey for this account on this device.</i></p>"}`;
    if (usable.length) {
      $("approveRec").onclick = () => act("approve recovery", async () => {
        const signed = await guardianSign(r.entry, cfg.network.passphrase, Number(gs.rule_id), cfg.contracts.verifier, usable[0]);
        const out = await chain.relayer("/recovery/approve", { id, entry: signed });
        $("approveBox").innerHTML = `<p>Approved (${out.approvals} of ${out.threshold}).${out.submitted ? " The proposal is on chain: it takes effect in 48 hours unless the owner cancels." : ""}</p>`;
        return out;
      });
    }
  } catch (e: any) {
    $("approveBox").textContent = `recovery request: ${e?.message ?? e}`;
  }
}

/** New device, after 48 hours: make the new passkey the owner and sign in. */
async function finishRecovery() {
  const rec = readJson(RECOVERY_KEY, null);
  if (!rec) throw new Error("no recovery started on this device");
  const out = await chain.relayer("/recovery/finish", { account: rec.account });
  saveAccount({ credentialId: rec.credentialId, publicKey: fromHexStr(rec.publicKey), address: rec.account,
                attachment: rec.attachment, transports: rec.transports, testKey: rec.testKey });
  try { localStorage.removeItem(RECOVERY_KEY); } catch { /* */ }
  return out;
}

async function refresh() {
  const c = cfg.contracts;
  try {
    await refreshTargets();
    const V = target();
    const [cons, dep, total, supply, sleeves, paused, hwm, history, eurc] = await Promise.all([
      chain.view(V, "get_nav_conservative"), chain.view(V, "get_nav_deposit"), chain.view(V, "total_assets"),
      chain.view(V, "total_supply"), chain.view(V, "get_sleeves"), chain.view(V, "is_paused"),
      chain.view(V, "get_high_water_mark"), chain.view(V, "get_nav_history"),
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
        <tr><td>vault</td><td><span class="mono">${short(V)}</span> ${V === c.vault ? "(shared Cash Reserve)" : "(your own vault)"} · ${paused ? "PAUSED" : "running"}</td></tr>
      </table>
      <table><tr><th>sleeve</th><th>target</th><th>held (units)</th><th>contract</th></tr>${rows}</table>`;

    const a = loadAccount();
    if (a && !a.address) {
      $("acct").innerHTML = `<b>Passkey saved, account setup unfinished.</b> Click either Create button to finish (no new passkey is created), or "Start over with a new passkey".`;
      $("claims").textContent = "";
    } else if (a) {
      const [usdc, shares, claim] = await Promise.all([
        chain.view(cfg.assets.USDC, "balance", [sv.addr(a.address)]), chain.view(V, "balance", [sv.addr(a.address)]),
        chain.view(V, "get_claim", [sv.addr(a.address), sv.addr(cfg.assets.EURC)]),
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
      if (cfg.contracts.recovery_registry) {
        const gs = await chain.view(cfg.contracts.recovery_registry, "guardians_of", [sv.addr(a.address)]).catch(() => null);
        const pend = await chain.view(a.address, "get_pending_recovery").catch(() => null);
        $("guardians").innerHTML = (gs ? `guardians: <b>${gs.guardians.length}</b>, approvals needed <b>${gs.threshold}</b>` : "no guardians yet - add some so a lost passkey is not a lost account")
          + (pend ? `<br><b style="color:#b00">A recovery is pending</b>: a new owner key takes effect at ${new Date(Number(pend.executable_at) * 1000).toLocaleString()} unless you cancel.` : "");
      }
    } else {
      $("acct").textContent = "no account in this browser yet";
      $("claims").textContent = "";
    }
    if (wallet) {
      const ws = await walletStatus(chain, wallet.address);
      const shares = ws.exists ? await chain.view(V, "balance", [sv.addr(wallet.address)]) : 0;
      const claim = ws.exists ? await chain.view(V, "get_claim", [sv.addr(wallet.address), sv.addr(cfg.assets.EURC)]) : 0;
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
