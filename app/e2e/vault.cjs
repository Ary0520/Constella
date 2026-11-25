/** Step 10.5 e2e: money in and out of the shared basket with a passkey account
 * (relayer pays, real WebAuthn signatures): test USDC -> deposit 10 -> redeem all. */
const { APP, launch, signedIn, text } = require("./lib.cjs");
(async () => {
  const { b, page, errs } = await launch();
  const account = await signedIn(page);
  console.log("ACCOUNT:", account);
  const cfg = await (await fetch(new URL("/config.json", APP))).json();
  await page.goto(new URL(`/vault/${cfg.contracts.vault}`, APP).href); await page.waitForSelector('[data-act="faucet"]', { timeout: 60000 });
  const settle = async (label) => {
    await page.waitForFunction(() => document.querySelector('[role="status"],[role="alert"]'), null, { timeout: 180000 });
    const out = await text(page, '[role="status"],[role="alert"]');
    console.log(label + ":", out);
    if (await page.$('[role="alert"]')) throw new Error(out);
  };
  await page.click('[data-act="faucet"]'); await settle("FAUCET");
  for (let i = 0; i < 30; i++) {   // the faucet pays asynchronously
    await page.reload(); await page.waitForSelector('[data-pos]', { timeout: 60000 });
    if (!(await text(page, ".c-kvs")).includes("Your USDC$0.00")) break;
    await page.waitForTimeout(4000);
  }
  await page.fill("#dep", "10"); await page.click('[data-act="deposit"]'); await settle("DEPOSIT");
  await page.waitForFunction(() => !/^\$0\.00$/.test(document.querySelector("[data-pos]").textContent.trim()), null, { timeout: 60000 });
  console.log("POSITION AFTER DEPOSIT:", await text(page, "[data-pos]"));
  await page.click('[data-act="redeem-all"]'); await page.click('[data-act="redeem"]'); await settle("REDEEM");
  await page.waitForFunction(() => /^\$0\.00$/.test(document.querySelector("[data-pos]").textContent.trim()), null, { timeout: 60000 });
  if (errs.length) throw new Error("page errors: " + errs.join(" | "));
  console.log("VAULT PASS: deposit + redeem with a passkey, zero XLM"); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
