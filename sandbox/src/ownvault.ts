/**
 * Step 3: arguments for `vault_factory.create_vault_with_policy(creator, params, terms)`
 * built from the thesis API's signable response (`bind_policy` terms and the
 * policy hash, OPEN-ISSUES #3). A Soroban contracttype struct is an ScMap
 * whose symbol keys are sorted; field names match the contract exactly
 * (VaultParams, PolicyTerms, PolicySleeveTarget in contracts/vault-factory).
 */
import { xdr } from "@stellar/stellar-sdk";
import { sv } from "./chain";

export interface BindTerms {
  min_liquidity_sleeve_bps: number;
  max_single_asset_bps: number;
  max_issuer_bps: number;
  max_slippage_bps: number;
  rebalance_band_bps: number;
  sleeves: { asset_id: string; target_weight_bps: number; min_band_bps: number; max_band_bps: number; sleeve_type: string }[];
}

export interface OwnVaultInput {
  owner: string;          // the creator: admin, pause authority and policy owner
  numeraire: string;      // the vault currency (a registered cash asset)
  feeBps: number;         // Constella's fee: the factory default (D-O)
  capUnits: bigint;       // TVL cap = per-user cap (a private vault)
  name: string;
  symbol: string;
  salt: Uint8Array;       // 32 bytes; the address is bound to (creator, salt)
  bind: BindTerms;
  policyHashHex: string;  // 64 hex chars
}

/** A contracttype struct: ScMap with keys in sorted order. */
export function struct(fields: Record<string, xdr.ScVal>): xdr.ScVal {
  return xdr.ScVal.scvMap(Object.keys(fields).sort().map((k) => new xdr.ScMapEntry({ key: sv.sym(k), val: fields[k] })));
}

function hexBytes(hex: string, n: number): Uint8Array {
  if (!/^[0-9a-fA-F]*$/.test(hex) || hex.length !== n * 2) throw new Error(`expected ${n} bytes of hex`);
  return Uint8Array.from(hex.match(/../g)!.map((b) => parseInt(b, 16)));
}

export function createVaultArgs(i: OwnVaultInput): xdr.ScVal[] {
  if (i.salt.length !== 32) throw new Error("salt must be 32 bytes");
  const params = struct({
    admin: sv.addr(i.owner),
    pause_authority: sv.addr(i.owner),
    numeraire: sv.addr(i.numeraire),
    performance_fee_bps: sv.u32(i.feeBps),
    tvl_cap: sv.i128(i.capUnits),
    per_user_cap: sv.i128(i.capUnits),
    name: sv.str(i.name),
    symbol: sv.str(i.symbol),
    salt: sv.bytes(i.salt),
  });
  const b = i.bind;
  const terms = struct({
    policy_hash: sv.bytes(hexBytes(i.policyHashHex, 32)),
    min_liquidity_sleeve_bps: sv.u32(b.min_liquidity_sleeve_bps),
    max_single_asset_bps: sv.u32(b.max_single_asset_bps),
    max_issuer_bps: sv.u32(b.max_issuer_bps),
    max_slippage_bps: sv.u32(b.max_slippage_bps),
    rebalance_band_bps: sv.u32(b.rebalance_band_bps),
    sleeves: xdr.ScVal.scvVec(b.sleeves.map((s) => struct({
      asset_id: sv.sym(s.asset_id),
      target_weight_bps: sv.u32(s.target_weight_bps),
      min_band_bps: sv.u32(s.min_band_bps),
      max_band_bps: sv.u32(s.max_band_bps),
      sleeve_type: sv.sym(s.sleeve_type),
    }))),
  });
  return [sv.addr(i.owner), params, terms];
}
