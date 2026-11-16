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
    assert r.profile == "cash_reserve" and r.numeraire == "USDC"
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
    assert any("conflicts with the cash reserve profile" in r for r in e.value.reasons)
    near = e.value.nearest
    assert near.weights_bps["USDC"] >= 8500 and check_allocation(near.weights_bps, p, c, "USDC", soft=False) == []


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
    assert e.value.nearest is None and "USDC" in e.value.reasons[0]


def test_unknown_profiles_are_refused():
    p = retail()
    with pytest.raises(ValueError):
        optimize_portfolio(p, cands(p), profile="yolo")


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


def test_the_lending_cap_binds_in_a_profile_where_the_currency_floor_does_not_imply_it(monkeypatch):
    # Under Cash Reserve, USDC >= 85 % already implies lending <= 15 %. A
    # future profile (e.g. "Yield") has a lower currency floor, so the
    # lending cap must bind on its own.
    from src import optimizer
    monkeypatch.setitem(optimizer.PROFILES, "test_low_floor", optimizer.Profile(min_numeraire=0.50, max_lending=0.15))
    p = retail(target_yield_orientation="maximize", currency_exposure={})
    c = cands(p)
    r = optimize_portfolio(p, c, profile="test_low_floor")
    assert r.weights_bps["BLEND_USDC_SUPPLY"] <= 1500
    assert r.weights_bps["USDC"] >= 5000
    assert check_allocation(r.weights_bps, p, c, "USDC", profile="test_low_floor") == []


def test_the_vault_currency_risk_is_disclosed_not_called_within_the_cap():
    from src.explainer import generate_explanation
    p = retail()
    c = cands(p)
    r = optimize_portfolio(p, c)
    warnings = generate_explanation("Keep my runway safe in dollars.", p, r, c).issuer_concentration_warnings
    circle = [w for w in warnings if "CIRCLE" in w]
    assert circle and "USDC" in circle[0] and "depeg" in circle[0]
    assert all("within the configured cap" not in w for w in circle)
