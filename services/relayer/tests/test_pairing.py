import pytest

from constella_relayer.gateway import Rejected
from constella_relayer.pairing import TTL, Pairing, check_code

ACCT = "C" + "A" * 55
KEY = "04" + "ab" * 64


def test_open_join_read_with_a_shared_check_code():
    p = Pairing(clock=lambda: 0.0)
    code = p.start(ACCT)["code"]
    assert len(code) == 8
    assert p.status(code)["device_key"] is None
    j = p.join(code, KEY, "phone")
    s = p.status(code)
    assert (s["device_key"], s["name"], s["account"]) == (KEY, "phone", ACCT)
    assert j["check"] == s["check"] == check_code(KEY) and len(s["check"]) == 6


def test_codes_are_single_use_expire_and_validate_input():
    now = [0.0]
    p = Pairing(clock=lambda: now[0])
    code = p.start(ACCT)["code"]
    p.join(code, KEY, "phone")
    with pytest.raises(Rejected, match="PAIRING_USED"):
        p.join(code, KEY, "again")
    code2 = p.start(ACCT)["code"]
    now[0] += TTL + 1
    with pytest.raises(Rejected, match="PAIRING_NOT_FOUND"):
        p.status(code2)
    with pytest.raises(Rejected, match="INVALID_ADDRESS"):
        p.start("GABC")
    with pytest.raises(Rejected, match="INVALID_OWNER_KEY"):
        p.join(p.start(ACCT)["code"], "nothex", "x")
