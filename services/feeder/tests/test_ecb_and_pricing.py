"""Unit tests on real data: the ECB fixture was downloaded from
https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml on 15 Nov 2026."""
from datetime import date, datetime, timezone
from decimal import Decimal
from pathlib import Path

import pytest

from constella_feeder.ecb import EcbError, EcbRate, parse_usd_rate
from constella_feeder.pricing import MAX_JUMP_BPS, GateRejected, OnChain, Update, decide, eurc_in_usdc

FIXTURE = Path(__file__).parent / "fixtures" / "ecb_eurofxref_daily_20261115.xml"


def utc(*a):
    return int(datetime(*a, tzinfo=timezone.utc).timestamp())


def test_parses_the_real_ecb_document():
    r = parse_usd_rate(FIXTURE.read_bytes())
    assert r.day == date(2026, 11, 13)
    assert r.usd_per_eur == Decimal("1.1052")
    # 14:15 CET (UTC+1 in November) = 13:15 UTC
    assert r.reference_unix == utc(2026, 11, 13, 13, 15)


def test_summer_time_concertation_is_converted_with_cest():
    assert EcbRate(date(2026, 7, 1), Decimal("1.1")).reference_unix == utc(2026, 7, 1, 12, 15)


@pytest.mark.parametrize("doc", [
    b"not xml",
    FIXTURE.read_bytes().replace(b"currency='USD'", b"currency='XXX'"),
    FIXTURE.read_bytes().replace(b"rate='1.1052'", b"rate='abc'"),
    FIXTURE.read_bytes().replace(b"rate='1.1052'", b"rate='-1'"),
    FIXTURE.read_bytes().replace(b"<Cube time='2026-11-13'>", b"<Cube time='2026-11-13'><Cube time='2026-11-12'/>"),
])
def test_malformed_documents_are_rejected(doc):
    with pytest.raises(EcbError):
        parse_usd_rate(doc)


def test_price_is_par_value_in_oracle_units_rounded_down():
    r = parse_usd_rate(FIXTURE.read_bytes())
    u = eurc_in_usdc(r)
    assert (u.price, u.timestamp, u.liquidity_depth_usd) == (11_052_000, utc(2026, 11, 13, 13, 15), 0)
    assert eurc_in_usdc(EcbRate(date(2026, 1, 2), Decimal("1.10529999"))).price == 11_052_999


@pytest.mark.parametrize("bad", ["0.5", "2.0"])
def test_sanity_band(bad):
    with pytest.raises(GateRejected):
        eurc_in_usdc(EcbRate(date(2026, 1, 2), Decimal(bad)))


NOW = utc(2026, 11, 15, 16, 0)
U = Update(price=11_052_000, timestamp=utc(2026, 11, 13, 13, 15))


def test_submit_when_nothing_on_chain_yet():
    assert decide(U, None, NOW) == "submit"


def test_skip_when_identical_data_is_on_chain():
    assert decide(U, OnChain(U.price, U.timestamp), NOW) == "skip"


@pytest.mark.parametrize("current", [
    OnChain(11_000_000, U.timestamp),        # same timestamp, different price
    OnChain(11_050_000, U.timestamp + 86_400),  # on-chain data is newer
])
def test_conflicting_or_older_data_is_refused(current):
    with pytest.raises(GateRejected):
        decide(U, current, NOW)


def test_future_data_is_refused():
    with pytest.raises(GateRejected):
        decide(U, None, U.timestamp - 1)


def test_jump_guard_boundary():
    prev_ts = U.timestamp - 86_400
    base = 10_000_000
    ok = Update(price=base + base * MAX_JUMP_BPS // 10_000, timestamp=U.timestamp)
    too_far = Update(price=ok.price + 1_000, timestamp=U.timestamp)
    assert decide(ok, OnChain(base, prev_ts), NOW) == "submit"
    with pytest.raises(GateRejected):
        decide(too_far, OnChain(base, prev_ts), NOW)
