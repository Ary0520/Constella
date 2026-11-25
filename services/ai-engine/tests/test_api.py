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
                                 headers={"Content-Type": "application/json", "Origin": "http://localhost:5173"}, method="POST" if body is not None else "GET")
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
    assert alloc["profile"] == "thesis" and alloc["numeraire"] == "USDC"
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


# --- Step 4: thesis-driven baskets through the API -------------------------------------
OWNER = "GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR"


def test_the_response_names_the_vault_currency_contract(base):
    status, out, _ = call(base, "/thesis", {"text": "Keep a safe runway in dollars with at least 10% instant cash.",
                                            "jurisdiction": "DE", "owner": OWNER})
    assert status == 200
    from tests.fixtures import USDC_MAINNET
    assert out["allocation"]["numeraire"] == "USDC" and out["numeraire_contract"] == USDC_MAINNET


def test_different_theses_give_different_signed_terms(base):
    terms = []
    for text in ["Keep a safe runway in dollars with at least 10% instant cash.",
                 "Keep a safe runway in dollars, no lending.",
                 "Keep 18 months of runway safe, hold a third in euro. Up to 40% per issuer and 40% per asset."]:
        status, out, _ = call(base, "/thesis", {"text": text, "jurisdiction": "DE", "owner": OWNER})
        assert status == 200, out
        terms.append(tuple(sorted((s["asset_id"], s["target_weight_bps"]) for s in out["bind_policy"]["sleeves"])))
    assert len(set(terms)) == 3
    eur = dict(terms[2])
    assert 2833 <= eur["EURC"] <= 3833


def test_a_mostly_euro_thesis_stays_in_a_usdc_vault_and_says_why(base):
    # The chain's price feed is USDC-based (vault-factory NumeraireNotOracleBase),
    # so the API never offers a euro vault yet: it explains and offers the nearest mix.
    status, out, _ = call(base, "/thesis", {"text": "Keep 70% in euros and the rest in dollars.", "jurisdiction": "DE", "owner": OWNER})
    assert status == 422 and out["error"] == "INFEASIBLE"
    assert any("euro" in r.lower() and "price feed" in r.lower() for r in out["reasons"])
    assert out["nearest"]["numeraire"] == "USDC"


def test_cors_answers_only_a_listed_origin():
    """API_CORS_ORIGIN is a comma-separated allow-list (console + product app);
    the caller's origin is echoed only when listed (Vary: Origin)."""
    app, console = "https://constella-app.vercel.app", "https://constella-sandbox.vercel.app"
    srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(Engine(fixture_registry), f"{console}, {app}"))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{srv.server_address[1]}/health"
    try:
        for origin, expected in [(app, app), (console, console), ("https://evil.example", None), (None, None)]:
            req = urllib.request.Request(url, headers={"Origin": origin} if origin else {})
            with urllib.request.urlopen(req) as r:
                assert r.headers.get("Access-Control-Allow-Origin") == expected, origin
                assert "Origin" in (r.headers.get("Vary") or "")
    finally:
        srv.shutdown()
