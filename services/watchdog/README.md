# Constella watchdog (decision D-M: email alerts)

Read-only operations monitor. Every `WATCHDOG_INTERVAL_SECONDS` it checks, and emails the ops team:

| Check | Fails when |
|---|---|
| `balance:<label>` | a relayer / keeper / feeder account is below its minimum XLM (`WATCHDOG_ACCOUNTS=label:G...:min,...`) |
| `paused:<vault>` | the vault is paused |
| `price:<vault>:<asset>` | a funded non-cash sleeve has no valid oracle price (deposits/redemptions halt) |
| `claims:<vault>:<asset>` | frozen-sleeve claims open longer than `WATCHDOG_CLAIM_MAX_HOURS` (72) |
| `rent:<contract>` | a vault / policy / oracle / registry instance has < `WATCHDOG_TTL_MIN_DAYS` (7) of storage left - the keeper is not renewing |
| `relayer:health` | `RELAYER_HEALTH_URL` does not answer `{"ok": true}` |

Emails: one **ALERT** when a check starts failing, a **REMINDER** every `WATCHDOG_REMIND_HOURS` (6) while it
keeps failing, **RESOLVED** when it passes again; healthy checks send nothing. A mail failure never stops the
checks. The current status is written to `WATCHDOG_STATUS_FILE` (JSON) for dashboards.

SMTP: `ALERT_SMTP_HOST`, `ALERT_SMTP_PORT` (587), `ALERT_SMTP_TLS` (starttls | ssl | none - none only for a
local sink), `ALERT_SMTP_USER`, `ALERT_SMTP_PASSWORD`, `ALERT_FROM`, `ALERT_TO`. Without `ALERT_SMTP_HOST`,
notices are only logged. **Real delivery needs SMTP credentials from the owner.**

Tests: `pytest` (alert state machine: alert once, remind, resolve, independent checks, mail failure);
`services/keeper/tests/localnet_e2e.sh` scenario 8 on a real local network with a local SMTP sink.
