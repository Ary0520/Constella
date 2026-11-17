"""Alert state machine and email delivery (decision D-M: email for now).

A check is identified by a stable key. On the first failure an ALERT email
goes out; while it keeps failing, a REMINDER every `remind_hours`; when it
passes again, a RESOLVED email. Nothing is sent for checks that stay healthy.
Delivery failures never stop the checks (they are logged and retried on the
next transition or reminder).
"""
from __future__ import annotations

import smtplib
import ssl
from dataclasses import dataclass
from email.message import EmailMessage
from typing import Callable, Dict, List, Optional


@dataclass(frozen=True)
class Finding:
    key: str          # stable id, e.g. "balance:relayer1"
    ok: bool
    summary: str      # one line, human readable
    detail: str = ""


@dataclass(frozen=True)
class Notice:
    kind: str         # "ALERT" | "REMINDER" | "RESOLVED"
    finding: Finding


def transitions(findings: List[Finding], state: Dict, now: float, remind_hours: float) -> List[Notice]:
    """Pure: which notices to send, updating `state` {key: {"since", "last_sent"}}."""
    out: List[Notice] = []
    for f in findings:
        s = state.get(f.key)
        if not f.ok:
            if s is None:
                state[f.key] = {"since": now, "last_sent": now, "summary": f.summary}
                out.append(Notice("ALERT", f))
            elif now - s["last_sent"] >= remind_hours * 3600:
                s["last_sent"] = now
                out.append(Notice("REMINDER", f))
        elif s is not None:
            del state[f.key]
            out.append(Notice("RESOLVED", f))
    return out


@dataclass
class Smtp:
    host: str
    port: int
    sender: str
    recipients: List[str]
    user: Optional[str] = None
    password: Optional[str] = None
    tls: str = "starttls"  # "starttls" | "ssl" | "none" (local test sinks only)

    def send(self, subject: str, body: str) -> None:
        msg = EmailMessage()
        msg["Subject"], msg["From"], msg["To"] = subject, self.sender, ", ".join(self.recipients)
        msg.set_content(body)
        ctx = ssl.create_default_context()
        if self.tls == "ssl":
            conn = smtplib.SMTP_SSL(self.host, self.port, context=ctx, timeout=30)
        else:
            conn = smtplib.SMTP(self.host, self.port, timeout=30)
        with conn:
            if self.tls == "starttls":
                conn.starttls(context=ctx)
            if self.user:
                conn.login(self.user, self.password or "")
            conn.send_message(msg)


def render(n: Notice, network: str) -> tuple[str, str]:
    f = n.finding
    subject = f"[Constella {network}] {n.kind}: {f.summary}"
    body = f"{n.kind}: {f.summary}\n\ncheck: {f.key}\n{f.detail}\n"
    if n.kind == "RESOLVED":
        body = f"RESOLVED: {f.key} is healthy again.\n{f.summary}\n"
    return subject, body


def deliver(notices: List[Notice], send: Callable[[str, str], None], network: str, log: Callable[..., None]) -> None:
    for n in notices:
        subject, body = render(n, network)
        try:
            send(subject, body)
            log(action="email", kind=n.kind, key=n.finding.key)
        except Exception as e:  # never stop checking because mail failed
            log(action="email-failed", kind=n.kind, key=n.finding.key, reason=type(e).__name__)
