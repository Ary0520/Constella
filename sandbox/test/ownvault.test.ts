/** Step 3: arguments for vault_factory.create_vault_with_policy, built from the
 * thesis API's signable response. Soroban decodes a contracttype struct from
 * an ScMap whose symbol keys are in sorted order, field names exactly as in
 * the contract (VaultParams, PolicyTerms, PolicySleeveTarget). */
import { describe, expect, it } from "vitest";
import { scValToNative, StrKey, xdr } from "@stellar/stellar-sdk";
import { createVaultArgs } from "../src/ownvault";

const OWNER = StrKey.encodeContract(new Uint8Array(32).fill(3) as any);
const USDC = StrKey.encodeContract(new Uint8Array(32).fill(4) as any);
const bind = {
  min_liquidity_sleeve_bps: 1000, max_single_asset_bps: 2500, max_issuer_bps: 3000, max_slippage_bps: 50, rebalance_band_bps: 200,
  sleeves: [{ asset_id: "BLEND_USDC_SUPPLY", target_weight_bps: 1500, min_band_bps: 1300, max_band_bps: 1700, sleeve_type: "lending" },
            { asset_id: "USDC", target_weight_bps: 8500, min_band_bps: 8300, max_band_bps: 8700, sleeve_type: "numeraire" }],
};
const HASH = "ab".repeat(32);

const keys = (v: xdr.ScVal) => v.map()!.map((e) => e.key().sym().toString());

describe("create_vault_with_policy arguments", () => {
  const args = createVaultArgs({ owner: OWNER, numeraire: USDC, feeBps: 1000, capUnits: 25_000n * 10_000_000n,
                                 name: "cRUNWAY", symbol: "CRUN", salt: new Uint8Array(32).fill(9), bind, policyHashHex: HASH });

  it("is (creator, params, terms) with the creator first", () => {
    expect(args).toHaveLength(3);
    expect(scValToNative(args[0])).toBe(OWNER);
  });

  it("encodes VaultParams with sorted keys and the creator as admin and pause authority", () => {
    expect(keys(args[1])).toEqual(["admin", "name", "numeraire", "pause_authority", "per_user_cap", "performance_fee_bps", "salt", "symbol", "tvl_cap"]);
    const p = scValToNative(args[1]);
    expect([p.admin, p.pause_authority, p.numeraire, p.performance_fee_bps]).toEqual([OWNER, OWNER, USDC, 1000]);
    expect([p.tvl_cap, p.per_user_cap]).toEqual([250_000_000_000n, 250_000_000_000n]);
    expect(args[1].map()!.find((e) => e.key().sym().toString() === "performance_fee_bps")!.val().switch().name).toBe("scvU32");
  });

  it("encodes PolicyTerms exactly from the signed bind_policy terms", () => {
    expect(keys(args[2])).toEqual(["max_issuer_bps", "max_single_asset_bps", "max_slippage_bps", "min_liquidity_sleeve_bps",
                                   "policy_hash", "rebalance_band_bps", "sleeves"]);
    const t = scValToNative(args[2]);
    expect(Buffer.from(t.policy_hash).toString("hex")).toBe(HASH);
    expect([t.min_liquidity_sleeve_bps, t.max_single_asset_bps, t.max_issuer_bps, t.max_slippage_bps, t.rebalance_band_bps])
      .toEqual([1000, 2500, 3000, 50, 200]);
    const sleeves = args[2].map()!.find((e) => e.key().sym().toString() === "sleeves")!.val().vec()!;
    expect(keys(sleeves[0])).toEqual(["asset_id", "max_band_bps", "min_band_bps", "sleeve_type", "target_weight_bps"]);
    expect(sleeves[1].map()!.find((e) => e.key().sym().toString() === "asset_id")!.val().switch().name).toBe("scvSymbol");
    expect(t.sleeves.map((s: any) => [s.asset_id, s.target_weight_bps, s.sleeve_type]))
      .toEqual([["BLEND_USDC_SUPPLY", 1500, "lending"], ["USDC", 8500, "numeraire"]]);
  });

  it("refuses a policy hash that is not 32 bytes", () => {
    expect(() => createVaultArgs({ owner: OWNER, numeraire: USDC, feeBps: 1000, capUnits: 1n, name: "x", symbol: "X",
                                   salt: new Uint8Array(32), bind, policyHashHex: "ab" })).toThrow();
  });
});
