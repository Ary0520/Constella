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
