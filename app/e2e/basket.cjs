/** Step 10.4 e2e: thesis -> draft -> review -> ONE passkey signature -> the
 * user's own live basket. Checks the Rule 10 disclaimer is shown verbatim and
 * that Create stays disabled until the review is confirmed. */
const { APP, launch, signedIn, text } = require("./lib.cjs");
const RULE10 = "This simulation uses historical data. Past performance does not predict future returns. This is not financial advice.";
(async () => {
  const { b, page, errs } = await launch();
  const account = await signedIn(page);
  console.log("ACCOUNT:", account);
  await page.goto(new URL("/new", APP).href); await page.waitForSelector("#thesis");
  await page.fill("#thesis", "Keep 18 months of runway safe, hold a third in euro, at least 10% instant cash. Up to 40% per issuer and 40% per asset.");
  await page.click('[data-act="draft"]');
  await page.waitForSelector('[data-act="create"], [data-refused], [role="alert"]', { timeout: 180000 });
  if (await page.$('[role="alert"]')) throw new Error(await text(page, '[role="alert"]'));
  if (await page.$("[data-refused]")) throw new Error("refused: " + await text(page, "[data-refused]"));
  const disc = await text(page, "[data-disclaimer]");
  if (disc !== RULE10) throw new Error(`disclaimer differs: ${disc}`);
  console.log("REVIEW:", (await text(page, ".c-main")).slice(0, 260));
  if (!(await page.isDisabled('[data-act="create"]'))) throw new Error("create is enabled before the review is confirmed");
  await page.check('[data-act="confirm"]'); await page.click('[data-act="create"]');
  await page.waitForURL(/\/vault\/C[A-Z2-7]{55}$/, { timeout: 240000 });
  await page.waitForSelector("[data-pos]", { timeout: 90000 });
  console.log("CREATED:", page.url(), "|", (await text(page, "h1")));
  if (errs.length) throw new Error("page errors: " + errs.join(" | "));
  console.log("BASKET PASS"); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
