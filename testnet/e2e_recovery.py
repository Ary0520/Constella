"""Step 8 live proof on Stellar testnet: account recovery through the HOSTED relayer.

Drives the same calls as the console, with software passkeys (real P-256
WebAuthn signatures, verified on chain by the WebAuthn verifier):

  default run
    1. a new smart account (relayer /accounts) with owner passkey O
    2. O installs 2-of-2 guardians G1, G2 (recovery_registry.set_guardians, sponsored)
    3. a new passkey N asks for recovery (/recovery/request); G1 approves -> nothing
       submitted; G2 approves -> the merged entry is submitted (account.propose_recovery)
    4. on chain: get_pending_recovery = N, executable 48 h later; the registry
       refuses to finish early
    5. a second account: recovery proposed, then the OWNER cancels it (sponsored)
  --finish (48 h later)
    /recovery/finish on the saved account -> N is the owner, recovery_registry
    .owner_account(N) = the account, and N signs an owner call.

Test keys of the pending recovery are kept OUTSIDE the repo
(~/.config/constella/testnet/recovery_e2e.json); they are throwaway testnet keys.
"""
from __future__ import annotations

import importlib.util
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

from stellar_sdk import Network, scval, xdr

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "services/relayer/src"))
from constella_relayer.gateway import address_credentials  # noqa: E402
from constella_relayer.passkey import SoftwarePasskey, sign_auth_entry  # noqa: E402

spec = importlib.util.spec_from_file_location("ld", ROOT / "localnet/deploy.py")
ld = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ld)

TESTNET = Network.TESTNET_NETWORK_PASSPHRASE
SAVED = Path.home() / ".config/constella/testnet/recovery_e2e.json"
say = lambda *a: print(*a, flush=True)


def post(url: str, body: dict) -> dict:
    req = urllib.request.Request(url, json.dumps(body).encode(), {"Content-Type": "application/json", "User-Agent": "constella-e2e"})
    try:
        with urllib.request.urlopen(req, timeout=180) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"{url}: {e.code} {e.read().decode()}") from None


def b64(obj) -> str:
    return obj.to_xdr()


def func_b64(contract: str, fn: str, args: list) -> str:
    return xdr.HostFunction(type=xdr.HostFunctionType.HOST_FUNCTION_TYPE_INVOKE_CONTRACT,
                            invoke_contract=xdr.InvokeContractArgs(contract_address=ld.A(contract).address,
                                                                   function_name=xdr.SCSymbol(fn.encode()), args=args)).to_xdr()


def external(verifier: str, key: bytes) -> xdr.SCVal:
    return scval.to_vec([scval.to_symbol("External"), scval.to_address(verifier), scval.to_bytes(key)])


class Ctx:
    def __init__(self):
        self.cfg = json.loads((ROOT / "sandbox/config.testnet.json").read_text())
        self.ct = json.loads((ROOT / "testnet/state/config.json").read_text())["contracts"]
        self.rel = self.cfg["services"]["relayer"]
        self.verifier = self.ct["verifier"]
        self.reg = self.ct["recovery_registry"]
        self.net = ld.Net(self.cfg["network"]["rpc_url"], self.cfg["network"]["friendbot"], TESTNET, allowed=(TESTNET,))
        self.src = json.loads((ROOT / "testnet/state/config.json").read_text())["accounts"]["admin"]

    def view(self, contract, fn, args=()):
        return self.net.view(contract, fn, list(args), self.src)

    def sponsored(self, owner: SoftwarePasskey, account: str, contract: str, fn: str, args: list, rule: int | None = None) -> dict:
        func = func_b64(contract, fn, args)
        prep = post(self.rel + "/prepare", {"func": func})
        rule = rule if rule is not None else int(self.view(account, "get_owner_rule"))
        signed = []
        for e in prep["auth"]:
            entry = xdr.SorobanAuthorizationEntry.from_xdr(e)
            ac = address_credentials(entry.credentials)
            mine = ac is not None and ld.A(account).address.to_xdr() == ac.address.to_xdr()
            signed.append(b64(sign_auth_entry(entry, owner, self.verifier, rule, prep["latest_ledger"] + 60, TESTNET)) if mine else e)
        return post(self.rel + "/submit", {"func": func, "auth": signed})

    def new_account(self, owner: SoftwarePasskey) -> str:
        # Directly on the (permissionless) account factory: the relayer's
        # /accounts path is proven separately and has a per-client daily quota.
        admin = ld.Keypair.from_secret(json.loads((Path.home() / ".config/constella/testnet/keys.json").read_text())["admin"])
        a = self.net.invoke(admin, self.ct["account_factory"], "create_account", [scval.to_bytes(owner.public_key)])
        return getattr(a, "address", str(a))

    def request(self, account: str, new: SoftwarePasskey, threshold: int):
        """(request id, unsigned entry with its expiry fixed, valid_until)."""
        func = func_b64(account, "propose_recovery", [external(self.verifier, new.public_key)])
        prep = post(self.rel + "/prepare", {"func": func})
        mine = [xdr.SorobanAuthorizationEntry.from_xdr(e) for e in prep["auth"]]
        mine = [e for e in mine if (a := address_credentials(e.credentials)) and a.address.to_xdr() == ld.A(account).address.to_xdr()]
        assert len(mine) == 1, "exactly one entry for the account"
        valid_until = prep["latest_ledger"] + 2_000  # ~3 h is plenty for this run
        entry = mine[0]
        address_credentials(entry.credentials).signature_expiration_ledger = xdr.Uint32(valid_until)
        rid = post(self.rel + "/recovery/request", {"account": account, "new_key": new.public_key.hex(), "threshold": threshold,
                                                    "func": func, "entry": b64(entry)})["id"]
        return rid, entry, valid_until

    def approve(self, rid: str, entry, guardian: SoftwarePasskey, account: str, valid_until: int) -> dict:
        rule = int(self.view(self.reg, "guardians_of", [ld.A(account)])["rule_id"])
        signed = sign_auth_entry(entry, guardian, self.verifier, rule, valid_until, TESTNET)
        return post(self.rel + "/recovery/approve", {"id": rid, "entry": b64(signed)})


def run() -> None:
    c = Ctx()
    owner, g1, g2, new = SoftwarePasskey(), SoftwarePasskey(), SoftwarePasskey(), SoftwarePasskey()
    account = c.new_account(owner)
    say("1. account", account)

    keys = scval.to_vec([scval.to_bytes(g1.public_key), scval.to_bytes(g2.public_key)])
    out = c.sponsored(owner, account, c.reg, "set_guardians", [ld.A(account), keys, ld.U32(2)])
    gs = c.view(c.reg, "guardians_of", [ld.A(account)])
    assert gs["threshold"] == 2 and len(gs["guardians"]) == 2, gs
    say("2. guardians set (2-of-2), rule", gs["rule_id"], "tx", out["hash"][:12], "fee", out.get("fee"))

    rid, entry, vu = c.request(account, new, 2)
    a1 = c.approve(rid, entry, g1, account, vu)
    assert a1["ready"] is False and "submitted" not in a1, a1
    assert c.view(account, "get_pending_recovery") is None, "one guardian alone proposes nothing"
    say("3. request", rid, "- guardian 1 approved: not submitted (1 of 2)")
    a2 = c.approve(rid, entry, g2, account, vu)
    assert a2["ready"] is True and a2["submitted"]["hash"], a2
    say("   guardian 2 approved: submitted, tx", a2["submitted"]["hash"][:12], "fee", a2["submitted"].get("fee"))

    pend = c.view(account, "get_pending_recovery")
    assert pend is not None, "a pending recovery on chain"
    nk = pend["new_owner"]
    assert bytes(nk[2]).hex() == new.public_key.hex(), pend
    wait_h = (pend["executable_at"] - time.time()) / 3600
    assert 47 < wait_h <= 48.1, wait_h
    say(f"4. on chain: pending recovery to the new passkey, executable in {wait_h:.1f} h")
    try:
        post(c.rel + "/recovery/finish", {"account": account})
        raise AssertionError("finish before 48 h must fail")
    except RuntimeError as e:
        say("   finishing early is refused:", str(e)[:90])
    assert c.view(c.reg, "owner_account", [scval.to_bytes(new.public_key)]) is None

    SAVED.parent.mkdir(parents=True, exist_ok=True)
    SAVED.write_text(json.dumps({"account": account, "new_pem": new.private_pem().decode(), "owner_pem": owner.private_pem().decode(),
                                 "executable_at": pend["executable_at"]}))
    SAVED.chmod(0o600)

    # 5. the owner cancels a recovery they did not ask for.
    owner2, h1, new2 = SoftwarePasskey(), SoftwarePasskey(), SoftwarePasskey()
    acct2 = c.new_account(owner2)
    c.sponsored(owner2, acct2, c.reg, "set_guardians", [ld.A(acct2), scval.to_vec([scval.to_bytes(h1.public_key)]), ld.U32(1)])
    rid2, entry2, vu2 = c.request(acct2, new2, 1)
    assert c.approve(rid2, entry2, h1, acct2, vu2)["ready"] is True
    assert c.view(acct2, "get_pending_recovery") is not None
    out = c.sponsored(owner2, acct2, acct2, "cancel_recovery", [])
    assert c.view(acct2, "get_pending_recovery") is None
    say("5. second account", acct2, "- recovery proposed, owner cancelled it, tx", out["hash"][:12])
    say("E2E RECOVERY OK (propose + cancel). Run with --finish after",
        time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime(pend["executable_at"])))


def finish() -> None:
    c = Ctx()
    s = json.loads(SAVED.read_text())
    account, new = s["account"], SoftwarePasskey.from_pem(s["new_pem"].encode())
    out = post(c.rel + "/recovery/finish", {"account": account})
    say("finished, tx", out["hash"][:12])
    rec = c.view(c.reg, "owner_account", [scval.to_bytes(new.public_key)])
    assert getattr(rec, "address", str(rec)) == account, rec
    old = SoftwarePasskey.from_pem(s["owner_pem"].encode())
    signers = c.view(account, "get_context_rule", [ld.U32(int(c.view(account, "get_owner_rule")))])["signers"]
    keys = [bytes(x[2]).hex() for x in signers]
    assert new.public_key.hex() in keys and old.public_key.hex() not in keys, keys
    assert c.view(account, "get_pending_recovery") is None
    say("E2E RECOVERY FINISH OK - the new passkey owns", account)


if __name__ == "__main__":
    finish() if "--finish" in sys.argv else run()
