"""Audit A2: caps the user writes must reach the policy - tightened as
written, loosened only with an explicit confirmation flag, never silently
replaced by defaults. REGISTRY.md: issuer and single-asset caps are
user-configurable in the signed policy (defaults 30 % / 25 %)."""
from src.optimizer import optimize_portfolio
from src.parser import parse_thesis
from src.retriever import retrieve_candidates
from src.signer import assemble_canonical_policy
from tests.fixtures import fixture_registry


def flagged(policy, *words):
    text = " ".join(policy.ambiguities).lower()
    return all(w in text for w in words)


def test_a_looser_issuer_cap_is_honoured_and_flagged_for_confirmation():
    p = parse_thesis("Keep 10% instant liquidity, no more than 40% per issuer, rest in dollar t-bills.")
    assert p.max_issuer_weight == 0.40
    assert flagged(p, "40", "issuer", "confirm")


def test_tighter_caps_are_applied_as_written_without_noise():
    p = parse_thesis("Max 20% per issuer and at most 15% in any single asset, keep 10% cash.")
    assert (p.max_issuer_weight, p.max_single_asset_weight) == (0.20, 0.15)
    assert not flagged(p, "confirm")


def test_cap_phrasings():
    for text, want in [
        ("Issuer cap of 25% please, safe treasury runway.", 0.25),
        ("25% max per issuer, safe runway.", 0.25),
        ("Limit any single issuer to 22.5%, preserve capital.", 0.225),
        ("Not more than 18% from any one issuer for runway safety.", 0.18),
    ]:
        assert parse_thesis(text).max_issuer_weight == want, text
    for text, want in [
        ("No single asset above 12% please, safe runway.", 0.12),
        ("Cap each asset at 20%, preserve capital.", 0.20),
    ]:
        assert parse_thesis(text).max_single_asset_weight == want, text


def test_conflicting_caps_take_the_strictest_and_are_flagged():
    p = parse_thesis("No more than 40% per issuer. Actually max 20% per issuer for safety runway.")
    assert p.max_issuer_weight == 0.20
    assert flagged(p, "conflict", "issuer")


def test_impossible_or_unsupported_caps_keep_the_default_and_are_flagged():
    p = parse_thesis("Allow 0% per issuer and up to 150% in any single asset, safe runway.")
    assert (p.max_issuer_weight, p.max_single_asset_weight) == (0.30, 0.25)
    assert flagged(p, "issuer") and flagged(p, "asset")
    m = parse_thesis("At least 30% per issuer, safe runway please.")
    assert m.max_issuer_weight == 0.30 and flagged(m, "minimum", "issuer")


def test_a_maximum_cash_is_not_mistaken_for_a_cash_floor():
    p = parse_thesis("Keep at most 10% cash and put the rest in dollar t-bills for yield.")
    assert p.min_liquidity_sleeve != 0.10 or flagged(p, "maximum", "cash")
    assert flagged(p, "maximum", "cash")
    q = parse_thesis("Keep at least 20% cash, rest in dollar t-bills for runway.")
    assert q.min_liquidity_sleeve == 0.20


def test_allocations_and_caps_coexist():
    p = parse_thesis("70% dollar t-bill funds, 20% euro t-bills, 10% instant liquidity, no more than 40% per issuer.")
    assert p.currency_exposure == {"USD": 0.70, "EUR": 0.20}
    assert p.min_liquidity_sleeve == 0.10
    assert p.max_issuer_weight == 0.40


def test_without_caps_the_documented_defaults_apply():
    p = parse_thesis("Safe treasury runway in dollars with 10% instant liquidity.")
    assert (p.max_issuer_weight, p.max_single_asset_weight) == (0.30, 0.25)


def test_the_users_caps_reach_the_policy_they_sign():
    thesis = "Safe runway: 10% instant liquidity, max 20% per issuer, no single asset above 15%, rest in dollar t-bills."
    p = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    cands = retrieve_candidates(p, fixture_registry())
    signed = assemble_canonical_policy(p, optimize_portfolio(p, cands), cands, owner="GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR", created_at="2026-11-17T00:00:00Z")
    assert signed.constraints.max_issuer_weight_bps == 2000
    assert signed.constraints.max_single_asset_weight_bps == 1500


def test_two_cash_floors_take_the_higher_whatever_the_order():
    for text in ["Keep at least 20% cash, and a 5% buffer, rest in dollar t-bills.",
                 "Keep a 5% buffer, and at least 20% cash, rest in dollar t-bills."]:
        p = parse_thesis(text)
        assert p.min_liquidity_sleeve == 0.20, text
        assert any("applied the higher" in a for a in p.ambiguities)
