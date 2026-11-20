const { chromium } = require("playwright");
const t = async (p, s) => ((await p.textContent(s)) || "").replace(/\s+/g, " ").trim();
const done = (p) => p.waitForFunction(() => document.getElementById("status").className !== "busy", null, { timeout: 90000 });
(async () => {
  const b = await chromium.launch(); const p = await b.newPage();
  await p.goto("http://localhost:5173/"); await p.waitForSelector("#createTest");
  await p.click("#createTest"); await done(p); console.log("CREATE:", await t(p, "#status"));
  await p.click("#faucet"); await done(p);
  await p.fill("#dep", "150"); await p.click("#deposit"); await done(p); console.log("DEPOSIT:", await t(p, "#status"));
  await p.waitForTimeout(3500); console.log("ACCOUNT:", await t(p, "#acct"));
  await b.close();
})();
