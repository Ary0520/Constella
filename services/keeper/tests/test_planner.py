import pytest

from constella_keeper.planner import (ROUTE_BLEND, ROUTE_SOROSWAP, PlanError, Sleeve, min_out_for, needs_rebalance,
                                      plan, to_bps, weights)

ONE = 10 ** 7
USDC, EURC, BLEND = "CUSDC", "CEURC", "CBLEND"


def usdc(held, target=8500, band=200):
    return Sleeve("USDC", USDC, target, band, held, ONE, True)


def eurc(held, target=1500, band=200, price=ONE * 108 // 100):
    return Sleeve("EURC", EURC, target, band, held, price, False)


def fair(a_in, a_out, amt):
    """A pool at the oracle price, 0.3% fee."""
    p = {USDC: ONE, EURC: ONE * 108 // 100}
    return amt * p[a_in] // p[a_out] * 997 // 1000


def policy_accepts(op, sleeves, slip):
    by = {s.address: s for s in sleeves}
    a, b = by[op.asset_in], by[op.asset_out]
    vin = op.amount_in * a.price // 10 ** a.decimals
    vout = op.min_amount_out * b.price // 10 ** b.decimals
    return vout * 10000 >= vin * (10000 - slip)


def test_in_band_means_no_trade():
    s = [usdc(8_500 * ONE), eurc(1_500 * ONE * 100 // 108)]
    assert needs_rebalance(s, ONE, 200) == {}
    assert plan(s, ONE, 200, 50, fair) is None


def test_a_fresh_deposit_buys_the_targets_with_policy_exact_minimums():
    s = [usdc(10_000 * ONE), eurc(0)]
    p = plan(s, ONE, 200, 50, fair)
    assert len(p.operations) == 1
    op = p.operations[0]
    assert (op.asset_in, op.asset_out, op.route) == (USDC, EURC, ROUTE_SOROSWAP)
    assert op.amount_in == 1_500 * ONE
    assert policy_accepts(op, s, 50)
    # ...and it is the SMALLEST accepted minimum: one unit less is refused.
    op.min_amount_out -= 1
    assert not policy_accepts(op, s, 50)
    assert dict(p.weights_after) == {"USDC": 8500, "EURC": 1500}


def test_overweight_is_sold_back_to_the_vault_currency():
    s = [usdc(7_000 * ONE), eurc(3_000 * ONE * 100 // 108)]
    p = plan(s, ONE, 200, 50, fair)
    op = p.operations[0]
    assert (op.asset_in, op.asset_out) == (EURC, USDC)
    assert p.drift_bps["EURC"] > 0
    assert policy_accepts(op, s, 50)
    assert abs(dict(p.weights_after)["EURC"] - 1500) <= 1


def test_an_off_oracle_pool_is_refused_not_traded():
    s = [usdc(10_000 * ONE), eurc(0)]
    with pytest.raises(PlanError, match="below the oracle minimum"):
        plan(s, ONE, 200, 50, lambda a, b, amt: fair(a, b, amt) * 98 // 100)


def test_unpriced_or_missing_currency_fails_closed():
    with pytest.raises(PlanError, match="no valid oracle price"):
        plan([usdc(9_000 * ONE), eurc(1_000 * ONE, price=None)], ONE, 200, 50, fair)
    with pytest.raises(PlanError, match="exactly one cash"):
        plan([eurc(1 * ONE, target=10_000), Sleeve("EURX", "CX", 0, 200, 5 * ONE, ONE, False)], ONE, 200, 50, fair)


def test_blend_legs_use_the_blend_route_and_no_amm_quote():
    b_price = ONE * 105 // 100  # 1 bToken = 1.05 USDC
    s = [usdc(10_000 * ONE), Sleeve("BLEND_USDC_SUPPLY", BLEND, 1500, 200, 0, b_price, False, True)]

    def no_quote(*a):
        raise AssertionError("Blend legs must not be quoted on an AMM")
    op = plan(s, ONE, 200, 50, no_quote).operations[0]
    assert (op.asset_in, op.asset_out, op.route) == (USDC, BLEND, ROUTE_BLEND)
    assert policy_accepts(op, s, 50)


def test_buys_never_spend_more_cash_than_the_vault_will_have():
    # Cash target 0: the EURC buy (5000) would need the sale's full value,
    # but only its minimum (after slippage) is certain - buys are scaled.
    s = [usdc(1_000 * ONE, target=0), eurc(0, target=5000), Sleeve("X", "CX", 5000, 200, 9_000 * ONE, ONE, False)]
    p = plan(s, ONE, 200, 50, lambda a, b, amt: amt)
    sells = sum(o.min_amount_out for o in p.operations if o.asset_out == USDC)
    spent = sum(o.amount_in for o in p.operations if o.asset_in == USDC)
    assert sells < 4_000 * ONE  # the sale's certain proceeds are below its value
    assert spent <= 1_000 * ONE + sells and spent > 4_900 * ONE


@pytest.mark.parametrize("vals", [[1, 1, 1], [3, 3, 4], [0, 0, 7], [999_999, 1, 0]])
def test_bps_always_sum_to_10000(vals):
    assert sum(to_bps(vals)) == 10000


def test_min_out_is_exact_across_decimals():
    a = Sleeve("A", "CA", 0, 0, 0, 12_345_678, False, decimals=7)
    b = Sleeve("B", "CB", 0, 0, 0, 98_765, False, decimals=6)
    for amt in [1, 7, 10 ** 7, 123_456_789]:
        m = min_out_for(a, b, amt, 37)
        need = amt * a.price // 10 ** 7 * (10000 - 37)
        assert (m * b.price // 10 ** 6) * 10000 >= need
        assert m == 0 or ((m - 1) * b.price // 10 ** 6) * 10000 < need


def test_an_unpriced_sleeve_blocks_the_plan_even_if_it_would_not_trade():
    # EURC sits at target but has no price: total value is unknown, so no
    # weights - and no trades for the other sleeves - can be trusted.
    s = [usdc(7_000 * ONE, target=8000), eurc(1_000 * ONE, target=1000, price=None),
         Sleeve("X", "CX", 1000, 200, 2_000 * ONE, ONE, False)]
    with pytest.raises(PlanError, match="EURC has no valid oracle price"):
        plan(s, ONE, 200, 50, lambda a, b, amt: amt)


def test_raising_cash_sells_the_claim_asset_first_and_covers_the_need():
    from constella_keeper.planner import plan_raise_cash
    s = [usdc(100 * ONE, target=3000), eurc(700 * ONE, target=7000)]
    need = 400 * ONE
    p = plan_raise_cash(s, ONE, 50, fair, need, prefer=EURC)
    assert [(o.asset_in, o.asset_out) for o in p.operations] == [(EURC, USDC)]
    assert sum(o.min_amount_out for o in p.operations) >= need
    assert all(policy_accepts(o, s, 50) for o in p.operations)
    # ... and only just: one unit less would not cover it at the minimum.
    op = p.operations[0]
    smaller = min_out_for(s[1], s[0], op.amount_in - 1, 50)
    assert smaller < need


def test_raising_cash_refuses_what_cannot_be_raised_or_an_off_oracle_pool():
    from constella_keeper.planner import plan_raise_cash
    s = [usdc(100 * ONE, target=3000), eurc(10 * ONE, target=7000)]
    with pytest.raises(PlanError, match="cannot raise"):
        plan_raise_cash(s, ONE, 50, fair, 400 * ONE, prefer=EURC)
    s = [usdc(100 * ONE, target=3000), eurc(700 * ONE, target=7000)]
    with pytest.raises(PlanError, match="below the oracle minimum"):
        plan_raise_cash(s, ONE, 50, lambda a, b, amt: fair(a, b, amt) * 98 // 100, 400 * ONE, prefer=EURC)
