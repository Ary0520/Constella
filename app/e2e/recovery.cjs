/** Step 10.6 e2e: guardians + recovery across three browser profiles, each with
 * its own virtual authenticator (real passkey creation and signing). */
const { chromium } = require("playwright");
const { APP, virtualPasskey, signedIn, text } = require("./lib.cjs");
const status = async (p, label) => {
  await p.waitForFunction(() => document.querySelector('[role="status"],[role="alert"]'), null, { timeout: 240000 });
  const t = await text(p, '[role="status"],[role="alert"]'); console.log(label + ":", t.slice(0, 140));
  if (await p.$('[role="alert"]')) throw new Error(t);
  return t;
};
(async () => {
  const b = await chromium.launch({ executablePath: process.env.PW_CHROMIUM || undefined });
  const ctx = async () => { const p = await (await b.newContext()).newPage(); const errs = []; p.on("pageerror", (e) => errs.push(e.message)); p.errs = errs; return p; };
  const O = await ctx(), G = await ctx(), N = await ctx();
  const account = await signedIn(O); console.log("OWNER:", account);
  await O.goto(new URL("/account", APP).href); await O.waitForSelector('[data-act="invite-guardian"]', { timeout: 60000 });
  await O.click('[data-act="invite-guardian"]'); await O.waitForSelector("[data-guard-link]");
  const link = (await text(O, "[data-guard-link]")).match(/https?:\S+\?guard=[A-Z0-9]{8}/)[0]; console.log("GUARDIAN LINK:", link);
  await G.goto(APP); await virtualPasskey(G); await G.goto(link); await G.click('[data-act="become-guardian"]');
  await G.waitForSelector("[data-check]", { timeout: 120000 });
  const gCheck = (await text(G, "[data-check]")).match(/code ([0-9A-F]{6})/)[1];
  const oMsg = await status(O, "OWNER SEES GUARDIAN");
  if (!oMsg.includes(gCheck)) throw new Error(`check codes differ: guardian ${gCheck} / owner "${oMsg}"`);
  await O.click('[data-act="save-guardians"]'); await O.waitForFunction(() => /Guardians saved/.test(document.body.textContent), null, { timeout: 240000 });
  console.log("GUARDIANS SAVED");

  await N.goto(APP); await virtualPasskey(N); await N.click('[data-way="lost"]'); await N.fill("#recAccount", account);
  await N.click('[data-act="start-recovery"]'); await N.waitForSelector("[data-request]", { timeout: 180000 });
  const req = await text(N, "[data-request]");
  const rlink = req.match(/https?:\S+\?recover=[A-Z0-9]{8}/)[0], nCheck = req.match(/check code ([0-9A-F]{6})/)[1];
  console.log("RECOVERY LINK:", rlink);
  await G.goto(rlink); await G.waitForSelector('[data-act="approve-recovery"]', { timeout: 90000 });
  const shown = await text(G, "b[data-check]");
  if (shown !== nCheck) throw new Error(`guardian sees ${shown}, new device shows ${nCheck}`);
  await G.click('[data-act="approve-recovery"]'); const g = await status(G, "APPROVE");
  if (!/on chain/.test(g)) throw new Error("not submitted");

  for (let i = 0; i < 12 && !(await O.$("[data-pending]")); i++) { await O.reload(); await O.waitForSelector("[data-guardians]"); await O.waitForTimeout(5000); }
  console.log("OWNER:", (await text(O, "[data-pending]")).slice(0, 120));
  await O.click('[data-act="cancel-recovery"]'); await status(O, "CANCEL");
  for (const p of [O, G, N]) if (p.errs.length) throw new Error("page errors: " + p.errs.join(" | "));
  console.log("RECOVERY PASS", account); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
