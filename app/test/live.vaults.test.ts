/** Opt-in (LIVE=1): read real testnet vaults with the app's own code. */
import { readFileSync } from "node:fs";
import { Chain } from "../src/core/chain";
import { listBaskets, readActivity, readVault } from "../src/core/vaults";

const live = process.env.LIVE === "1" ? describe : describe.skip;
live("live testnet reads", () => {
  const chain = new Chain(JSON.parse(readFileSync("config.testnet.json", "utf8")));
  it("the 3-asset vault values every sleeve and its weights sum to 100 %", async () => {
    const v = await readVault(chain, "CDAJDSXBYMHJMGL35WXL73MDRLPLFOSEM4MZ4VZRWNY6F7XMFGAP4QQS");
    expect(v.sleeves.map((s) => s.asset_id).sort()).toEqual(["BLEND_USDC_SUPPLY", "EURC", "USDC"]);
    expect(v.sleeves.every((s) => s.priced)).toBe(true);
    expect(v.sleeves.reduce((n, s) => n + (s.weight_bps ?? 0), 0)).toBe(10000);
    expect(v.navConservative).not.toBeNull();
  }, 60000);
  it("shows today's keeper rebalance in the activity", async () => {
    const a = await readActivity(chain, "CDAJDSXBYMHJMGL35WXL73MDRLPLFOSEM4MZ4VZRWNY6F7XMFGAP4QQS");
    expect(a.some((x) => x.label === "Keeper rebalanced")).toBe(true);
    expect(a.some((x) => x.label === "Deposit")).toBe(true);
  }, 60000);
  it("lists the shared Cash Reserve first", async () => {
    const b = await listBaskets(chain, null);
    expect(b[0]).toEqual({ address: chain.cfg.contracts.vault, shared: true });
  }, 60000);
});
