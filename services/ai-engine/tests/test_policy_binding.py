"""OPEN-ISSUES #3: the signed policy hash must describe exactly what is enforced.

* The canonical policy names the REAL owner (the account that signs
  register_policy), never a placeholder; an invalid address is refused.
* The on-chain bind_policy terms are derived from the canonical policy itself,
  so the hashed constraints and the enforced constraints cannot drift
  (previously: hashed rebalance band 500 bps, enforced 200 bps).
* Canonical JSON is RFC 8785 (JCS) compatible: only integers, strings, booleans
  and null are allowed (Python's json number formatting differs from JCS for
  floats), keys sorted, no whitespace.
* The HTTP API only returns a policy hash for a given, valid owner.
"""
import json

import pytest
from stellar_sdk import Keypair

from src.api import Engine
from src.models import AssetCandidate, OptimizationResult, PolicyDraft, SleeveType
from src.signer import assemble_canonical_policy, onchain_terms, to_canonical_json

OWNER_G = Keypair.random().public_key
OWNER_C = "CA6YQJ4PISBYUXMYTFGCAZPKM6K4XFAMW6MFIOGLWQ6FPSP637U2DFDC"  # a contract (smart account) address


def _inputs():
    draft = PolicyDraft(jurisdiction="DE", investor_class="retail", min_liquidity_sleeve=0.10,
                        max_issuer_weight=0.30, max_single_asset_weight=0.25)
    cands = [
        AssetCandidate(asset_id="USDC", display_name="USDC", currency="USD", issuer="CIRCLE", contract_address="CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA",
                       sleeve_type=SleeveType.CASH_STABLE, liquidity_tier=0, permissionless=True),
        AssetCandidate(asset_id="BLEND_USDC_SUPPLY", display_name="Blend USDC", currency="USD", issuer="BLEND", contract_address="CBYJTGQVTT3LNLOS4L2HB7AEIJOZZFYQLEXZ4R6YZNKQOQJME53HWLCN",
                       sleeve_type=SleeveType.LENDING, liquidity_tier=0, permissionless=True),
    ]
    opt = OptimizationResult(weights={"USDC": 0.85, "BLEND_USDC_SUPPLY": 0.15},
                             weights_bps={"USDC": 8500, "BLEND_USDC_SUPPLY": 1500}, numeraire="USDC",
                             sleeve_weights={}, currency_weights={}, issuer_weights={}, algorithm_used="LP", profile="cash_reserve")
    return draft, opt, cands


def test_owner_is_required_and_has_no_placeholder_default():
    draft, opt, cands = _inputs()
    with pytest.raises(TypeError):
        assemble_canonical_policy(draft, opt, cands)  # no owner -> refuse, never a placeholder


@pytest.mark.parametrize("bad", ["GBADMIN1234567890", "", "not-an-address", OWNER_G[:-1] + "A"])
def test_invalid_owner_addresses_are_refused(bad):
    draft, opt, cands = _inputs()
    with pytest.raises(ValueError):
        assemble_canonical_policy(draft, opt, cands, owner=bad)


@pytest.mark.parametrize("owner", [OWNER_G, OWNER_C])
def test_the_canonical_policy_names_the_real_owner(owner):
    draft, opt, cands = _inputs()
    p = assemble_canonical_policy(draft, opt, cands, owner=owner)
    assert p.admin.admin_address == owner
    assert p.admin.multisig_signers == [owner]


def test_hashed_constraints_equal_the_enforced_terms():
    draft, opt, cands = _inputs()
    p = assemble_canonical_policy(draft, opt, cands, owner=OWNER_C)
    t = onchain_terms(p)
    c = p.constraints
    assert t["rebalance_band_bps"] == c.rebalance_band_bps
    assert t["max_slippage_bps"] == c.max_slippage_bps
    assert t["min_liquidity_sleeve_bps"] == c.min_liquidity_sleeve_bps
    assert t["max_single_asset_bps"] == c.max_single_asset_weight_bps
    assert t["max_issuer_bps"] == c.max_issuer_weight_bps
    by_id = {s.asset_id: s for s in p.sleeves}
    assert {s["asset_id"] for s in t["sleeves"]} == set(by_id)
    for s in t["sleeves"]:
        cs = by_id[s["asset_id"]]
        assert s["target_weight_bps"] == cs.target_weight_bps
        assert s["min_band_bps"] == max(0, cs.target_weight_bps - cs.band_bps)
        assert s["max_band_bps"] == min(10_000, cs.target_weight_bps + cs.band_bps)
        assert s["sleeve_type"] == cs.sleeve_type
    assert [s["sleeve_type"] for s in t["sleeves"] if s["asset_id"] == "USDC"] == ["numeraire"]


def test_canonical_json_is_jcs_safe_and_refuses_floats():
    draft, opt, cands = _inputs()
    p = assemble_canonical_policy(draft, opt, cands, owner=OWNER_G, created_at="2026-11-24T00:00:00Z")
    out = to_canonical_json(p)
    assert out == json.dumps(json.loads(out), sort_keys=True, separators=(",", ":"), ensure_ascii=False)

    def no_floats(v):
        if isinstance(v, float):
            return False
        if isinstance(v, dict):
            return all(no_floats(x) for x in v.values())
        if isinstance(v, list):
            return all(no_floats(x) for x in v)
        return True
    assert no_floats(json.loads(out))
    # A float smuggled into the policy is refused rather than hashed.
    p.basket_name = 1.5  # type: ignore[assignment]
    with pytest.raises(ValueError):
        to_canonical_json(p)


def _engine(monkeypatch):
    draft, opt, cands = _inputs()
    import src.api as api
    monkeypatch.setattr(api, "parse_thesis", lambda text, jurisdiction, investor_class: draft)
    monkeypatch.setattr(api, "retrieve_candidates", lambda d, reg: cands)
    monkeypatch.setattr(api, "optimize_portfolio", lambda d, c: opt)
    monkeypatch.setattr(api, "simulate_portfolio", lambda w, c, d: type("S", (), {"model_dump": lambda self: {}})())
    monkeypatch.setattr(api, "generate_explanation", lambda t, d, o, c: type("E", (), {"model_dump": lambda self: {}})())
    reg = type("R", (), {"eligibility": lambda self, j, c: []})()
    return Engine(lambda: reg)


def test_api_hash_only_for_a_valid_owner(monkeypatch):
    eng = _engine(monkeypatch)
    base = {"text": "keep it safe", "jurisdiction": "DE"}
    status, out = eng.thesis(dict(base, owner=OWNER_C))
    assert status == 200
    assert out["policy"]["admin"]["admin_address"] == OWNER_C
    assert len(out["policy_hash"]) == 64 and out["signable"] is True
    assert out["bind_policy"]["rebalance_band_bps"] == out["policy"]["constraints"]["rebalance_band_bps"]

    status, out = eng.thesis(base)  # no owner: a preview, never a signable hash
    assert status == 200 and out["policy_hash"] is None and out["signable"] is False

    status, out = eng.thesis(dict(base, owner="GBADMIN1234567890"))
    assert status == 400 and out["error"] == "INVALID_OWNER"
