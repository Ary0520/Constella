"""Step 4 (docs/plans/NEXT.md, decision D-P): different theses give different,
explained baskets. Constraints come from the thesis (liquidity floor, currency
split, exclusions, horizon, caps) instead of one fixed profile; every rule stays
fail-closed (A1). Registry: the on-chain-shaped fixture (tests/fixtures.py)."""
import pytest

from src.optimizer import InfeasibleAllocation, check_allocation, optimize_portfolio
from src.parser import parse_thesis
from src.retriever import retrieve_candidates
from tests.fixtures import entry, fixture_address, fixture_entries, fixture_registry


def run(text, cls="retail", jur="DE", reg=None):
    draft = parse_thesis(text, jurisdiction=jur, investor_class=cls)
    cands = retrieve_candidates(draft, reg or fixture_registry())
    return draft, cands, optimize_portfolio(draft, cands)


def nonzero(r):
    return {a: b for a, b in r.weights_bps.items() if b > 0}


# --- the parser reads the founding doc's own thesis -------------------------------------

def test_a_third_in_euro_is_read_and_the_rest_stays_in_the_vault_currency():
    d = parse_thesis("Keep 18 months of runway safe, hold a third in euro, and stay liquid enough to borrow against.",
                     jurisdiction="DE", investor_class="retail")
    assert d.currency_exposure["EUR"] == pytest.approx(1 / 3, abs=0.005)
    assert d.currency_exposure["USD"] == pytest.approx(2 / 3, abs=0.005)


@pytest.mark.parametrize("phrase,eur", [("half in euros", 0.5), ("a quarter in euros", 0.25), ("33% in euros", 0.33)])
def test_fractions_and_partial_splits(phrase, eur):
    d = parse_thesis(f"Keep a safe runway, {phrase}.", jurisdiction="DE", investor_class="retail")
    assert d.currency_exposure["EUR"] == pytest.approx(eur, abs=0.005)
    assert sum(d.currency_exposure.values()) == pytest.approx(1.0, abs=0.005)


# --- different theses, different baskets ------------------------------------------------

def test_the_default_cash_thesis_is_still_the_cash_reserve_mix_and_takes_no_currency_risk():
    _, _, r = run("We are a small studio. Keep a safe runway in dollars with at least 10% instant cash.")
    assert nonzero(r) == {"USDC": 8500, "BLEND_USDC_SUPPLY": 1500}
    assert r.numeraire == "USDC"


def test_no_lending_means_no_lending():
    _, _, r = run("Keep a safe runway in dollars, no lending.")
    assert nonzero(r) == {"USDC": 10000}


def test_an_unrequested_currency_gets_nothing():
    _, _, r = run("Keep a safe dollar runway, lend a little.")
    assert r.weights_bps.get("EURC", 0) == 0


def test_a_third_in_euro_within_raised_caps_holds_a_third_in_euro():
    _, _, r = run("Keep 18 months of runway safe, hold a third in euro. Up to 40% per issuer and 40% per asset.")
    assert r.numeraire == "USDC"
    assert 2833 <= r.weights_bps["EURC"] <= 3833
    assert r.weights_bps.get("BLEND_USDC_SUPPLY", 0) <= 1500
    assert r.currency_weights["EUR"] == pytest.approx(r.weights_bps["EURC"] / 10_000, abs=1e-4)


def test_a_third_in_euro_under_default_caps_is_refused_with_the_nearest_mix_and_the_reason():
    with pytest.raises(InfeasibleAllocation) as e:
        run("Keep 18 months of runway safe, hold a third in euro.")
    assert e.value.nearest is not None and e.value.nearest.weights_bps["EURC"] <= 2500
    assert any("cap" in r or "euro" in r.lower() or "EUR" in r for r in e.value.reasons)


def test_a_mostly_euro_thesis_becomes_a_euro_vault():
    _, _, r = run("Keep 70% in euros and the rest in dollars, at least 10% instant cash.")
    assert r.numeraire == "EURC"
    assert r.weights_bps["EURC"] >= 6500


def test_a_professional_treasury_holds_t_bill_funds_within_caps():
    _, _, r = run("Park 18 months of runway in dollar T-bill funds, at least 10% instant cash.", cls="professional")
    funds = {a: b for a, b in nonzero(r).items() if a in ("FRANKLIN_BENJI", "SPIKO_USD", "ONDO_USDY")}
    assert funds, "a professional dollar treasury gets T-bill funds"
    assert all(b <= 2500 for b in funds.values())
    assert r.weights_bps["USDC"] >= 1000


def test_different_theses_give_different_baskets():
    theses = [("Keep a safe runway in dollars with at least 10% instant cash.", "retail"),
              ("Keep a safe runway in dollars, no lending.", "retail"),
              ("Keep 70% in euros and the rest in dollars.", "retail"),
              ("Park 18 months of runway in dollar T-bill funds, at least 10% instant cash.", "professional")]
    baskets = {tuple(sorted(nonzero(run(t, cls=c)[2]).items())) for t, c in theses}
    assert len(baskets) == len(theses)


# --- liquidity tiers (spec: no illiquid assets in liquid baskets) ------------------------

def test_illiquid_assets_stay_out_of_a_liquid_basket():
    illiquid = entry("PRIVATE_CREDIT", "CREDCO", 3, fixture_address("PC"), tier=3)
    reg = fixture_registry(fixture_entries() + [illiquid], yields={"PRIVATE_CREDIT": 9.0, "BLEND_USDC_SUPPLY": 2.4})
    _, _, r = run("Keep a safe dollar runway, lend a little, chase some yield.", reg=reg)
    assert r.weights_bps.get("PRIVATE_CREDIT", 0) == 0


def test_every_result_passes_the_integer_exact_final_check():
    for text, cls in [("Keep 70% in euros and the rest in dollars.", "retail"),
                      ("Park 18 months of runway in dollar T-bill funds, at least 10% instant cash.", "professional")]:
        d, cands, r = run(text, cls=cls)
        assert check_allocation(r.weights_bps, d, cands, r.numeraire) == []


# --- the optimiser's own liquidity and vault-currency rules (direct, with positive controls) ---
from src.models import PolicyDraft  # noqa: E402


def _asset(asset_id, issuer, sleeve, tier, pct):
    from src.models import AssetCandidate, SleeveType
    return AssetCandidate(asset_id=asset_id, display_name=asset_id, issuer=issuer, sleeve_type=SleeveType(sleeve), currency="USD",
                          liquidity_tier=tier, permissionless=True, contract_address=fixture_address(asset_id),
                          current_yield_pct=pct)


def _cands(extra):
    base = [a for a in fixture_registry().list_assets() if a.asset_id in {"USDC", "BLEND_USDC_SUPPLY"}]
    return base + extra


def _policy(horizon):
    return PolicyDraft(jurisdiction="DE", investor_class="retail", currency_exposure={"USD": 1.0}, liquidity_horizon_hours=horizon,
                       max_single_asset_weight=0.40, max_issuer_weight=0.40, min_liquidity_sleeve=0.10,
                       target_yield_orientation="maximize", risk_tolerance="growth")


def test_an_illiquid_asset_is_held_only_when_the_horizon_allows_a_month():
    c = _cands([_asset("PRIVATE_CREDIT", "CREDCO", "yield_dollar", 3, 9.0)])
    assert optimize_portfolio(_policy(24), c).weights_bps["PRIVATE_CREDIT"] == 0
    assert optimize_portfolio(_policy(720), c).weights_bps["PRIVATE_CREDIT"] > 0  # control: the rule, not the data, blocks it
    assert check_allocation({"USDC": 8000, "BLEND_USDC_SUPPLY": 1000, "PRIVATE_CREDIT": 1000}, _policy(24), c, "USDC", soft=False)


def test_assets_slower_than_the_horizon_are_at_most_a_quarter():
    c = _cands([_asset("WEEKLY_FUND", "FUNDCO", "usd_gov_fund", 2, 6.0)])
    w = optimize_portfolio(_policy(24), c).weights_bps["WEEKLY_FUND"]
    assert 0 < w <= 2500
    assert optimize_portfolio(_policy(168), c).weights_bps["WEEKLY_FUND"] > 2500  # control: up to the 40 % cap
    assert check_allocation({"USDC": 6000, "BLEND_USDC_SUPPLY": 1000, "WEEKLY_FUND": 3000}, _policy(24), c, "USDC", soft=False)


def test_a_dollar_thesis_never_gets_a_euro_vault_currency():
    reg = fixture_registry()
    c = [a for a in reg.list_assets() if a.asset_id in {"EURC", "BLEND_USDC_SUPPLY"}]  # no USDC available
    from src.optimizer import choose_numeraire
    assert choose_numeraire(_policy(24), c) is None
    with pytest.raises(InfeasibleAllocation) as e:
        optimize_portfolio(_policy(24), c)
    assert "USD" in e.value.reasons[0]


def test_the_currency_split_lands_near_what_was_asked_not_at_the_edge():
    # Two-sided: a third in euro means ~33 %, not "at least 28 %" pushed up to
    # the cap by the diversification objective.
    _, _, r = run("Keep 18 months of runway safe, hold a third in euro. Up to 40% per issuer and 40% per asset.")
    assert abs(r.weights_bps["EURC"] - 3333) <= 100
