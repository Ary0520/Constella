"""Step 7 (spec THESIS-ENGINE.md Step 5: Explain): the diff against the thesis
states the REAL reason for each divergence, and parser notes are shown as what
they are (a default to confirm), not as an excluded asset."""
from src.explainer import generate_explanation
from src.optimizer import InfeasibleAllocation, optimize_portfolio
from src.parser import parse_thesis
from src.retriever import retrieve_candidates
from tests.fixtures import fixture_registry


def explain(text, cls="retail"):
    d = parse_thesis(text, jurisdiction="DE", investor_class=cls)
    c = retrieve_candidates(d, fixture_registry())
    try:
        r = optimize_portfolio(d, c)
    except InfeasibleAllocation as e:
        r = e.nearest
    return d, generate_explanation(text, d, r, c)


def item(rep, category, needle):
    return next(i for i in rep.thesis_diff if i.category == category and needle in i.requested)


def test_a_capped_currency_names_the_binding_cap():
    _, rep = explain("Keep 18 months of runway safe, hold a third in euro.")   # default caps: 25 % per asset
    eur = item(rep, "currency", "EUR")
    assert "25" in eur.explanation and "per-asset cap" in eur.explanation
    assert "liquidity buffers" not in eur.explanation


def test_a_matched_currency_says_matched():
    _, rep = explain("Keep 18 months of runway safe, hold a third in euro. Up to 40% per issuer and 40% per asset.")
    assert "Matched" in item(rep, "currency", "EUR").explanation


def test_a_default_is_shown_as_a_default_to_confirm_not_an_excluded_asset():
    d, rep = explain("Keep a safe runway.")
    notes = [i for i in rep.thesis_diff if i.category == "ambiguity"]
    usd = next(i for i in notes if "defaulted to USD" in i.requested)
    assert "confirm" in usd.proposed.lower() and "closed universe" not in usd.explanation.lower()


def test_an_unregistered_asset_is_still_shown_as_excluded():
    _, rep = explain("Allocate 50% to TSLA stock and keep the rest in safe dollars.")
    tsla = next(i for i in rep.thesis_diff if i.category == "ambiguity" and "TSLA" in i.requested)
    assert "Excluded" in tsla.proposed and "closed universe" in tsla.explanation.lower()
