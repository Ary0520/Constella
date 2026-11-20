/** The console runs in a browser: Vite does not polyfill Node's Buffer, and
 * @types/node (for these tests) would hide such a use from tsc. */
import { readFileSync, readdirSync } from "node:fs";

it("src/ uses no Node-only globals", () => {
  for (const f of readdirSync("src")) {
    const code = readFileSync(`src/${f}`, "utf8").replace(/\/\/.*$/gm, "");
    expect(code, f).not.toMatch(/\bBuffer\b|\brequire\(|\bprocess\./);
  }
});
