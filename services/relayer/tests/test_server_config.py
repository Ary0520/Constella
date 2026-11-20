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


def test_cors_is_opt_in_and_limited_to_one_origin():
    import json, threading, urllib.request
    from http.server import ThreadingHTTPServer
    from constella_relayer.server import make_handler

    class FakeRelayer:
        keypairs = []

    for origin, expected in [(None, None), ("http://localhost:5173", "http://localhost:5173")]:
        srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(FakeRelayer(), cors_origin=origin))
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        with urllib.request.urlopen(f"http://127.0.0.1:{srv.server_address[1]}/health") as r:
            assert r.headers.get("Access-Control-Allow-Origin") == expected
            assert json.loads(r.read())["ok"] is True
        srv.shutdown()


def test_per_function_fee_caps_parse_and_reject_garbage():
    import pytest
    from constella_relayer.server import parse_fee_caps
    assert parse_fee_caps("add_device:30000000") == {"add_device": 30_000_000}
    assert parse_fee_caps("") == {}
    with pytest.raises(ValueError):
        parse_fee_caps("add_device:lots")
