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


def test_cors_is_opt_in_and_answers_only_listed_origins():
    """RELAYER_CORS_ORIGIN is a comma-separated allow-list (the console and the
    product app). The response names the CALLER's origin only when listed
    (Vary: Origin); an unlisted or missing origin gets no CORS header."""
    import json, threading, urllib.request
    from http.server import ThreadingHTTPServer
    from constella_relayer.server import make_handler

    class FakeRelayer:
        keypairs = []

    app, console = "https://constella-app.vercel.app", "https://constella-sandbox.vercel.app"
    cases = [(None, app, None), (f"{console},{app}", app, app), (f"{console}, {app}", console, console),
             (f"{console},{app}", "https://evil.example", None), (f"{console},{app}", None, None)]
    for allowed, origin, expected in cases:
        srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(FakeRelayer(), cors_origin=allowed))
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        req = urllib.request.Request(f"http://127.0.0.1:{srv.server_address[1]}/health", headers={"Origin": origin} if origin else {})
        with urllib.request.urlopen(req) as r:
            assert r.headers.get("Access-Control-Allow-Origin") == expected, (allowed, origin)
            if allowed:
                assert "Origin" in (r.headers.get("Vary") or "")
            assert json.loads(r.read())["ok"] is True
        srv.shutdown()


def test_per_function_fee_caps_parse_and_reject_garbage():
    import pytest
    from constella_relayer.server import parse_fee_caps
    assert parse_fee_caps("add_device:30000000") == {"add_device": 30_000_000}
    assert parse_fee_caps("") == {}
    with pytest.raises(ValueError):
        parse_fee_caps("add_device:lots")


def test_vault_functions_parse_and_are_limited_to_vault_user_calls():
    from constella_relayer.server import parse_vault_functions
    assert parse_vault_functions("deposit, redeem,withdraw,redeem_frozen") == frozenset({"deposit", "redeem", "withdraw", "redeem_frozen"})
    assert parse_vault_functions("") == frozenset()
    for bad in ["set_keeper", "deposit,propose_upgrade", "transfer"]:
        with pytest.raises(ValueError):
            parse_vault_functions(bad)


def test_account_functions_parse_and_are_limited_to_recovery_calls():
    from constella_relayer.server import parse_account_functions
    assert parse_account_functions("propose_recovery,cancel_recovery") == frozenset({"propose_recovery", "cancel_recovery"})
    assert parse_account_functions("") == frozenset()
    for bad in ["add_signer", "propose_recovery,remove_context_rule", "execute_recovery", "add_context_rule"]:
        with pytest.raises(ValueError):
            parse_account_functions(bad)


def test_recovery_routes_collect_approvals_and_submit_once_ready(tmp_path):
    """Server wiring: /recovery/request -> /recovery/approve x threshold -> the
    merged entry is submitted through the relayer's normal sponsored path."""
    import json, threading, urllib.request
    from http.server import ThreadingHTTPServer
    from constella_relayer.recovery import RecoveryMailbox
    from constella_relayer.server import make_handler
    from tests.test_recovery import ACCOUNT, G1, G2, NEW_KEY, call, entry, func, signer_keys

    submitted = []

    class Relayer:  # records what the server hands to the real submit path
        keypairs = []

        def submit(self, func_b64, auth):
            submitted.append((func_b64, auth))
            return type("S", (), {"__dict__": {"hash": "abc", "fee": 1}})()

    box = RecoveryMailbox(str(tmp_path / "r.json"))
    srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(Relayer(), recovery=box))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    def post(path, body):
        req = urllib.request.Request(base + path, json.dumps(body).encode(), {"Content-Type": "application/json"})
        with urllib.request.urlopen(req) as r:
            return json.loads(r.read())

    c = call()
    rid = post("/recovery/request", {"account": ACCOUNT, "new_key": NEW_KEY, "threshold": 2, "func": func(c), "entry": entry(c)})["id"]
    assert post("/recovery/get", {"id": rid})["approvals"] == []
    assert post("/recovery/approve", {"id": rid, "entry": entry(c, signers={G1: b"s1"})})["ready"] is False
    assert submitted == []
    out = post("/recovery/approve", {"id": rid, "entry": entry(c, signers={G2: b"s2"})})
    assert out["ready"] is True and out["submitted"]["hash"] == "abc"
    assert len(submitted) == 1 and submitted[0][0] == func(c) and sorted(signer_keys(submitted[0][1][0])) == sorted([G1, G2])
    assert post("/recovery/get", {"id": rid})["submitted"] == "abc"
    srv.shutdown()
