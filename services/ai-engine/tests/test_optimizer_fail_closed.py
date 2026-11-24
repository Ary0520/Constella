"""Audit A1 + decision D-A: the optimizer never relaxes a rule and never
returns weights that break one; conflicts are explained with the nearest
valid allocation for the user to accept."""
import pytest

from src.models import PolicyDraft
from src.optimizer import InfeasibleAllocation, check_allocation, optimize_portfolio
from src.parser import parse_thesis
from src.retriever import retrieve_candidates
from tests.fixtures import fixture_registry


def cands(policy):
    return retrieve_candidates(policy, fixture_registry())


def retail(**kw):
    base = dict(jurisdiction="EU", investor_class="retail", currency_exposure={"USD": 1.0})
    base.update(kw)
    return PolicyDraft(**base)


def test_retail_cash_reserve_keeps_85_percent_usdc_and_at_most_15_percent_lending():
    p = retail(target_yield_orientation="maximize")
    c = cands(p)
    r = optimize_portfolio(p, c)
    assert r.profile == "thesis" and r.numeraire == "USDC"  # D-P: a plain USD cash thesis still gives 85/15
    assert r.weights_bps["USDC"] >= 8500
    assert r.weights_bps["BLEND_USDC_SUPPLY"] <= 1500
    assert r.weights_bps["BLEND_USDC_SUPPLY"] > 0, "yield-seeking uses the lending room"
    assert sum(r.weights_bps.values()) == 10_000
    assert check_allocation(r.weights_bps, p, c, "USDC") == []


def test_a_users_tight_issuer_cap_is_respected_not_relaxed():
    p = retail(max_issuer_weight=0.02, currency_exposure={"USD": 0.9, "EUR": 0.1})
    c = cands(p)
    with pytest.raises(InfeasibleAllocation) as e:
        optimize_portfolio(p, c)
    # EUR >= 5 % is impossible while Circle (non-USDC) <= 2 %: never quietly loosened.
    near = e.value.nearest
    assert near is not None and near.weights_bps["EURC"] <= 200
    assert check_allocation(near.weights_bps, p, c, "USDC", soft=False) == []
    assert e.value.relaxed == ["currency split"]


def test_a_conflicting_currency_split_is_explained_with_the_nearest_valid_mix():
    p = parse_thesis("Half in euro please: 50% euro and 50% dollar, safe runway.", jurisdiction="EU", investor_class="retail")
    c = cands(p)
    with pytest.raises(InfeasibleAllocation) as e:
        optimize_portfolio(p, c)
    assert any("cannot be met under your limits" in r and "per-asset cap" in r for r in e.value.reasons)
    near = e.value.nearest
    assert near.weights_bps["EURC"] <= 2500 and check_allocation(near.weights_bps, p, c, "USDC", soft=False) == []


def test_a_currency_with_no_eligible_asset_is_reported():
    p = retail(currency_exposure={"USD": 0.7, "GBP": 0.3})
    with pytest.raises(InfeasibleAllocation) as e:
        optimize_portfolio(p, cands(p))
    assert any("no eligible asset in GBP" in r for r in e.value.reasons)


def test_without_the_vault_currency_there_is_no_allocation_at_all():
    p = retail()
    no_usdc = [x for x in cands(p) if x.asset_id != "USDC"]
    with pytest.raises(InfeasibleAllocation) as e:
        optimize_portfolio(p, no_usdc)
    # A USD thesis never falls back to an unrequested currency (EURC) as vault currency.
    assert e.value.nearest is None and "USD" in e.value.reasons[0]


def test_an_explicit_vault_currency_that_is_not_available_is_refused():
    p = retail()
    with pytest.raises(InfeasibleAllocation):
        optimize_portfolio(p, cands(p), numeraire="NOT_A_CANDIDATE")


def test_the_final_check_catches_every_rule():
    p = retail(max_issuer_weight=0.30, max_single_asset_weight=0.25, min_liquidity_sleeve=0.10)
    c = cands(retail(currency_exposure={}))  # USDC, EURC, Blend
    usd_only = cands(p)
    assert "not eligible candidates: EURC" in check_allocation({"USDC": 8500, "EURC": 1500}, p, usd_only, "USDC", soft=False)
    ok = {"USDC": 8500, "EURC": 0, "BLEND_USDC_SUPPLY": 1500}
    assert check_allocation(ok, p, c, "USDC") == []
    assert check_allocation({**ok, "USDC": 8499, "EURC": 1}, p, c, "USDC", soft=False)  # vault currency < 85 %
    assert check_allocation({"USDC": 8400, "EURC": 0, "BLEND_USDC_SUPPLY": 1600}, p, c, "USDC", soft=False)  # lending
    assert check_allocation({**ok, "USDC": 8501}, p, c, "USDC")  # sum != 10,000
    tight = retail(max_issuer_weight=0.05, max_single_asset_weight=0.25)
    assert check_allocation({"USDC": 8500, "EURC": 1000, "BLEND_USDC_SUPPLY": 500}, tight, c, "USDC", soft=False)  # issuer
    small = retail(max_single_asset_weight=0.05)
    assert check_allocation({"USDC": 8500, "EURC": 0, "BLEND_USDC_SUPPLY": 1500}, small, c, "USDC", soft=False)  # asset cap
    eur = retail(currency_exposure={"USD": 0.8, "EUR": 0.2})
    assert check_allocation(ok, eur, c, "USDC")  # soft: EUR split not met


def test_the_vault_currency_is_exempt_from_both_concentration_caps():
    p = retail(max_single_asset_weight=0.25, max_issuer_weight=0.30)
    c = cands(p)
    r = optimize_portfolio(p, c)
    assert r.weights["USDC"] > 0.30 and r.issuer_weights["CIRCLE"] > 0.30, "exempt, governed by profile + floor"


def test_each_lending_pool_is_capped_even_when_the_cash_floor_is_low():
    # D-P: the 15 % per-pool cap (D-A, one audited pool) binds on its own - a
    # 5 % floor and a yield-maximising thesis still cannot lend more.
    p = retail(target_yield_orientation="maximize", min_liquidity_sleeve=0.05)
    c = cands(p)
    r = optimize_portfolio(p, c)
    assert 0 < r.weights_bps["BLEND_USDC_SUPPLY"] <= 1500
    assert check_allocation({**r.weights_bps, "BLEND_USDC_SUPPLY": 1501, "USDC": r.weights_bps["USDC"] - 1}, p, c, "USDC", soft=False)

def test_the_vault_currency_risk_is_disclosed_not_called_within_the_cap():
    from src.explainer import generate_explanation
    p = retail()
    c = cands(p)
    r = optimize_portfolio(p, c)
    warnings = generate_explanation("Keep my runway safe in dollars.", p, r, c).issuer_concentration_warnings
    circle = [w for w in warnings if "CIRCLE" in w]
    assert circle and "USDC" in circle[0] and "depeg" in circle[0]
    assert all("within the configured cap" not in w for w in circle)


def test_the_signed_policy_declares_the_vault_currency_for_the_chain():
    # A1b: the policy contract exempts only the sleeve typed "numeraire".
    from src.signer import assemble_canonical_policy
    p = retail(currency_exposure={})
    c = cands(p)
    r = optimize_portfolio(p, c)
    signed = assemble_canonical_policy(p, r, c, owner="GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR", created_at="2026-11-17T00:00:00Z")
    kinds = {s.asset_id: s.sleeve_type for s in signed.sleeves}
    assert kinds["USDC"] == "numeraire"
    assert all(k != "numeraire" for a, k in kinds.items() if a != "USDC")


def test_the_signer_refuses_assets_outside_the_candidates():
    from src.signer import assemble_canonical_policy
    p = retail()
    c = cands(p)
    r = optimize_portfolio(p, c)
    r.weights_bps = {**r.weights_bps, "FRANKLIN_BENJI": 1}
    with pytest.raises(ValueError):
        assemble_canonical_policy(p, r, c, owner="GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR", created_at="2026-11-17T00:00:00Z")
