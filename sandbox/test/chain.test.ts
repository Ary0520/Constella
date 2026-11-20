/** The console encodes calls exactly like the relayer's proven Python code. */
import { funcB64, sv } from "../src/chain";
import vectors from "./func_vectors.json";

describe("host function encoding matches the relayer's Python func_b64", () => {
  for (const v of vectors) {
    it(v.function, () => {
      const a = v.account;
      expect(funcB64(v.contract, v.function, [sv.i128(BigInt(v.amount)), sv.addr(a), sv.addr(a), sv.addr(a)])).toBe(v.func_b64);
    });
  }
});
