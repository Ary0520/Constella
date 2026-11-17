from constella_watchdog.alerts import Finding, Notice, deliver, render, transitions


def F(ok, key="balance:relayer1"):
    return Finding(key, ok, "relayer1 balance 3.00 XLM (minimum 50)")


def test_alert_once_remind_later_resolve_when_healthy():
    st = {}
    assert [n.kind for n in transitions([F(True)], st, 0, 6)] == []
    assert [n.kind for n in transitions([F(False)], st, 0, 6)] == ["ALERT"]
    assert transitions([F(False)], st, 3600, 6) == []                       # no spam
    assert [n.kind for n in transitions([F(False)], st, 6 * 3600, 6)] == ["REMINDER"]
    assert transitions([F(False)], st, 7 * 3600, 6) == []
    assert [n.kind for n in transitions([F(True)], st, 8 * 3600, 6)] == ["RESOLVED"]
    assert st == {}
    assert transitions([F(True)], st, 9 * 3600, 6) == []


def test_independent_checks_have_independent_state():
    st = {}
    ns = transitions([F(False, "a"), F(True, "b"), F(False, "c")], st, 0, 6)
    assert sorted(n.finding.key for n in ns) == ["a", "c"]
    ns = transitions([F(True, "a"), F(True, "b"), F(False, "c")], st, 10, 6)
    assert [(n.kind, n.finding.key) for n in ns] == [("RESOLVED", "a")]


def test_mail_failure_never_stops_checking():
    logs = []
    def boom(subject, body):
        raise ConnectionRefusedError()
    deliver([Notice("ALERT", F(False))], boom, "testnet", lambda **k: logs.append(k))
    assert logs == [{"action": "email-failed", "kind": "ALERT", "key": "balance:relayer1", "reason": "ConnectionRefusedError"}]


def test_subject_names_network_kind_and_problem():
    s, b = render(Notice("ALERT", F(False)), "testnet")
    assert s == "[Constella testnet] ALERT: relayer1 balance 3.00 XLM (minimum 50)"
    assert "balance:relayer1" in b
