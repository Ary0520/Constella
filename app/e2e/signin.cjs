/** Step 10.2 e2e: "I already have a passkey - sign in" through the REAL WebAuthn
 * path. Chromium's virtual authenticator holds a discoverable passkey whose
 * P-256 key we create here; its account is created on the account factory
 * (testnet/e2e_recovery.py --create-account; no relayer quota needed). The app
 * then discovers the passkey, recovers its public key from the signature and
 * finds the account on chain - exactly as on a real device.
 * APP_URL (default http://localhost:4173), PW_CHROMIUM, PY. */
const { chromium } = require("playwright");
const { generateKeyPairSync, randomBytes } = require("node:crypto");
const { execFileSync } = require("node:child_process");
const path = require("node:path");

const APP = process.env.APP_URL || "http://localhost:4173/";

async function virtualPasskey(page, rpId, privateKey) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", { options: {
    protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
  await cdp.send("WebAuthn.addCredential", { authenticatorId, credential: {
    credentialId: randomBytes(16).toString("base64"), isResidentCredential: true, rpId,
    privateKey: privateKey.export({ format: "der", type: "pkcs8" }).toString("base64"),
    userHandle: randomBytes(16).toString("base64"), signCount: 0 } });
  return { cdp, authenticatorId };
}

(async () => {
  const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const jwk = privateKey.export({ format: "jwk" });
  const pub = Buffer.concat([Buffer.from([4]), Buffer.from(jwk.x, "base64url"), Buffer.from(jwk.y, "base64url")]);
  const root = path.resolve(__dirname, "../..");
  const py = process.env.PY || `${process.env.HOME}/.cache/constella-venv/bin/python`;
  const account = execFileSync(py, [path.join(root, "testnet/e2e_recovery.py"), "--create-account", pub.toString("hex")]).toString().trim().split("\n").pop();
  console.log("ACCOUNT:", account);

  const b = await chromium.launch({ executablePath: process.env.PW_CHROMIUM || undefined });
  const page = await (await b.newContext()).newPage();
  const errs = []; page.on("pageerror", (e) => errs.push(e.message));
  await page.goto(APP); await page.waitForSelector('[data-way="signin"]');
  await virtualPasskey(page, new URL(APP).hostname, privateKey);
  await page.click('[data-way="signin"]');
  await page.waitForSelector(".c-who", { timeout: 120000 });
  const who = (await page.textContent(".c-who")).replace(/\s+/g, " ");
  console.log("SIGNED IN AS:", who);
  if (!who.includes(account.slice(-6))) throw new Error(`shows ${who}, expected ${account}`);
  await page.reload(); await page.waitForSelector(".c-who", { timeout: 60000 });
  console.log("STILL SIGNED IN AFTER RELOAD");
  if (errs.length) throw new Error("page errors: " + errs.join(" | "));
  console.log("SIGN-IN PASS"); await b.close();
})().catch((e) => { console.error("FAIL:", e.message); process.exit(1); });
