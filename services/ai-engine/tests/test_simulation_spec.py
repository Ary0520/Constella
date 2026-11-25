"""Step 7 (docs/plans/NEXT.md): the simulation to the spec
(docs/build-spec/ai-engine/THESIS-ENGINE.md, Step 4: Simulate; founding doc 2.2):
  * depeg: 5 % depeg of the LARGEST stable constituent - and everything
    denominated in it (a Blend USDC supply position is USDC);
  * issuer outage: the largest-weight issuer frozen 30 days - the basket
    impact, including what can still be redeemed meanwhile;
  * worst-case exit: the slowest plausible full exit (the freeze), with the
    normal exit time beside it;
  * historical replay only from real history - never a made-up line;
  * the Rule 10 disclaimer, verbatim."""
import pandas as pd
import pytest

from src.models import AssetCandidate, SleeveType
from src.simulator import DISCLAIMER, simulate_portfolio
from src.models import PolicyDraft


def asset(aid, issuer, sleeve, currency, tier):
    return AssetCandidate(asset_id=aid, display_name=aid, issuer=issuer, sleeve_type=SleeveType(sleeve), currency=currency,
                          liquidity_tier=tier, permissionless=True)


USDC = asset("USDC", "CIRCLE", "cash", "USD", 0)
EURC = asset("EURC", "CIRCLE", "cash", "EUR", 0)
BLEND = asset("BLEND_USDC_SUPPLY", "BLEND", "lending", "USD", 1)
BENJI = asset("FRANKLIN_BENJI", "FRANKLIN_TEMPLETON", "usd_gov_fund", "USD", 1)
CANDS = [USDC, EURC, BLEND, BENJI]
POLICY = PolicyDraft(jurisdiction="DE", investor_class="retail")


def test_the_depeg_hits_the_largest_stable_constituent_and_what_is_denominated_in_it():
    r = simulate_portfolio({"USDC": 0.51, "EURC": 0.34, "BLEND_USDC_SUPPLY": 0.15}, CANDS, POLICY)
    # USDC is the largest stable constituent; the Blend supply is USDC too; EURC is untouched.
    assert r.depeg_stress.nav_impact_pct == pytest.approx(-0.05 * (0.51 + 0.15), abs=1e-4)
    assert "USDC" in r.depeg_stress.description and "BLEND_USDC_SUPPLY" in r.depeg_stress.description


def test_when_euro_cash_is_largest_only_the_euro_side_depegs():
    r = simulate_portfolio({"USDC": 0.30, "EURC": 0.60, "BLEND_USDC_SUPPLY": 0.10}, CANDS, POLICY)
    assert r.depeg_stress.nav_impact_pct == pytest.approx(-0.05 * 0.60, abs=1e-4)
    assert "EURC" in r.depeg_stress.description


def test_an_issuer_freeze_shows_what_is_trapped_and_what_can_still_be_redeemed():
    r = simulate_portfolio({"USDC": 0.51, "EURC": 0.34, "BLEND_USDC_SUPPLY": 0.15}, CANDS, POLICY)
    f = r.issuer_freeze_stress
    assert "CIRCLE" in f.description
    assert f.trapped_liquidity_pct == pytest.approx(0.85, abs=1e-4)
    # During a Circle freeze nothing is instant; the Blend supply (24 h) still is.
    assert r.redeemable_during_freeze.instant_pct == pytest.approx(0.0, abs=1e-4)
    assert r.redeemable_during_freeze.within_24h_pct == pytest.approx(0.15, abs=1e-4)


def test_the_worst_case_exit_is_the_freeze_and_the_normal_exit_is_shown_beside_it():
    r = simulate_portfolio({"USDC": 0.85, "BLEND_USDC_SUPPLY": 0.15}, CANDS, POLICY)
    assert r.normal_exit_hours == 24          # the Blend supply redeems within a day
    assert r.worst_case_exit_hours == 720     # a 30-day freeze of the largest issuer
    assert "30" in r.worst_case_scenario and "CIRCLE" in r.worst_case_scenario


def test_no_history_means_no_line_not_a_made_up_one():
    r = simulate_portfolio({"USDC": 0.85, "BLEND_USDC_SUPPLY": 0.15}, CANDS, POLICY)
    assert r.historical_nav_series == []
    assert r.history_available is False and "no" in r.history_note.lower()


def test_real_history_is_replayed():
    nav = pd.DataFrame({"USDC": [1.0, 1.0, 1.0], "BLEND_USDC_SUPPLY": [1.0, 1.001, 1.002]})
    r = simulate_portfolio({"USDC": 0.85, "BLEND_USDC_SUPPLY": 0.15}, CANDS, POLICY, historical_nav=nav)
    assert r.history_available is True and len(r.historical_nav_series) == 3
    assert r.historical_nav_series[0] == pytest.approx(100.0) and r.historical_nav_series[-1] > 100.0


def test_the_rule_10_disclaimer_is_verbatim():
    r = simulate_portfolio({"USDC": 1.0}, CANDS, POLICY)
    assert r.disclaimer == DISCLAIMER == ("This simulation uses historical data. Past performance does not predict future returns. "
                                          "This is not financial advice.")
