"""HTTP API for the sponsor relayer.

  POST /prepare  {"func": b64}                 -> {"auth": [b64], "latest_ledger": n}
  POST /submit   {"func": b64, "auth": [b64]}  -> {"hash", "user", "fee_charged", ...}
  GET  /health                                 -> {"ok": true, "relayer": G...}

Errors: HTTP 400 {"error": CODE, "detail": ...} with the gateway's stable codes.
Binds to 127.0.0.1 by default: put it behind your API gateway / rate limiter,
never expose it directly (same advice as the OpenZeppelin Relayer docs).

Configuration (environment only - the secret is never logged):
  CONSTELLA_RPC_URL, CONSTELLA_NETWORK_PASSPHRASE,
  RELAYER_SECRETS (comma-separated pool, D-I; each must be in the fee-sponsor's
  relayer set) or RELAYER_SECRET (pool of one),
  SPONSOR_CONTRACT_ID, SPONSOR_ALLOWED ("C...:deposit,redeem;C...:fn"),
  RELAYER_MAX_FEE_STROOPS (default 1_000_000), RELAYER_HOST, RELAYER_PORT.
"""
from __future__ import annotations

import json
import os
import sys
from dataclasses import asdict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from stellar_sdk import Keypair, SorobanServer

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


def make_handler(relayer: Relayer):
    class Handler(BaseHTTPRequestHandler):
        server_version = "constella-relayer"

        def log_message(self, fmt, *args):  # no request bodies in logs
            sys.stderr.write("%s %s\n" % (self.address_string(), fmt % args))

        def _reply(self, status: int, body: dict) -> None:
            data = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            if self.path == "/health":
                self._reply(200, {"ok": True, "relayers": [k.public_key for k in relayer.keypairs]})
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
                return self._reply(404, {"error": "NOT_FOUND"})
            except Rejected as e:
                return self._reply(400, {"error": e.code, "detail": e.detail})
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
    )
    host, port = env.get("RELAYER_HOST", "127.0.0.1"), int(env.get("RELAYER_PORT", "8787"))
    sys.stderr.write(f"constella-relayer pool of {len(relayer.keypairs)} on {host}:{port}\n")
    ThreadingHTTPServer((host, port), make_handler(relayer)).serve_forever()


if __name__ == "__main__":
    cli()
