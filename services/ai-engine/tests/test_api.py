"""The thesis API over real HTTP (in-process server, fixture registry in the
on-chain entry shape; no LLM)."""
import json
import threading
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer

import pytest

from src.api import Engine, make_handler
from tests.fixtures import fixture_registry


@pytest.fixture(scope="module")
def base():
    srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(Engine(fixture_registry), "http://localhost:5173"))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{srv.server_address[1]}"
    srv.shutdown()


def call(base, path, body=None):
    req = urllib.request.Request(base + path, data=json.dumps(body).encode() if body is not None else None,
                                 headers={"Content-Type": "application/json"}, method="POST" if body is not None else "GET")
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, json.loads(r.read()), dict(r.headers)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read()), dict(e.headers)


def test_a_thesis_becomes_a_signed_policy_with_exact_on_chain_terms(base):
    status, out, headers = call(base, "/thesis", {"text": "Keep a safe runway in dollars, at least 10% instant cash.",
                                                  "jurisdiction": "DE", "investor_class": "retail",
                                                  "owner": "GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR"})
    assert status == 200, out
    assert out["signable"] is True and out["policy"]["admin"]["admin_address"] == "GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR"
    assert headers["Access-Control-Allow-Origin"] == "http://localhost:5173"
    alloc = out["allocation"]
    assert alloc["profile"] == "cash_reserve" and alloc["numeraire"] == "USDC"
    assert alloc["weights_bps"]["USDC"] >= 8500
    bind = out["bind_policy"]
    kinds = {s["asset_id"]: s["sleeve_type"] for s in bind["sleeves"]}
    assert kinds["USDC"] == "numeraire"                       # A1b on chain
    assert sum(s["target_weight_bps"] for s in bind["sleeves"]) == 10_000
    assert all(s["min_band_bps"] <= s["target_weight_bps"] <= s["max_band_bps"] for s in bind["sleeves"])
    assert bind["min_liquidity_sleeve_bps"] == 1000
    assert len(out["policy_hash"]) == 64 and out["parse_source"] == "rules"
    assert "FRANKLIN_BENJI" in out["ineligible"], "retail: permissionless only (D-B)"


def test_jurisdiction_comes_from_onboarding_and_bad_input_is_refused(base):
    assert call(base, "/thesis", {"text": "safe runway"})[1]["error"] == "JURISDICTION_REQUIRED"
    assert call(base, "/thesis", {"text": "", "jurisdiction": "DE"})[1]["error"] == "INVALID_TEXT"
    assert call(base, "/thesis", {"text": "safe runway", "jurisdiction": "DE", "investor_class": "whale"})[0] == 400


def test_a_sanctioned_jurisdiction_gets_no_allocation(base):
    status, out, _ = call(base, "/thesis", {"text": "safe runway in dollars", "jurisdiction": "IR", "investor_class": "retail"})
    assert status == 422 and out["error"] == "INFEASIBLE"


def test_llm_is_off_unless_the_server_enables_it(base):
    status, out, _ = call(base, "/thesis", {"text": "safe runway in dollars", "jurisdiction": "DE", "use_llm": True})
    assert status == 200 and out["parse_source"] == "rules" and "disabled" in out["ai_note"]


def test_registry_and_health(base):
    s, out, _ = call(base, "/registry?jurisdiction=US&investor_class=retail")
    assert s == 200 and {a["asset_id"] for a in out["assets"]} >= {"USDC", "EURC"}
    assert "ONDO_USDY" in out["ineligible"]
    assert call(base, "/health")[1] == {"ok": True, "llm": False}
