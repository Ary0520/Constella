"""Step 3: the keeper finds users' own vaults through the factory index
(vault_count / vault_at), remembers what it has seen, and keeps the static
KEEPER_VAULTS list. Only the chain reader is supplied by the test: a table
of what `vault_count` / `vault_at` return, exactly as the contract does."""
from stellar_sdk import scval

from constella_keeper.main import discover_vaults

F1, F2 = "CFACTORY1", "CFACTORY2"


def reader(chain_state, calls):
    def read(contract, fn, *args):
        calls.append((contract, fn, *[scval.to_native(a) for a in args]))
        vaults = chain_state[contract]
        if fn == "vault_count":
            return len(vaults)
        if fn == "vault_at":
            i = scval.to_native(args[0])
            return vaults[i] if i < len(vaults) else None
        raise AssertionError(fn)
    return read


def test_static_vaults_come_first_then_every_factory_vault_in_order():
    calls, state = [], {}
    chain = {F1: ["CV1", "CV2"], F2: ["CV3"]}
    got = discover_vaults(reader(chain, calls), ["CSTATIC"], [F1, F2], state)
    assert got == ["CSTATIC", "CV1", "CV2", "CV3"]


def test_only_new_vaults_are_read_on_later_ticks():
    calls, state = [], {}
    chain = {F1: ["CV1", "CV2"]}
    discover_vaults(reader(chain, calls), [], [F1], state)
    calls.clear()
    chain[F1].append("CV3")
    got = discover_vaults(reader(chain, calls), [], [F1], state)
    assert got == ["CV1", "CV2", "CV3"]
    assert calls == [(F1, "vault_count"), (F1, "vault_at", 2)]


def test_duplicates_are_listed_once():
    calls, state = [], {}
    chain = {F1: ["CV1"]}
    assert discover_vaults(reader(chain, calls), ["CV1"], [F1], state) == ["CV1"]


def test_a_factory_that_cannot_be_read_does_not_drop_known_vaults():
    calls, state = [], {}
    chain = {F1: ["CV1"]}
    discover_vaults(reader(chain, calls), [], [F1], state)

    def broken(contract, fn, *args):
        raise RuntimeError("rpc down")
    assert discover_vaults(broken, ["CS"], [F1], state) == ["CS", "CV1"]
