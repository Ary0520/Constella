import { friendlyChainError } from "../src/screens/Vault";
it("vault errors read as plain words; unknown ones pass through", () => {
  expect(friendlyChainError("SIMULATION_FAILED: HostError: Error(Contract, #10)")).toMatch(/price is unavailable/);
  expect(friendlyChainError("SIMULATION_FAILED: HostError: Error(Contract, #19)")).toMatch(/per-person limit/);
  expect(friendlyChainError("RATE_LIMITED: once a day")).toMatch(/one top-up a day/);
  expect(friendlyChainError("something else")).toBe("something else");
});
