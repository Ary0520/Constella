/** Step 3 browser e2e: a passkey account (relayer pays, zero XLM) drafts a
 * thesis, creates its OWN vault from the signed policy with one signature,
 * and deposits into it. Uses the in-browser TEST passkey (same signature
 * format the account checks). CONSOLE_URL selects the console. */
const { chromium } = require("playwright");
const t = async (p, s) => ((await p.textContent(s)) || "").replace(/\s+/g, " ").trim();
// Wait until THIS action finished: its label is in the status line and it is no longer busy.
const doneWith = (p, label) => p.waitForFunction((l) => { const s = document.getElementById("status");
  return s.className !== "busy" && s.textContent.includes(l); }, label, { timeout: 180000 });
const done = (p) => p.waitForFunction(() => document.getElementById("status").className !== "busy", null, { timeout: 180000 });
const ok = async (p, label) => { const s = await t(p, "#status"); console.log(label + ":", s); if (/fail|error/i.test(s)) throw new Error(s); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.PW_CHROMIUM || undefined }); const p = await b.newPage();
  await p.goto(process.env.CONSOLE_URL || "http://localhost:5173/"); await p.waitForSelector("#createTest");
  await p.click("#createTest"); await doneWith(p, "create test passkey account"); await ok(p, "CREATE ACCOUNT");
  await p.click("#faucet"); await doneWith(p, "faucet"); await ok(p, "FAUCET");
  await p.click("#run"); await p.waitForSelector("#mkVault", { timeout: 120000 });
  const draft = await t(p, "#thesisOut"); console.log("DRAFT:", draft.slice(0, 160));
  // Step 7: the simulation (worst case first) and the Rule 10 disclaimer come BEFORE the create button.
  for (const must of ["Simulation (before any money moves)", "Worst case", "Issuer freeze", "Depeg",
                      "Past performance does not predict future returns", "Your thesis vs this basket"]) {
    if (!draft.includes(must)) throw new Error(`draft is missing: ${must}`);
  }
  await p.click("#mkVault"); await doneWith(p, "create my vault"); await ok(p, "CREATE MY VAULT");
  const sel = await p.$eval("#target", (s) => ({ value: s.value, label: s.options[s.selectedIndex].text }));
  console.log("SELECTED:", sel.label);
  if (!/My vault/.test(sel.label)) throw new Error("own vault not selected after creation");
  await p.fill("#dep", "50"); await p.click("#deposit"); await doneWith(p, "deposit"); await ok(p, "DEPOSIT INTO MY VAULT");
  await p.waitForTimeout(4000); await p.click("#refresh"); await p.waitForTimeout(6000);
  const acct = await t(p, "#acct"), ov = await t(p, "#overview");
  console.log("ACCOUNT:", acct.slice(0, 200)); console.log("OVERVIEW:", ov.slice(0, 240));
  if (!/your own vault/.test(ov)) throw new Error("overview is not showing the own vault");
  if (!/vault shares (?!0\.0000)/.test(acct)) throw new Error("no shares in the own vault after deposit");
  console.log("OWN VAULT BROWSER PASS", sel.value); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
