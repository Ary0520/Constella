/** Step 10.2 e2e: "I already have a passkey - sign in" through the REAL WebAuthn
 * path (virtual authenticator holding a passkey whose key we made; its account
 * created on the factory). APP_URL, PW_CHROMIUM, PY. */
const { launch, signedIn, text } = require("./lib.cjs");
(async () => {
  const { b, page, errs } = await launch();
  const account = await signedIn(page);
  const who = await text(page, ".c-who");
  console.log("ACCOUNT:", account, "| SHOWN:", who);
  if (!who.includes(account.slice(-6))) throw new Error(`shows ${who}, expected ${account}`);
  await page.reload(); await page.waitForSelector(".c-who", { timeout: 60000 });
  if (errs.length) throw new Error("page errors: " + errs.join(" | "));
  console.log("SIGN-IN PASS (also after reload)"); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
