import pytest

from constella_relayer.accounts import DAY, RateLimiter, parse_owner_key
from constella_relayer.gateway import Rejected

KEY = "04" + "11" * 64


def test_owner_key_must_be_an_uncompressed_p256_key_in_hex():
    assert parse_owner_key(KEY) == bytes.fromhex(KEY)
    for bad in [None, 42, KEY[:-2], "zz" + KEY[2:], "02" + KEY[2:]]:
        with pytest.raises(Rejected) as e:
            parse_owner_key(bad)
        assert e.value.code == "INVALID_OWNER_KEY"


def test_per_client_and_global_daily_limits():
    now = [1000.0]
    lim = RateLimiter(per_client=2, total=3, clock=lambda: now[0])
    for _ in range(2):
        lim.check("1.1.1.1"); lim.count("1.1.1.1")
    with pytest.raises(Rejected, match="this client"):
        lim.check("1.1.1.1")
    lim.check("2.2.2.2"); lim.count("2.2.2.2")
    with pytest.raises(Rejected, match="capacity"):
        lim.check("3.3.3.3")
    now[0] += DAY  # the window slides
    lim.check("1.1.1.1"); lim.check("3.3.3.3")


def test_checking_does_not_consume_the_allowance():
    lim = RateLimiter(per_client=1, total=10, clock=lambda: 0.0)
    for _ in range(5):
        lim.check("a")  # e.g. requests for accounts that already exist
    lim.count("a")
    with pytest.raises(Rejected):
        lim.check("a")
