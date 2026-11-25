/** Step 10.3: the numbers the portfolio shows. Valuation mirrors the vault and
 * the keeper planner exactly (cash at face value; other sleeves at units x
 * oracle price / 10^7, rounded down); weights sum to exactly 100 %. */
import { formatUnits, realisedApr, sleeveView, toBps } from "../src/core/vaults";

const ONE = 10_000_000n;

describe("amounts", () => {
  it("formats 7-decimal units for people", () => {
    expect(formatUnits(1_234_567_890n)).toBe("123.46");
    expect(formatUnits(5n)).toBe("0.0000005");
    expect(formatUnits(0n)).toBe("0.00");
    expect(formatUnits(-25n * ONE)).toBe("-25.00");
    expect(formatUnits(12_345_678n * ONE)).toBe("12,345,678.00");
    expect(formatUnits(1_234_567_890n, 4)).toBe("123.4568");
  });
});

describe("sleeves", () => {
  const sleeves = [
    { asset_id: "USDC", target_weight_bps: 5083, current_balance: 5_374_822n, contract_address: "CUSDC", sleeve_type: 0, band_bps: 200 },
    { asset_id: "EURC", target_weight_bps: 3417, current_balance: 3_257_822n, contract_address: "CEURC", sleeve_type: 2, band_bps: 200 },
    { asset_id: "BLEND", target_weight_bps: 1500, current_balance: 1_500_000n, contract_address: "CBLEND", sleeve_type: 3, band_bps: 200 },
  ];
  it("values cash at face and the rest at the oracle price, weights sum to 10000", () => {
    const v = sleeveView(sleeves, { CEURC: 10_951_674n, CBLEND: ONE }, "CUSDC");
    expect(v.map((s) => s.value)).toEqual([5_374_822n, 3_257_822n * 10_951_674n / ONE, 1_500_000n]);
    expect(v.reduce((n, s) => n + (s.weight_bps ?? 0), 0)).toBe(10000);
    expect(v[1].priced).toBe(true);
    expect(v.every((s) => Math.abs((s.weight_bps ?? -1e9) - s.target_weight_bps) <= s.band_bps)).toBe(true);
  });
  it("an unpriced non-cash holding is flagged, not valued at zero silently", () => {
    const v = sleeveView(sleeves, { CBLEND: ONE }, "CUSDC");
    expect(v[1].priced).toBe(false);
    expect(v[1].value).toBeNull();
    expect(v.every((s) => s.weight_bps === null)).toBe(true);
  });
  it("largest-remainder rounding always totals exactly 10000", () => {
    for (const xs of [[1n, 1n, 1n], [3n, 3n, 4n], [0n, 0n, 7n], [999_999n, 1n, 0n]]) {
      expect(toBps(xs).reduce((a, b) => a + b, 0)).toBe(10000);
    }
    expect(toBps([0n, 0n])).toEqual([0, 0]);
  });
});

describe("realised return", () => {
  it("annualises the NAV change between the first and last points", () => {
    const year = 31_536_000n;
    expect(realisedApr([{ nav: ONE, timestamp: 0n }, { nav: ONE * 102n / 100n, timestamp: year }])).toBeCloseTo(2.0, 6);
    expect(realisedApr([{ nav: ONE, timestamp: 0n }, { nav: ONE * 101n / 100n, timestamp: year / 2n }])).toBeCloseTo(2.0, 6);
  });
  it("needs two points over at least a day, else null", () => {
    expect(realisedApr([])).toBeNull();
    expect(realisedApr([{ nav: ONE, timestamp: 0n }])).toBeNull();
    expect(realisedApr([{ nav: ONE, timestamp: 0n }, { nav: 2n * ONE, timestamp: 3600n }])).toBeNull();
  });
});

describe("typed amounts", () => {
  it("parses what people type into exact 7-decimal units", async () => {
    const { parseUnits } = await import("../src/core/vaults");
    expect(parseUnits("100")).toBe(1_000_000_000n);
    expect(parseUnits(" 1,250.5 ")).toBe(12_505_000_000n);
    expect(parseUnits("0.0000001")).toBe(1n);
    expect(parseUnits(".5")).toBe(5_000_000n);
  });
  it("refuses anything that is not a positive amount with at most 7 decimals", async () => {
    const { parseUnits } = await import("../src/core/vaults");
    for (const bad of ["", "0", "-1", "1e3", "abc", "1.00000001", "1.2.3", "NaN", "Infinity"]) {
      expect(() => parseUnits(bad), bad).toThrow();
    }
  });
});
