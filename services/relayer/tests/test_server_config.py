"""Relayer configuration parsing (the allow-list is security configuration)."""
import pytest

from constella_relayer.server import parse_allowed


def test_the_allow_list_parses_contracts_and_functions():
    a = "CAAAA:deposit, redeem ;CBBBB:withdraw"
    assert parse_allowed(a) == {"CAAAA": frozenset({"deposit", "redeem"}), "CBBBB": frozenset({"withdraw"})}


@pytest.mark.parametrize("bad", ["", " ; ", "GAAAA:deposit", "CAAAA:", "CAAAA"])
def test_an_empty_or_malformed_allow_list_is_refused(bad):
    with pytest.raises(ValueError):
        parse_allowed(bad)
