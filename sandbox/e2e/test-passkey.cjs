/** Browser check of the in-browser TEST passkey (local testing only): create,
 * faucet, sponsored deposit - no authenticator involved. */
const { chromium } = require("playwright");
const t = async (p, s) => ((await p.textContent(s)) || "").replace(/\s+/g, " ").trim();
const done = (p) => p.waitForFunction(() => document.getElementById("status").className !== "busy", null, { timeout: 90000 });
(async () => {
  const b = await chromium.launch(); const p = await b.newPage();
  await p.goto(process.env.CONSOLE_URL || "http://localhost:5173/"); await p.waitForSelector("#createTest");
  await p.click("#createTest"); await done(p); console.log("CREATE:", await t(p, "#status"));
  await p.click("#faucet"); await done(p);
  await p.fill("#dep", "50"); await p.click("#deposit"); await done(p); console.log("DEPOSIT:", await t(p, "#status"));
  await p.waitForTimeout(3500); const acct = await t(p, "#acct"); console.log("ACCOUNT:", acct);
  if (!/vault shares (?!0\.0000)/.test(acct)) throw new Error("no shares after deposit");
  console.log("TEST PASSKEY BROWSER PASS"); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
