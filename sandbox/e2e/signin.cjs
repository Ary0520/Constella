/** Sign in with an existing passkey (headless Chromium + virtual platform
 * authenticator): the account is found again from the passkey alone, and a
 * passkey without an account finishes setup. */
const { chromium } = require("playwright");
const t = async (p, s) => ((await p.textContent(s)) || "").replace(/\s+/g, " ").trim();
const done = (p) => p.waitForFunction(() => document.getElementById("status").className !== "busy", null, { timeout: 120000 });
(async () => {
  const b = await chromium.launch(); const p = await b.newPage();
  const cdp = await p.context().newCDPSession(p);
  await cdp.send("WebAuthn.enable");
  await cdp.send("WebAuthn.addVirtualAuthenticator", { options: { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
  await p.goto(process.env.CONSOLE_URL || "http://localhost:5173/"); await p.waitForSelector("#signin");
  // 1. create, remember the address, forget it in this browser
  await p.click("#create"); await done(p);
  const created = (await t(p, "#acct")).match(/account (C[A-Z0-9]{55})/)[1];
  await p.click("#forget"); await p.waitForTimeout(1500);
  // 2. sign in with the same passkey -> same account
  await p.click("#signin"); await done(p);
  const back = ((await t(p, "#acct")).match(/account (C[A-Z0-9]{55})/) || [])[1];
  console.log("SIGN-IN:", await t(p, "#status"), "| same account:", back === created);
  if (back !== created) throw new Error("sign-in did not find the account");
  // 3. it can deposit with the recovered key
  await p.click("#faucet"); await done(p);
  await p.fill("#dep", "20"); await p.click("#deposit"); await done(p);
  console.log("DEPOSIT AFTER SIGN-IN:", await t(p, "#status"));
  if (!/deposit: done/.test(await t(p, "#status"))) throw new Error("deposit after sign-in failed");
  console.log("SIGN-IN E2E PASS"); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
