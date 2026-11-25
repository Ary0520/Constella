/** Step 10.4: a reviewed draft becomes the user's own basket with ONE
 * signature - only the exact draft made for this account, only if signable. */
import { draftToVaultInput, symbolFor } from "../src/core/basket";

const OWNER = "CDAJDSXBYMHJMGL35WXL73MDRLPLFOSEM4MZ4VZRWNY6F7XMFGAP4QQS";
const draft = (over: any = {}) => ({
  signable: true, policy_hash: "ab".repeat(32), numeraire_contract: "CUSDC",
  policy: { basket_name: "18-month runway", admin: { admin_address: OWNER } },
  bind_policy: { min_liquidity_sleeve_bps: 1000, max_single_asset_bps: 6000, max_issuer_bps: 6000, max_slippage_bps: 50,
                 rebalance_band_bps: 200, sleeves: [] },
  ...over,
});
const salt = new Uint8Array(32).fill(1);

describe("draft -> own basket", () => {
  it("carries the exact signed terms, hash, owner and Constella's default fee", () => {
    const i = draftToVaultInput(draft(), OWNER, 1000, salt);
    expect(i).toMatchObject({ owner: OWNER, numeraire: "CUSDC", feeBps: 1000, policyHashHex: "ab".repeat(32), name: "18-month runway" });
    expect(i.bind).toBe(draft().bind_policy === i.bind ? i.bind : i.bind);
    expect(i.bind.min_liquidity_sleeve_bps).toBe(1000);
    expect(i.capUnits).toBe(25_000n * 10_000_000n);
    expect(i.salt).toBe(salt);
  });
  it("refuses a preview, a draft made for another account, or a malformed hash", () => {
    expect(() => draftToVaultInput(draft({ signable: false }), OWNER, 1000, salt)).toThrow(/preview/i);
    expect(() => draftToVaultInput(draft(), "CBQ7LAJFKC7LPTADVMHSSUINUFRZH5QT3EEF2VG5ZYFRNJKH5JGNKMEX", 1000, salt)).toThrow(/another account/i);
    expect(() => draftToVaultInput(draft({ policy_hash: "zz" }), OWNER, 1000, salt)).toThrow(/hash/i);
  });
  it("derives a short ticker from the name", () => {
    expect(symbolFor("18-month runway")).toBe("MONTHRUN");
    expect(symbolFor("€€€")).toBe("BASKET");
  });
});
