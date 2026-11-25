/** Step 10.6 e2e: add a second device to an account (two browser profiles). */
const { chromium } = require("playwright");
const { APP, virtualPasskey, signedIn, text } = require("./lib.cjs");
(async () => {
  const b = await chromium.launch({ executablePath: process.env.PW_CHROMIUM || undefined });
  const O = await (await b.newContext()).newPage(), D = await (await b.newContext()).newPage();
  const account = await signedIn(O); console.log("OWNER:", account);
  await O.goto(new URL("/account", APP).href); await O.waitForSelector('[data-act="add-device"]', { timeout: 60000 });
  await O.click('[data-act="add-device"]'); await O.waitForFunction(() => /\?pair=/.test(document.body.textContent), null, { timeout: 60000 });
  const link = (await text(O, ".c-main")).match(/https?:\S+\?pair=[A-Z0-9]{8}/)[0]; console.log("PAIR LINK:", link);
  await D.goto(APP); await virtualPasskey(D); await D.goto(link); await D.click('[data-act="join"]');
  await D.waitForSelector("[data-check]", { timeout: 120000 });
  const dCheck = (await text(D, "[data-check]")).match(/code ([0-9A-F]{6})/)[1];
  await O.waitForSelector("[data-approve]", { timeout: 120000 });
  const oCheck = (await text(O, "[data-approve]")).match(/code ([0-9A-F]{6})/)[1];
  if (dCheck !== oCheck) throw new Error(`check codes differ ${dCheck} / ${oCheck}`);
  await O.click('[data-act="approve-device"]');
  await D.waitForSelector(".c-who", { timeout: 240000 });
  const who = await text(D, ".c-who"); console.log("NEW DEVICE:", who);
  if (!who.includes(account.slice(-6))) throw new Error("new device not signed in to the account");
  console.log("PAIRING PASS"); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
