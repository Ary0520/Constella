/** The app runs in a browser: Vite does not polyfill Node's Buffer, and
 * @types/node (for these tests) would hide such a use from tsc. */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const files = (d: string): string[] =>
  readdirSync(d).flatMap((f) => (statSync(join(d, f)).isDirectory() ? files(join(d, f)) : [join(d, f)]));

it("src/ uses no Node-only globals", () => {
  const all = files("src").filter((f) => /\.(ts|tsx)$/.test(f));
  expect(all.length).toBeGreaterThan(4);
  for (const f of all) {
    const code = readFileSync(f, "utf8").replace(/\/\/.*$/gm, "");
    expect(code, f).not.toMatch(/\bBuffer\b|\brequire\(|\bprocess\./);
  }
});
