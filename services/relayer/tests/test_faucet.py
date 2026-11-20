import pytest

from constella_relayer.accounts import RateLimiter
from constella_relayer.faucet import Faucet, parse_address
from constella_relayer.gateway import Rejected


def test_the_faucet_refuses_mainnet():
    with pytest.raises(ValueError, match="test networks only"):
        Faucet(None, "Public Global Stellar Network ; September 2015", "S" + "A" * 55, "C", 1, RateLimiter(1, 1), RateLimiter(1, 1))


def test_addresses_are_validated():
    assert parse_address("GAD5SR3CXQLQUU64C6MN7VKRXRY5WCWYB3HT5JQI6NHV6BYX3MI5QRHF")
    assert parse_address("CBVEAQSRMIIH5CGQWWVKWX3JRAT6RAKXOCZBABHIV4QCELLEU6E675Z4")
    for bad in [None, "", "hello", "GAD5SR3CXQLQ"]:
        with pytest.raises(Rejected):
            parse_address(bad)


def test_limits_apply_before_anything_is_sent():
    from stellar_sdk import Keypair
    f = Faucet(None, "Test SDF Network ; September 2015", Keypair.random().secret, "C", 1,
               RateLimiter(1, 10, clock=lambda: 0.0), RateLimiter(3, 10, clock=lambda: 0.0))
    f.per_address.count("GADDR")
    with pytest.raises(Rejected, match="RATE_LIMITED"):
        f.send("GADDR", "1.1.1.1")   # refused before touching the network (server is None)
