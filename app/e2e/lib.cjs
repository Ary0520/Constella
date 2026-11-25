/** Shared e2e helpers: a P-256 passkey we control, its account on the factory,
 * and Chromium's virtual authenticator holding it (the REAL WebAuthn path). */
const { chromium } = require("playwright");
const { generateKeyPairSync, randomBytes } = require("node:crypto");
const { execFileSync } = require("node:child_process");
const path = require("node:path");

const APP = process.env.APP_URL || "http://localhost:4173/";
const root = path.resolve(__dirname, "../..");
const py = process.env.PY || `${process.env.HOME}/.cache/constella-venv/bin/python`;

function newKey() {
  const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const jwk = privateKey.export({ format: "jwk" });
  const pub = Buffer.concat([Buffer.from([4]), Buffer.from(jwk.x, "base64url"), Buffer.from(jwk.y, "base64url")]);
  return { privateKey, pub };
}

function createAccount(pub) {
  return execFileSync(py, [path.join(root, "testnet/e2e_recovery.py"), "--create-account", pub.toString("hex")]).toString().trim().split("\n").pop();
}

async function virtualPasskey(page, privateKey) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", { options: {
    protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
  if (privateKey) {
    await cdp.send("WebAuthn.addCredential", { authenticatorId, credential: {
      credentialId: randomBytes(16).toString("base64"), isResidentCredential: true, rpId: new URL(APP).hostname,
      privateKey: privateKey.export({ format: "der", type: "pkcs8" }).toString("base64"),
      userHandle: randomBytes(16).toString("base64"), signCount: 0 } });
  }
  return { cdp, authenticatorId };
}

async function launch() {
  const b = await chromium.launch({ executablePath: process.env.PW_CHROMIUM || undefined });
  const page = await (await b.newContext()).newPage();
  const errs = []; page.on("pageerror", (e) => errs.push(e.message));
  return { b, page, errs };
}

/** New account + sign in through the welcome screen. */
async function signedIn(page) {
  const { privateKey, pub } = newKey();
  const account = createAccount(pub);
  await page.goto(APP); await page.waitForSelector('[data-way="signin"]');
  await virtualPasskey(page, privateKey);
  await page.click('[data-way="signin"]');
  await page.waitForSelector(".c-who", { timeout: 120000 });
  return account;
}

const text = async (page, sel) => ((await page.textContent(sel)) || "").replace(/\s+/g, " ").trim();
module.exports = { APP, newKey, createAccount, virtualPasskey, launch, signedIn, text };
