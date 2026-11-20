"""
Watchdog loop: run every check, decide notices (alerts.transitions), email
them, and write the current status as JSON (WATCHDOG_STATUS_FILE) for any
dashboard. Read-only towards the chain.

Env: CONSTELLA_RPC_URL, CONSTELLA_NETWORK_PASSPHRASE, WATCHDOG_NETWORK (label),
WATCHDOG_SOURCE (any existing account, for simulations), WATCHDOG_VAULTS,
WATCHDOG_ACCOUNTS ("label:G...:min_xlm,..."), RELAYER_HEALTH_URL,
WATCHDOG_INTERVAL_SECONDS (300), WATCHDOG_ONCE, WATCHDOG_STATE_FILE,
WATCHDOG_STATUS_FILE, WATCHDOG_REMIND_HOURS (6), WATCHDOG_CLAIM_MAX_HOURS (72),
WATCHDOG_TTL_MIN_DAYS (7); email: ALERT_SMTP_HOST, ALERT_SMTP_PORT (587),
ALERT_SMTP_TLS (starttls|ssl|none), ALERT_SMTP_USER, ALERT_SMTP_PASSWORD,
ALERT_FROM, ALERT_TO (comma-separated). Without ALERT_SMTP_HOST notices are
only logged.
"""
from __future__ import annotations

import json
import os
import sys
import time
from typing import Dict, List

from constella_watchdog.alerts import Finding, Smtp, deliver, transitions
from constella_watchdog.checks import Reader, check_balance, check_relayer, check_vault


def log(**kw) -> None:
    print(json.dumps(kw, default=str), flush=True)


def _load(path: str) -> Dict:
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def _save(path: str, data: Dict) -> None:
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f, indent=1)
    os.replace(tmp, path)


def parse_accounts(spec: str):
    out = []
    for part in filter(None, (p.strip() for p in spec.split(","))):
        label, account, min_xlm = part.split(":")
        out.append((label, account, float(min_xlm)))
    return out


def run_checks(env, reader: Reader, state: Dict, now: float) -> List[Finding]:
    findings: List[Finding] = []
    for label, account, min_xlm in parse_accounts(env.get("WATCHDOG_ACCOUNTS", "")):
        try:
            findings.append(check_balance(reader, label, account, min_xlm))
        except Exception as e:
            findings.append(Finding(f"balance:{label}", False, f"{label} balance unreadable ({type(e).__name__})"))
    for v in filter(None, (x.strip() for x in env.get("WATCHDOG_VAULTS", "").split(","))):
        try:
            findings += check_vault(reader, v, state, now, float(env.get("WATCHDOG_CLAIM_MAX_HOURS", "72")),
                                    float(env.get("WATCHDOG_TTL_MIN_DAYS", "7")))
        except Exception as e:
            findings.append(Finding(f"vault:{v}", False, f"vault {v[:8]}... unreadable ({type(e).__name__})", str(e)[:300]))
    if env.get("RELAYER_HEALTH_URL"):
        findings.append(check_relayer(env["RELAYER_HEALTH_URL"]))
    return findings


def cli() -> None:
    env = os.environ
    for k in ("CONSTELLA_RPC_URL", "CONSTELLA_NETWORK_PASSPHRASE", "WATCHDOG_SOURCE"):
        if not env.get(k):
            sys.exit(f"missing {k}")
    reader = Reader(env["CONSTELLA_RPC_URL"], env["CONSTELLA_NETWORK_PASSPHRASE"], env["WATCHDOG_SOURCE"])
    smtp = None
    if env.get("ALERT_SMTP_HOST"):
        smtp = Smtp(env["ALERT_SMTP_HOST"], int(env.get("ALERT_SMTP_PORT", "587")), env["ALERT_FROM"],
                    [x.strip() for x in env["ALERT_TO"].split(",") if x.strip()], env.get("ALERT_SMTP_USER"),
                    env.get("ALERT_SMTP_PASSWORD"), env.get("ALERT_SMTP_TLS", "starttls"))
    network = env.get("WATCHDOG_NETWORK", "network")
    state_file = env.get("WATCHDOG_STATE_FILE", "watchdog_state.json")
    status_file = env.get("WATCHDOG_STATUS_FILE", "watchdog_status.json")
    while True:
        state = _load(state_file)
        now = time.time()
        findings = run_checks(env, reader, state, now)
        notices = transitions(findings, state.setdefault("alerts", {}), now, float(env.get("WATCHDOG_REMIND_HOURS", "6")))
        send = smtp.send if smtp else (lambda subject, body: log(action="would-email", subject=subject))
        deliver(notices, send, network, log)
        _save(state_file, state)
        _save(status_file, {"checked_at": now, "ok": all(f.ok for f in findings),
                            "checks": [{"key": f.key, "ok": f.ok, "summary": f.summary} for f in findings]})
        log(action="checked", total=len(findings), failing=sum(not f.ok for f in findings), notices=len(notices))
        if env.get("WATCHDOG_ONCE") == "1":
            return
        time.sleep(int(env.get("WATCHDOG_INTERVAL_SECONDS", "300")))
