import pytest

from src.yields import (S7, S12, blend_rates, blend_supply_apr_pct, blend_utilization, realized_apr_pct)

CFG = {"util": 7_500_000, "r_base": 100_000, "r_one": 500_000, "r_two": 5_000_000, "r_three": 15_000_000}


def data(b_supply, d_supply, ir_mod=S7, b_rate=S12, d_rate=S12):
    return {"b_supply": b_supply, "d_supply": d_supply, "ir_mod": ir_mod, "b_rate": b_rate, "d_rate": d_rate}


def test_matches_the_vector_from_the_real_blend_pool():
    # Printed by the Rust test that checks the formula against the real
    # pool's one-day accrual (<= 1 bp).
    assert blend_rates(CFG, data(230_000_000_000, 100_000_000_000), 1_000_000) == (389_856, 152_551)
    assert blend_supply_apr_pct(CFG, data(230_000_000_000, 100_000_000_000), 1_000_000) == pytest.approx(1.52551)


def approx_rates(util, ir_mod=1.0, bstop=0.1):
    """Float version of Blend's curve, for the segments the pool test cannot reach."""
    t, r0, r1, r2, r3 = 0.75, 0.01, 0.05, 0.5, 1.5
    if util <= t:
        b = (util / t * r1 + r0) * ir_mod
    elif util <= 0.95:
        b = ((util - t) / (0.95 - t) * r2 + r1 + r0) * ir_mod
    else:
        b = (util - 0.95) / 0.05 * r3 + ir_mod * (r0 + r1 + r2)
    return b, b * util * (1 - bstop)


@pytest.mark.parametrize("util_pct,ir_mod", [(20, 1.0), (75, 1.0), (85, 1.0), (85, 2.0), (95, 1.0), (97, 1.0), (97, 3.0)])
def test_every_segment_of_the_curve(util_pct, ir_mod):
    d = data(100_000_000_000, util_pct * 1_000_000_000, ir_mod=int(ir_mod * S7))
    assert blend_utilization(d) == util_pct * 100_000
    borrow, supply = blend_rates(CFG, d, 1_000_000)
    eb, es = approx_rates(util_pct / 100, ir_mod)
    assert borrow / S7 == pytest.approx(eb, rel=1e-5, abs=2e-7)
    assert supply / S7 == pytest.approx(es, rel=1e-5, abs=2e-7)


def test_no_borrowing_no_supply_yield_and_utilisation_caps_at_one():
    assert blend_rates(CFG, data(100, 0), 1_000_000)[1] == 0
    assert blend_utilization(data(100, 200)) == S7


def test_realised_apr_from_the_vaults_nav_history():
    day = 86_400
    pts = [(0, 10_000_000), (30 * day, 10_041_096)]  # +0.41096 % in 30 days
    assert realized_apr_pct(pts) == pytest.approx(5.0, rel=1e-4)
    # Window: only the last 7 days count.
    pts = [(0, 10_000_000), (23 * day, 10_000_000), (30 * day, 10_009_589)]
    assert realized_apr_pct(pts, window_days=7) == pytest.approx(5.0, rel=1e-3)
    assert realized_apr_pct(pts) == pytest.approx(1.1667, rel=1e-3)


def test_not_enough_data_is_unknown_not_zero():
    assert realized_apr_pct([]) is None
    assert realized_apr_pct([(0, 10_000_000)]) is None
    assert realized_apr_pct([(0, 10_000_000), (3_600, 10_001_000)]) is None


def test_live_rates_reach_the_explanation_only_when_real():
    from src.explainer import generate_explanation
    from src.optimizer import optimize_portfolio
    from src.thesis_llm import explain_facts
    from tests.test_optimizer_fail_closed import cands, retail
    p = retail(currency_exposure={})
    c = cands(p)
    for a in c:
        a.current_yield_pct = 0.0
    r = optimize_portfolio(p, c)
    rep = generate_explanation("safe runway", p, r, c)
    assert "variable_rates_pct_per_year" not in explain_facts(p, r, rep, c)
    blend = next(a for a in c if a.asset_id == "BLEND_USDC_SUPPLY")
    blend.current_yield_pct = 1.52551
    facts = explain_facts(p, r, rep, c)
    assert facts["variable_rates_pct_per_year"] == {blend.display_name: 1.53}
    assert "not promised" in facts["yields_note"]
