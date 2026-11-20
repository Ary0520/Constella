/** Add another device, in two separate browsers (laptop + phone), each with
 * its own virtual platform authenticator: pair, compare check codes, approve,
 * then the NEW device deposits on its own. */
const { chromium } = require("playwright");
const URL = process.env.CONSOLE_URL || "http://localhost:5173/";
const t = async (p, s) => ((await p.textContent(s)) || "").replace(/\s+/g, " ").trim();
const done = (p) => p.waitForFunction(() => document.getElementById("status").className !== "busy", null, { timeout: 180000 });
async function device(browser) {
  const ctx = await browser.newContext(); const p = await ctx.newPage();
  const cdp = await ctx.newCDPSession(p);
  await cdp.send("WebAuthn.enable");
  await cdp.send("WebAuthn.addVirtualAuthenticator", { options: { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
  return p;
}
(async () => {
  const b = await chromium.launch();
  const laptop = await device(b), phone = await device(b);
  await laptop.goto(URL); await laptop.waitForSelector("#create");
  await laptop.click("#create"); await done(laptop);
  const account = (await t(laptop, "#acct")).match(/account (C[A-Z0-9]{55})/)[1];
  await laptop.click("#addDevice");
  await laptop.waitForFunction(() => /pair=/.test(document.getElementById("pairBox").textContent), null, { timeout: 30000 });
  const link = (await t(laptop, "#pairBox")).match(/(https?:\/\/\S+\?pair=[A-Z0-9]+)/)[1];
  await phone.goto(link); await phone.waitForSelector("#joinBtn");
  await phone.click("#joinBtn");
  await phone.waitForFunction(() => /Check code/.test(document.getElementById("joinInfo").textContent), null, { timeout: 60000 });
  const phoneCode = (await t(phone, "#joinInfo")).match(/Check code ([A-F0-9]{6})/)[1];
  await laptop.waitForSelector("#approveDevice", { timeout: 30000 });
  const laptopCode = (await t(laptop, "#pairBox")).match(/Check code: ([A-F0-9]{6})/)[1];
  console.log("check codes match:", phoneCode === laptopCode, phoneCode);
  if (phoneCode !== laptopCode) throw new Error("check codes differ");
  await laptop.click("#approveDevice"); await done(laptop);
  console.log("LAPTOP:", await t(laptop, "#status"));
  await done(phone);
  const phoneAcct = ((await t(phone, "#acct")).match(/account (C[A-Z0-9]{55})/) || [])[1];
  console.log("PHONE uses the same account:", phoneAcct === account, "|", await t(phone, "#devices"));
  if (phoneAcct !== account) throw new Error("phone did not join the account");
  await phone.click("#faucet"); await done(phone);
  await phone.fill("#dep", "20"); await phone.click("#deposit"); await done(phone);
  console.log("PHONE DEPOSIT:", await t(phone, "#status"));
  if (!/deposit: done/.test(await t(phone, "#status"))) throw new Error("phone deposit failed");
  console.log("PAIRING E2E PASS"); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
