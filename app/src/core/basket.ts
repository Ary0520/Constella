/**
 * Step 10.4: thesis -> draft (thesis API) -> review -> the user's own basket
 * with ONE signature (`vault_factory.create_vault_with_policy`: deploys the
 * private vault, binds this exact policy with the user as owner, sets the
 * registry sleeves, Constella's keeper and router). The AI never submits:
 * the person reviews and signs (spec OVERVIEW: no auto-submit).
 */
import { Chain, Config, sv } from "./chain";
import { createVaultArgs, type OwnVaultInput } from "./ownvault";
import type { Actor } from "./accounts";

export const CAP_UNITS = 25_000n * 10_000_000n; // testnet private-vault cap (TVL = per-user)

export async function draftThesis(cfg: Config, text: string, owner: string | null, opts: { jurisdiction?: string; investor_class?: string } = {}) {
  const r = await fetch(cfg.services.thesis_api + "/thesis", {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text, jurisdiction: opts.jurisdiction ?? "DE", investor_class: opts.investor_class ?? "retail", use_llm: true,
                           ...(owner ? { owner } : {}) }),
  });
  const out = await r.json();
  if (!r.ok) {
    const reasons = Array.isArray(out.reasons) ? out.reasons.join(" ") : out.detail ?? "";
    throw new Error(`${out.error ?? "The engine refused this thesis"}${reasons ? `: ${reasons}` : ""}`);
  }
  return out;
}

export function symbolFor(name: string): string {
  return name.replace(/[^A-Za-z]/g, "").slice(0, 8).toUpperCase() || "BASKET";
}

export function draftToVaultInput(d: any, owner: string, feeBps: number, salt: Uint8Array): OwnVaultInput {
  if (!d?.signable) throw new Error("This is a preview. Sign in so the draft is made for your account.");
  if (d.policy?.admin?.admin_address !== owner) throw new Error("This draft was made for another account. Draft it again.");
  if (!/^[0-9a-f]{64}$/.test(String(d.policy_hash))) throw new Error("The draft's policy hash is malformed.");
  const name = String(d.policy?.basket_name || "My basket");
  return { owner, numeraire: d.numeraire_contract ?? "", feeBps, capUnits: CAP_UNITS, name, symbol: symbolFor(name), salt,
           bind: d.bind_policy, policyHashHex: d.policy_hash };
}

/** One signature; returns the new vault's address. */
export async function createBasket(chain: Chain, actor: Actor, draft: any): Promise<string> {
  const f = chain.cfg.contracts.vault_factory;
  const settings = await chain.view(f, "get_settings");
  const input = draftToVaultInput(draft, actor.address, Number(settings.default_fee_bps), crypto.getRandomValues(new Uint8Array(32)));
  if (!input.numeraire) input.numeraire = chain.cfg.assets.USDC;
  const before: string[] = (await chain.view(f, "vaults_of", [sv.addr(actor.address)]).catch(() => [])).map(String);
  await actor.call(f, "create_vault_with_policy", createVaultArgs(input));
  const after: string[] = (await chain.view(f, "vaults_of", [sv.addr(actor.address)])).map(String);
  const made = after.find((v) => !before.includes(v));
  if (!made) throw new Error("The basket was not created. Nothing was charged; try again.");
  return made;
}
