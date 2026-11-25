/** People-facing names for registry asset ids and sleeve types. */
const NAMES: Record<string, string> = {
  USDC: "US dollar cash (USDC)",
  EURC: "Euro cash (EURC)",
  BLEND_USDC_SUPPLY: "USDC lending on Blend",
  BENJI: "Franklin OnChain US Gov. Money Fund (BENJI)",
  USDY: "Ondo US Dollar Yield (USDY)",
  SPIKO_USTBL: "Spiko US T-Bills",
  SPIKO_EUTBL: "Spiko Euro T-Bills",
};
export const assetName = (id: string) => NAMES[id] ?? id.replace(/_/g, " ");
export const COLORS = ["#14121C", "#6A5CE0", "#B9B0F2", "#2F9E6E", "#B7791F", "#A19EB0"];
