"""
HTTP API for the thesis engine (used by the sandbox console and, later, the
real frontend). stdlib only.

  POST /thesis   {"text", "jurisdiction", "investor_class", "owner"?, "use_llm"?}
       -> draft, allocation, simulation, explanation (+ optional AI summary),
          the exact on-chain bind_policy terms, and - only when `owner` (the
          signing G/C address) is given - the canonical policy + its hash.
          422 {"error": "INFEASIBLE", "reasons", "nearest"?} when the rules
          cannot all hold (the optimizer fails closed).
  GET  /registry?jurisdiction=..&investor_class=..  -> assets + why not eligible
  GET  /yields   -> live rates read from the chain
  GET  /health

Config (env): CONSTELLA_RPC_URL, CONSTELLA_NETWORK_PASSPHRASE, REGISTRY_ID,
ORACLE_ID, READ_SOURCE (any existing account, for simulations), API_HOST
(127.0.0.1), API_PORT (8790), API_CORS_ORIGIN (comma-separated browser origins; http://localhost:5173),
LLM_ENABLED=1 to allow "use_llm" (then OPENROUTER_API_KEY and
LLM_RUN_BUDGET_USD, default 0.05 for the whole process).
"""
from __future__ import annotations

import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Callable, Dict, Optional
from urllib.parse import parse_qs, urlparse

from src.explainer import generate_explanation
from src.optimizer import InfeasibleAllocation, choose_numeraire, optimize_portfolio
from src.parser import parse_thesis
from src.retriever import AssetRegistry, retrieve_candidates
from src.signer import assemble_canonical_policy, compute_policy_hash, onchain_terms, validate_owner
from src.simulator import simulate_portfolio

MAX_BODY = 16 * 1024


# Only used to derive preview terms; never hashed or returned (see Engine.thesis).
_PREVIEW_OWNER = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4"


class Engine:
    def __init__(self, load_registry: Callable[[], AssetRegistry], llm_factory: Optional[Callable] = None,
                 live_yields: Optional[Callable] = None, cache_seconds: float = 60.0,
                 vault_numeraires: tuple = ("USDC",)):
        self.load_registry, self.llm_factory, self.live_yields = load_registry, llm_factory, live_yields
        # Step 4: vault currencies the chain can price today - the NAV oracle's
        # base (vault-factory refuses any other, NumeraireNotOracleBase).
        self.vault_numeraires = tuple(vault_numeraires)
        self.cache_seconds = cache_seconds
        self._reg, self._at = None, 0.0

    def registry(self) -> AssetRegistry:
        if self._reg is None or time.time() - self._at > self.cache_seconds:
            reg = self.load_registry()
            if self.live_yields:
                try:
                    reg.apply_yields(self.live_yields(reg.list_assets()))
                except Exception as e:  # no live rate = unknown, never a guess
                    sys.stderr.write(f"live yields unavailable: {type(e).__name__}\n")
            self._reg, self._at = reg, time.time()
        return self._reg

    def thesis(self, body: Dict) -> tuple[int, Dict]:
        text = body.get("text")
        if not isinstance(text, str) or not text.strip() or len(text) > 4000:
            return 400, {"error": "INVALID_TEXT", "detail": "1-4000 characters"}
        owner = body.get("owner")
        if owner is not None:
            try:
                validate_owner(owner)
            except ValueError:
                return 400, {"error": "INVALID_OWNER", "detail": "a G account or C smart-account address"}
        jur = str(body.get("jurisdiction", "")).strip()
        cls = str(body.get("investor_class", "retail")).strip()
        if not jur:
            return 400, {"error": "JURISDICTION_REQUIRED", "detail": "from onboarding, never from the text"}
        source, ai_note = "rules", None
        if body.get("use_llm") and self.llm_factory:
            from src.thesis_llm import parse_thesis_llm
            draft, source = parse_thesis_llm(text, self.llm_factory(), jurisdiction=jur, investor_class=cls)
        else:
            draft = parse_thesis(text, jurisdiction=jur, investor_class=cls)
            if body.get("use_llm"):
                ai_note = "AI reading is disabled on this server; used the rule-based parser."
        reg = self.registry()
        cands = retrieve_candidates(draft, reg)
        ids = [c.asset_id for c in cands]
        wanted = choose_numeraire(draft, cands)
        numeraire = wanted if wanted in self.vault_numeraires else next((n for n in self.vault_numeraires if n in ids), None)
        note = ([f"a {wanted} vault needs a price feed based on {wanted} on chain (coming); your vault currency is "
                 f"{numeraire} - euro holdings are kept within your caps"] if wanted and wanted != numeraire else [])
        try:
            opt = optimize_portfolio(draft, cands, numeraire=numeraire)
        except InfeasibleAllocation as e:
            return 422, {"error": "INFEASIBLE", "reasons": note + e.reasons, "relaxed": e.relaxed,
                         "nearest": e.nearest.model_dump() if e.nearest else None, "draft": draft.model_dump()}
        except ValueError as e:
            return 422, {"error": "INFEASIBLE", "reasons": [str(e)], "draft": draft.model_dump()}
        sim = simulate_portfolio(opt.weights, cands, draft)
        exp = generate_explanation(text, draft, opt, cands)
        summary = None
        if body.get("use_llm") and self.llm_factory:
            from src.thesis_llm import explain_llm
            summary, ai_note = explain_llm(draft, opt, exp, cands, self.llm_factory())
        # OPEN-ISSUES #3: the hash commits to the real owner and to exactly the
        # terms enforced on chain. Without an owner the response is a preview:
        # the same terms, but no signable hash.
        canonical = assemble_canonical_policy(draft, opt, cands, owner=owner or _PREVIEW_OWNER)
        hex_hash = compute_policy_hash(canonical)[0] if owner else None
        return 200, {
            "parse_source": source, "ai_note": ai_note, "draft": draft.model_dump(), "allocation": opt.model_dump(),
            "simulation": sim.model_dump(), "explanation": exp.model_dump(), "ai_summary": summary,
            "policy": json.loads(canonical.model_dump_json()) if owner else None, "policy_hash": hex_hash,
            "signable": bool(owner), "bind_policy": onchain_terms(canonical),
            "numeraire_contract": next(c.contract_address for c in cands if c.asset_id == opt.numeraire),
            "ineligible": reg.eligibility(jur, cls),
        }


def make_handler(engine: Engine, cors_origin: str):
    # Comma-separated allow-list of browser origins (the console, the app).
    allowed = frozenset(o.strip().rstrip("/") for o in (cors_origin or "").split(",") if o.strip())

    class Handler(BaseHTTPRequestHandler):
        server_version = "constella-thesis-api"

        def log_message(self, fmt, *args):  # never log request bodies (theses are private)
            sys.stderr.write("%s %s\n" % (self.address_string(), fmt % args))

        def _reply(self, status: int, body: Dict) -> None:
            data = json.dumps(body, default=str).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Vary", "Origin")
            origin = (self.headers.get("Origin") or "").rstrip("/")
            if origin in allowed:  # answer only a listed caller origin
                self.send_header("Access-Control-Allow-Origin", origin)
                self.send_header("Access-Control-Allow-Headers", "Content-Type")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_OPTIONS(self):
            self._reply(204, {})

        def do_GET(self):
            url = urlparse(self.path)
            try:
                if url.path == "/health":
                    return self._reply(200, {"ok": True, "llm": engine.llm_factory is not None})
                if url.path == "/registry":
                    q = parse_qs(url.query)
                    reg = engine.registry()
                    out = {"assets": [a.model_dump() for a in reg.list_assets()]}
                    if q.get("jurisdiction"):
                        out["ineligible"] = reg.eligibility(q["jurisdiction"][0], q.get("investor_class", ["retail"])[0])
                    return self._reply(200, out)
                if url.path == "/yields":
                    reg = engine.registry()
                    return self._reply(200, {a.asset_id: a.current_yield_pct for a in reg.list_assets() if a.current_yield_pct > 0})
                return self._reply(404, {"error": "NOT_FOUND"})
            except Exception as e:
                sys.stderr.write(f"internal error: {type(e).__name__}: {e}\n")
                return self._reply(500, {"error": "INTERNAL", "detail": type(e).__name__})

        def do_POST(self):
            if urlparse(self.path).path != "/thesis":
                return self._reply(404, {"error": "NOT_FOUND"})
            try:
                n = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                n = -1
            if n <= 0 or n > MAX_BODY:
                return self._reply(400, {"error": "INVALID_PARAMS", "detail": "body size"})
            try:
                body = json.loads(self.rfile.read(n))
                assert isinstance(body, dict)
            except Exception:
                return self._reply(400, {"error": "INVALID_PARAMS", "detail": "JSON object expected"})
            try:
                status, out = engine.thesis(body)
                return self._reply(status, out)
            except ValueError as e:  # e.g. unknown investor class
                return self._reply(400, {"error": "INVALID_PARAMS", "detail": str(e)[:200]})
            except Exception as e:
                sys.stderr.write(f"internal error: {type(e).__name__}: {e}\n")
                return self._reply(500, {"error": "INTERNAL", "detail": type(e).__name__})

    return Handler


def cli() -> None:
    env = os.environ
    rpc, pas, src = env["CONSTELLA_RPC_URL"], env["CONSTELLA_NETWORK_PASSPHRASE"], env["READ_SOURCE"]
    load = lambda: AssetRegistry.from_chain(rpc, pas, env["REGISTRY_ID"], src)
    yields = None
    if env.get("ORACLE_ID"):
        from src.yields import live_lending_yields
        yields = lambda cands: live_lending_yields(rpc, pas, src, env["ORACLE_ID"], cands)
    llm = None
    if env.get("LLM_ENABLED") == "1":
        from src.llm import OpenRouterClient, SpendGuard
        guard = SpendGuard(float(env.get("LLM_RUN_BUDGET_USD", "0.05")))  # shared by every request
        llm = lambda: OpenRouterClient(guard)
    engine = Engine(load, llm, yields)
    host, port = env.get("API_HOST", "127.0.0.1"), int(env.get("API_PORT", "8790"))
    sys.stderr.write(f"constella thesis API on {host}:{port} (LLM {'on' if llm else 'off'})\n")
    ThreadingHTTPServer((host, port), make_handler(engine, env.get("API_CORS_ORIGIN", "http://localhost:5173"))).serve_forever()


if __name__ == "__main__":
    cli()
