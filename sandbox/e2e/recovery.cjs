/** Step 8 browser e2e (hosted console + hosted relayer + testnet): three separate
 * browser profiles - OWNER, GUARDIAN, NEW DEVICE.
 *   owner invites a guardian (link) -> guardian creates a guardian passkey ->
 *   owner saves guardians (1-of-1) -> new device requests recovery (link) ->
 *   guardian approves -> the proposal is on chain -> owner sees it pending and
 *   cancels it.
 * The owner account is created on the account factory with a P-256 key made
 * here (the relayer's per-IP account quota is not needed for this test); every
 * step after that goes through the console UI. CONSOLE_URL selects the console. */
const { chromium } = require("playwright");
const { generateKeyPairSync, randomBytes } = require("node:crypto");
const { execFileSync } = require("node:child_process");
const path = require("node:path");
const t = async (p, s) => ((await p.textContent(s)) || "").replace(/\s+/g, " ").trim();
const doneWith = (p, label) => p.waitForFunction((l) => { const s = document.getElementById("status");
  return s.className !== "busy" && s.textContent.includes(l); }, label, { timeout: 240000 });
const ok = async (p, label) => { const s = await t(p, "#status"); console.log(label + ":", s.slice(0, 160)); if (/fail|error/i.test(s)) throw new Error(s); };
const URL0 = process.env.CONSOLE_URL || "http://localhost:5173/";

(async () => {
  // Owner key + account (factory), loaded as the console's test passkey.
  const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const jwk = privateKey.export({ format: "jwk" });
  const pub = Buffer.concat([Buffer.from([4]), Buffer.from(jwk.x, "base64url"), Buffer.from(jwk.y, "base64url")]);
  const root = path.resolve(__dirname, "../..");
  const py = process.env.PY || `${process.env.HOME}/.cache/constella-venv/bin/python`;
  const owner = execFileSync(py, [path.join(root, "testnet/e2e_recovery.py"), "--create-account", pub.toString("hex")]).toString().trim().split("\n").pop();
  console.log("OWNER ACCOUNT:", owner);

  const b = await chromium.launch({ executablePath: process.env.PW_CHROMIUM || undefined });
  const ctx = async () => (await b.newContext()).newPage();
  const O = await ctx(), G = await ctx(), N = await ctx();
  await O.goto(URL0); await O.waitForSelector("#addGuardian");
  await O.evaluate(([a, k, j]) => localStorage.setItem("constella-sandbox-account", JSON.stringify(
    { credentialId: "e2e-owner", publicKey: k, address: a, attachment: "test", testKey: j })), [owner, Array.from(pub), jwk]);
  await O.reload(); await O.waitForSelector("#addGuardian");

  // 1. invite a guardian
  await O.click("#addGuardian");
  await O.waitForFunction(() => /\?guard=/.test(document.getElementById("guardBox").textContent), null, { timeout: 60000 });
  const link = (await t(O, "#guardBox")).match(/https?:\S+\?guard=[A-Z0-9]{8}/)[0];
  console.log("GUARDIAN LINK:", link);
  await G.goto(link); await G.waitForSelector("#guardTest"); await G.click("#guardTest"); await doneWith(G, "become a guardian"); await ok(G, "GUARDIAN");
  const gCheck = (await t(G, "#guardJoin")).match(/code ([0-9A-Z]{6})/)[1];
  await doneWith(O, "add a guardian"); await ok(O, "OWNER SEES GUARDIAN");
  const oCheck = (await t(O, "#guardBox")).match(/code ([0-9A-Z]{6})/)[1];
  if (gCheck !== oCheck) throw new Error(`check codes differ: ${gCheck} vs ${oCheck}`);
  await O.fill("#threshold", "1"); await O.click("#saveGuardians"); await doneWith(O, "save guardians"); await ok(O, "SAVE GUARDIANS");

  // 2. new device requests recovery
  await N.goto(URL0); await N.waitForSelector("#recoverTest");
  await N.fill("#recAccount", owner); await N.click("#recoverTest"); await doneWith(N, "start recovery"); await ok(N, "REQUEST");
  const rlink = (await t(N, "#recBox")).match(/https?:\S+\?recover=[A-Z0-9]{8}/)[0];
  const nCheck = (await t(N, "#recBox")).match(/check code: ([0-9A-Z]{6})/)[1];

  // 3. the guardian approves (sees the same check code)
  await G.goto(rlink); await G.waitForSelector("#approveRec", { timeout: 60000 });
  const shown = (await t(G, "#approveBox")).match(/new passkey: ([0-9A-Z]{6})/)[1];
  if (shown !== nCheck) throw new Error(`guardian sees ${shown}, new device shows ${nCheck}`);
  await G.click("#approveRec"); await doneWith(G, "approve recovery"); await ok(G, "APPROVE");
  if (!/on chain/.test(await t(G, "#approveBox"))) throw new Error("not submitted: " + await t(G, "#approveBox"));

  // 4. the owner sees it pending and cancels
  await O.waitForTimeout(6000); await O.click("#refresh"); await O.waitForFunction(() => /recovery is pending/.test(document.getElementById("guardians").textContent), null, { timeout: 90000 });
  console.log("OWNER:", (await t(O, "#guardians")).slice(0, 200));
  await O.click("#cancelRecovery"); await doneWith(O, "cancel recovery"); await ok(O, "CANCEL");
  await O.waitForTimeout(6000); await O.click("#refresh"); await O.waitForTimeout(8000);
  if (/recovery is pending/.test(await t(O, "#guardians"))) throw new Error("still pending after cancel");
  console.log("RECOVERY BROWSER PASS", owner); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
