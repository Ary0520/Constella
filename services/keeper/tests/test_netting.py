"""Step 6 (founding doc 4.5; EXECUTION.md Steps 1-2; RISKS R04): flow netting.

Deposits and withdrawals arrive as cash (the vault currency). A rebalance
trades only the residual: sleeves more than HALF their band from target are
brought back to target, the cash sleeve absorbs it; leftover excess cash buys
the MOST underweight sleeves (no sells), a leftover cash shortfall sells the
MOST overweight ones (no buys). Sleeves within half their band are not traded
(the half-band margin keeps them inside their signed band after slippage,
which is capped at 0.5 % of the traded value)."""
from constella_keeper.planner import Sleeve, plan, plan_raise_cash, weights

ONE = 10 ** 7
ident = lambda a, b, amt: amt  # a pool exactly at the oracle price


def cash(v, t):
    return Sleeve("USDC", "CUSDC", t, 200, v * ONE, ONE, True)


def s(name, v, t):
    return Sleeve(name, "C" + name, t, 200, v * ONE, ONE, False)


def legs(p):
    by = {"CUSDC": "USDC"}
    return [(by.get(o.asset_in, o.asset_in[1:]), by.get(o.asset_out, o.asset_out[1:]), o.amount_in // ONE) for o in p.operations]


def in_band(p, sleeves):
    w = dict(p.weights_after)
    return all(abs(w[x.asset_id] - x.target_bps) <= x.band_bps for x in sleeves)


def test_a_deposit_buys_the_breached_sleeve_only_and_sells_nothing():
    # +500 cash after a deposit; B is 580 short (breach); A is +80 (within half band).
    sl = [cash(2500, 2000), s("A", 4080, 4000), s("B", 3420, 4000)]
    p = plan(sl, ONE, 200, 50, ident)
    assert legs(p) == [("USDC", "B", 580)], "A (in its half band) is not traded"
    assert in_band(p, sl)


def test_a_withdrawal_sells_only_the_overweight_breach():
    # cash -500 after withdrawals; A +450 (breach) is sold; B +50 is left alone.
    sl = [cash(1500, 2000), s("A", 4450, 4000), s("B", 4050, 4000)]
    p = plan(sl, ONE, 200, 50, ident)
    assert legs(p) == [("A", "USDC", 450)]
    assert in_band(p, sl)


def test_excess_cash_goes_to_the_most_underweight_sleeves_first_with_no_sells():
    # Only cash breaches (+250). Underweights A -100, B -100, C -90; D +40.
    sl = [cash(2250, 2000), s("A", 1900, 2000), s("B", 1900, 2000), s("C", 1910, 2000), s("D", 2040, 2000)]
    p = plan(sl, ONE, 200, 50, ident)
    got = legs(p)
    assert all(a == "USDC" for a, _, _ in got), "a deposit never causes a sale"
    assert dict((b, n) for _, b, n in got) == {"A": 100, "B": 100, "C": 50}
    assert in_band(p, sl)


def test_netted_flows_inside_the_bands_trade_nothing():
    # deposits and withdrawals of the period cancel in cash: nothing breaches.
    sl = [cash(2150, 2000), s("A", 3950, 4000), s("B", 3900, 4000)]
    assert plan(sl, ONE, 200, 50, ident) is None


def test_raising_cash_sells_the_most_overweight_sleeve_first():
    # B is the largest holding but underweight; A is overweight: sell A.
    sl = [cash(100, 2000), s("B", 520, 5000), s("A", 480, 3000)]
    p = plan_raise_cash(sl, ONE, 50, ident, 200 * ONE)
    assert [o.asset_in for o in p.operations] == ["CA"]
