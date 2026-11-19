/**
 * Real-browser test of the console (headless Chromium + a virtual WebAuthn
 * platform authenticator via the DevTools protocol), against localnet/up.sh
 * and `npm run dev`. Run: npx playwright install chromium && npm run test:browser
 * Covers the first real test's failure (relayer request interrupted -> the
 * passkey is kept -> "Finish setup" completes without a new passkey), then
 * faucet, a passkey-signed sponsored deposit, and the keeper's rebalance.
 */
const { chromium } = require("playwright");
const text = async (p, sel) => ((await p.textContent(sel)) || "").replace(/\s+/g, " ").trim();
(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  page.on("pageerror", (e) => console.log("PAGEERROR", e.message));
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", { options: { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
  await page.goto("http://localhost:5173/");
  await page.waitForSelector("#create");
  // 1. The relayer request is interrupted (as in the first real test).
  let dropped = false;
  await page.route("**/accounts", (r) => { if (!dropped && r.request().method() === "POST") { dropped = true; return r.abort(); } return r.continue(); });
  await page.click("#create");
  await page.waitForFunction(() => document.getElementById("status").className === "err", null, { timeout: 30000 });
  console.log("1 STATUS:", await text(page, "#status"));
  await page.reload(); await page.waitForSelector("#create"); await page.waitForTimeout(3000);
  console.log("1 AFTER RELOAD:", await text(page, "#acct"));
  const creds1 = (await cdp.send("WebAuthn.getCredentials", { authenticatorId })).credentials.length;
  await page.click("#create");
  await page.waitForFunction(() => document.getElementById("status").className !== "busy", null, { timeout: 60000 });
  const creds2 = (await cdp.send("WebAuthn.getCredentials", { authenticatorId })).credentials.length;
  if (creds1 !== creds2) throw new Error("finishing setup must not create a new passkey");
  console.log("1 FINISH:", await text(page, "#status"), "| passkeys before/after finish:", creds1, creds2);
  console.log("1 ACCOUNT:", await text(page, "#acct"));
  // 2. Faucet, then a passkey-signed deposit, then the keeper.
  await page.click("#faucet");
  await page.waitForFunction(() => document.getElementById("status").className !== "busy", null, { timeout: 60000 });
  console.log("2 FAUCET:", await text(page, "#status"), "|", await text(page, "#acct"));
  await page.fill("#dep", "200");
  await page.click("#deposit");
  await page.waitForFunction(() => document.getElementById("status").className !== "busy", null, { timeout: 90000 });
  console.log("2 DEPOSIT:", await text(page, "#status"), "|", await text(page, "#acct"));
  await page.waitForTimeout(25000); // the keeper rebalances within ~20 s
  console.log("3 AFTER 25s:", await text(page, "#acct"));
  console.log("3 OVERVIEW:", (await text(page, "#overview")).slice(0, 400));
  const acct = await text(page, "#acct");
  if (!/vault shares (?!0\.0000)/.test(acct)) throw new Error("deposit did not produce vault shares: " + acct);
  console.log("BROWSER E2E PASS");
  await browser.close();
})().catch((e) => { console.error("BROWSER E2E FAIL:", e.message); process.exit(1); });
