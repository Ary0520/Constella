"""HTTP API for the sponsor relayer.

  POST /prepare  {"func": b64}                 -> {"auth": [b64], "latest_ledger": n}
  POST /submit   {"func": b64, "auth": [b64]}  -> {"hash", "user", "fee_charged", ...}
  POST /accounts {"owner_key": hex}            -> {"account": C..., "existing": bool, ...}
  GET  /health                                 -> {"ok": true, "relayer": G...}

Errors: HTTP 400 {"error": CODE, "detail": ...} with the gateway's stable codes.
Binds to 127.0.0.1 by default: put it behind your API gateway / rate limiter,
never expose it directly (same advice as the OpenZeppelin Relayer docs).

Configuration (environment only - the secret is never logged):
  CONSTELLA_RPC_URL, CONSTELLA_NETWORK_PASSPHRASE,
  RELAYER_SECRETS (comma-separated pool, D-I; each must be in the fee-sponsor's
  relayer set) or RELAYER_SECRET (pool of one),
  SPONSOR_CONTRACT_ID, SPONSOR_ALLOWED ("C...:deposit,redeem;C...:fn"),
  SPONSOR_VAULT_FUNCTIONS ("deposit,redeem,withdraw,redeem_frozen": also allowed on
  any vault the fee-sponsor proves via a trusted factory; empty = off),
  RELAYER_MAX_FEE_STROOPS (default 1_000_000), RELAYER_HOST, RELAYER_PORT,
  ACCOUNT_FACTORY_ID (enables /accounts, audit A2), ACCOUNTS_PER_IP_PER_DAY (3),
  SPONSOR_ACCOUNT_FUNCTIONS ("propose_recovery,cancel_recovery": on accounts the
  fee-sponsor proves via a trusted account factory; empty = off), RECOVERY_REGISTRY_ID
  (enables /recovery/finish), RECOVERY_STATE_FILE (guardian approvals; /data/recovery.json),
  RELAYER_FN_MAX_FEE ("add_device:30000000": per-function caps for calls that
  create storage), ACCOUNTS_PER_DAY (200), ACCOUNT_MAX_FEE_STROOPS (30_000_000: an account
  deployment pays its storage rent), RELAYER_TRUST_PROXY=1 (use X-Forwarded-For's first
  address as the client; only behind your own proxy), RELAYER_CORS_ORIGIN
  (unset = no CORS headers; a comma-separated list of browser origins, e.g. the console and the app),
  FAUCET_SECRET + FAUCET_ASSET (enables POST /faucet {"address"} on TEST
  networks only; FAUCET_AMOUNT 1_000_000_000 = 100 units, per address 1/day,
  per IP 3/day, 200/day total).
"""
from __future__ import annotations

import json
import os
import sys
from dataclasses import asdict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from stellar_sdk import Keypair, SorobanServer

from .accounts import RateLimiter, parse_owner_key
from .faucet import Faucet, parse_address
from .pairing import Pairing
from .recovery import RecoveryMailbox
from .gateway import Rejected
from .relayer import Relayer

MAX_BODY_BYTES = 64 * 1024


def parse_allowed(spec: str) -> dict[str, frozenset[str]]:
    allowed: dict[str, frozenset[str]] = {}
    for part in filter(None, (p.strip() for p in spec.split(";"))):
        contract, _, fns = part.partition(":")
        names = frozenset(f.strip() for f in fns.split(",") if f.strip())
        if not contract.startswith("C") or not names:
            raise ValueError(f"bad SPONSOR_ALLOWED entry: {part!r}")
        allowed[contract.strip()] = names
    if not allowed:
        raise ValueError("SPONSOR_ALLOWED is empty")
    return allowed


# Step 3: the only functions that may be sponsored on users' own factory
# vaults (new addresses, proven on chain by a trusted factory).
VAULT_USER_FUNCTIONS = frozenset({"deposit", "redeem", "withdraw", "redeem_frozen"})


def parse_vault_functions(spec: str) -> frozenset[str]:
    names = frozenset(f.strip() for f in spec.split(",") if f.strip())
    if names - VAULT_USER_FUNCTIONS:
        raise ValueError(f"SPONSOR_VAULT_FUNCTIONS may only contain {sorted(VAULT_USER_FUNCTIONS)}")
    return names


# Step 8: the only account functions that may be sponsored on users' own smart
# accounts (proven on chain by a trusted account factory). Finishing a recovery
# goes through the recovery registry (relayer-built, /recovery/finish).
ACCOUNT_RECOVERY_FUNCTIONS = frozenset({"propose_recovery", "cancel_recovery"})


def parse_account_functions(spec: str) -> frozenset[str]:
    names = frozenset(f.strip() for f in spec.split(",") if f.strip())
    if names - ACCOUNT_RECOVERY_FUNCTIONS:
        raise ValueError(f"SPONSOR_ACCOUNT_FUNCTIONS may only contain {sorted(ACCOUNT_RECOVERY_FUNCTIONS)}")
    return names


def parse_fee_caps(spec: str) -> dict[str, int]:
    """'fn:stroops,fn:stroops' -> {fn: stroops} (per-function fee caps)."""
    out: dict[str, int] = {}
    for part in filter(None, (p.strip() for p in spec.split(","))):
        fn, _, cap = part.partition(":")
        if not fn or not cap.isdigit():
            raise ValueError(f"bad RELAYER_FN_MAX_FEE entry: {part!r}")
        out[fn.strip()] = int(cap)
    return out


def make_handler(relayer: Relayer, factory: str | None = None, limiter: RateLimiter | None = None,
                 *args, **kwargs):
    return _make_handler(relayer, factory, limiter, *args, **kwargs)


def _make_handler(relayer: Relayer, factory: str | None = None, limiter: RateLimiter | None = None,
                 trust_proxy: bool = False, cors_origin: str | None = None, faucet: "Faucet | None" = None,
                 pairing: Pairing | None = None, recovery: "RecoveryMailbox | None" = None,
                 recovery_registry: str | None = None):
    pairing = pairing or Pairing()
    # Comma-separated allow-list of browser origins (the console, the app).
    cors_allowed = frozenset(o.strip().rstrip("/") for o in (cors_origin or "").split(",") if o.strip())

    class Handler(BaseHTTPRequestHandler):
        server_version = "constella-relayer"

        def log_message(self, fmt, *args):  # no request bodies in logs
            sys.stderr.write("%s %s\n" % (self.address_string(), fmt % args))

        def _reply(self, status: int, body: dict) -> None:
            data = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            if cors_allowed:  # opt-in: answer only a listed caller origin
                self.send_header("Vary", "Origin")
                origin = (self.headers.get("Origin") or "").rstrip("/")
                if origin in cors_allowed:
                    self.send_header("Access-Control-Allow-Origin", origin)
                    self.send_header("Access-Control-Allow-Headers", "Content-Type")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_OPTIONS(self):
            self._reply(204, {})

        def do_GET(self):
            if self.path == "/health":
                self._reply(200, {"ok": True, "relayers": [k.public_key for k in relayer.keypairs], "faucet": faucet is not None})
            else:
                self._reply(404, {"error": "NOT_FOUND"})

        def do_POST(self):
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                length = -1
            if length <= 0 or length > MAX_BODY_BYTES:
                return self._reply(400, {"error": "INVALID_PARAMS", "detail": "body size"})
            try:
                body = json.loads(self.rfile.read(length))
                if not isinstance(body, dict):
                    raise ValueError("not an object")
            except ValueError:
                return self._reply(400, {"error": "INVALID_PARAMS", "detail": "body must be a JSON object"})
            try:
                if self.path == "/prepare":
                    return self._reply(200, relayer.prepare(body.get("func")))
                if self.path == "/submit":
                    return self._reply(200, asdict(relayer.submit(body.get("func"), body.get("auth"))))
                if self.path == "/pair/start":
                    return self._reply(200, pairing.start(body.get("account")))
                if self.path == "/pair/join":
                    return self._reply(200, pairing.join(body.get("code"), body.get("device_key"), body.get("name")))
                if self.path == "/pair/status":
                    return self._reply(200, pairing.status(body.get("code")))
                if self.path.startswith("/recovery/") and recovery is not None:
                    if self.path == "/recovery/request":
                        return self._reply(200, recovery.open(body.get("account"), body.get("new_key"), body.get("threshold"),
                                                              body.get("func"), body.get("entry")))
                    if self.path == "/recovery/get":
                        return self._reply(200, recovery.get(body.get("id")))
                    if self.path == "/recovery/approve":
                        out = recovery.approve(body.get("id"), body.get("entry"))
                        if out["ready"] and not recovery.get(out["id"])["submitted"]:
                            done = dict(vars(relayer.submit(out["func"], [out["entry"]])))
                            recovery.mark_submitted(out["id"], done["hash"])
                            out["submitted"] = done
                        return self._reply(200, out)
                    if self.path == "/recovery/finish" and recovery_registry:
                        return self._reply(200, relayer.finish_recovery(recovery_registry, body.get("account")))
                if self.path == "/faucet" and faucet:
                    client = self.client_address[0]
                    if trust_proxy and self.headers.get("X-Forwarded-For"):
                        client = self.headers["X-Forwarded-For"].split(",")[0].strip()
                    return self._reply(200, faucet.send(parse_address(body.get("address")), client))
                if self.path == "/accounts" and factory and limiter:
                    key = parse_owner_key(body.get("owner_key"))
                    client = self.client_address[0]
                    if trust_proxy and self.headers.get("X-Forwarded-For"):
                        client = self.headers["X-Forwarded-For"].split(",")[0].strip()
                    limiter.check(client)
                    out = relayer.create_account(factory, key)
                    if not out["existing"]:
                        limiter.count(client)
                    return self._reply(200, out)
                return self._reply(404, {"error": "NOT_FOUND"})
            except Rejected as e:
                return self._reply(429 if e.code == "RATE_LIMITED" else 400, {"error": e.code, "detail": e.detail})
            except Exception as e:  # never leak internals beyond a type name
                sys.stderr.write(f"internal error: {type(e).__name__}: {e}\n")
                return self._reply(500, {"error": "INTERNAL", "detail": type(e).__name__})

    return Handler


def cli() -> None:
    env = os.environ
    relayer = Relayer(
        server=SorobanServer(env["CONSTELLA_RPC_URL"]),
        network_passphrase=env["CONSTELLA_NETWORK_PASSPHRASE"],
        keypairs=[Keypair.from_secret(x.strip()) for x in env.get("RELAYER_SECRETS", env.get("RELAYER_SECRET", "")).split(",") if x.strip()],
        sponsor_contract=env["SPONSOR_CONTRACT_ID"],
        allowed=parse_allowed(env["SPONSOR_ALLOWED"]),
        max_fee_stroops=int(env.get("RELAYER_MAX_FEE_STROOPS", "1000000")),
        account_max_fee_stroops=int(env.get("ACCOUNT_MAX_FEE_STROOPS", "30000000")),
        function_max_fee=parse_fee_caps(env.get("RELAYER_FN_MAX_FEE", "add_device:30000000")),
        vault_functions=parse_vault_functions(env.get("SPONSOR_VAULT_FUNCTIONS", ""))
        | parse_account_functions(env.get("SPONSOR_ACCOUNT_FUNCTIONS", "")),
    )
    host, port = env.get("RELAYER_HOST", "127.0.0.1"), int(env.get("RELAYER_PORT", "8787"))
    sys.stderr.write(f"constella-relayer pool of {len(relayer.keypairs)} on {host}:{port}\n")
    factory = env.get("ACCOUNT_FACTORY_ID") or None
    limiter = RateLimiter(int(env.get("ACCOUNTS_PER_IP_PER_DAY", "3")), int(env.get("ACCOUNTS_PER_DAY", "200")))
    faucet = None
    if env.get("FAUCET_SECRET"):  # test networks only (the Faucet refuses others)
        faucet = Faucet(relayer.server, env["CONSTELLA_NETWORK_PASSPHRASE"], env["FAUCET_SECRET"], env["FAUCET_ASSET"],
                        int(env.get("FAUCET_AMOUNT", "1000000000")),
                        RateLimiter(int(env.get("FAUCET_PER_ADDRESS_PER_DAY", "1")), int(env.get("FAUCET_PER_DAY", "200"))),
                        RateLimiter(int(env.get("FAUCET_PER_IP_PER_DAY", "3")), int(env.get("FAUCET_PER_DAY", "200"))))
    # Step 8: account recovery - guardian approvals persist across restarts.
    recovery_registry = env.get("RECOVERY_REGISTRY_ID") or None
    recovery = RecoveryMailbox(env.get("RECOVERY_STATE_FILE") or ("/data/recovery.json" if os.path.isdir("/data") else "recovery.json"))
    handler = make_handler(relayer, factory, limiter, env.get("RELAYER_TRUST_PROXY") == "1", env.get("RELAYER_CORS_ORIGIN") or None, faucet,
                           recovery=recovery, recovery_registry=recovery_registry)
    ThreadingHTTPServer((host, port), handler).serve_forever()


if __name__ == "__main__":
    cli()
