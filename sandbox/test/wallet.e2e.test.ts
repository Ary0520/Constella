/**
 * The Stellar-wallet path (classic G account, self-paid, wallet-signed)
 * against the LIVE local stack. Opt-in: SANDBOX_E2E=1. A keypair signer
 * stands in for the wallet popup; every other line is the console's code.
 */
import { readFileSync } from "node:fs";
import { Keypair } from "@stellar/stellar-sdk";
import { Chain, Config, sv } from "../src/chain";
import { addUsdcTrustline, friendbotFund, keypairSigner, walletInvoke, walletStatus } from "../src/wallet";

const run = process.env.SANDBOX_E2E === "1" ? describe : describe.skip;
const ONE = 10_000_000n;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

run("Stellar wallet user on the live local stack", () => {
  const cfg: Config = JSON.parse(readFileSync("public/config.json", "utf8"));
  const chain = new Chain(cfg);
  const kp = Keypair.random();
  const w = keypairSigner(kp.secret(), cfg.network.passphrase);
  const c = cfg.contracts;
  const args = (n: bigint) => [sv.i128(n), sv.addr(w.address), sv.addr(w.address), sv.addr(w.address)];

  it("readiness: unfunded -> friendbot -> no trustline -> trustline -> test USDC", async () => {
    expect((await walletStatus(chain, w.address)).exists).toBe(false);
    await friendbotFund(chain, w.address);
    const s1 = await walletStatus(chain, w.address);
    expect(s1.exists && s1.xlm > 0n && !s1.hasTrustline).toBe(true);
    await addUsdcTrustline(chain, w);
    await chain.invokeAs("issuer", cfg.assets.USDC, "mint", [sv.addr(w.address), sv.i128(1_000n * ONE)]);
    const s2 = await walletStatus(chain, w.address);
    expect(s2.hasTrustline && s2.usdc === 1_000n * ONE).toBe(true);
  }, 120_000);

  it("deposit and redeem signed by the wallet; the user pays the XLM fee", async () => {
    const xlm0 = (await walletStatus(chain, w.address)).xlm;
    const dep = await walletInvoke(chain, w, c.vault, "deposit", args(300n * ONE));
    expect(dep.max_fee_xlm).toBeGreaterThan(0);
    const shares = BigInt(await chain.view(c.vault, "balance", [sv.addr(w.address)]));
    expect(shares).toBeGreaterThan(0n);
    expect((await walletStatus(chain, w.address)).xlm).toBeLessThan(xlm0); // fee paid in XLM by the user
    await walletInvoke(chain, w, c.vault, "redeem", args(50n * ONE));
    expect(BigInt(await chain.view(c.vault, "balance", [sv.addr(w.address)]))).toBe(shares - 50n * ONE);
  }, 120_000);

  it("frozen claim for a classic account is settled by the keeper too", async () => {
    await chain.invokeAs("admin", c.oracle, "trigger_circuit_breaker", [sv.addr(cfg.accounts.admin), sv.sym("CASH"), sv.sym("WALLET")]);
    try {
      await walletInvoke(chain, w, c.vault, "redeem_frozen", args(20n * ONE));
      expect(BigInt(await chain.view(c.vault, "get_claim", [sv.addr(w.address), sv.addr(cfg.assets.EURC)]))).toBeGreaterThan(0n);
    } finally {
      // Always restore prices, or a failed run leaves the shared stack frozen.
      await chain.invokeAs("admin", c.oracle, "clear_circuit_breaker", [sv.sym("CASH")]);
    }
    for (let i = 0; i < 30; i++) {
      if (BigInt(await chain.view(c.vault, "get_claim", [sv.addr(w.address), sv.addr(cfg.assets.EURC)])) === 0n) return;
      await sleep(3000);
    }
    throw new Error("the keeper did not settle the classic account's claim");
  }, 180_000);
});
