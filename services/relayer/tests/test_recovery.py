"""Step 8: the guardian-approval mailbox (M-of-N recovery), on real Soroban XDR.

A recovery proposal is ONE authorization entry of the user's account
(root invocation account.propose_recovery(new_owner)); each guardian signs the
same entry with its passkey under the recovery rule. The mailbox accepts only
signatures over exactly that entry and merges them (signer maps, in the
order the chain requires) until the threshold is reached. Nothing here can
recover an account: the account checks the guardians and the 48-hour delay."""
from __future__ import annotations

import base64
import os

import pytest
from stellar_sdk import Address, StrKey, scval, xdr

from constella_relayer.gateway import Rejected
from constella_relayer.recovery import RecoveryMailbox, merge_signed

ACCOUNT = StrKey.encode_contract(os.urandom(32))
OTHER = StrKey.encode_contract(os.urandom(32))
VERIFIER = StrKey.encode_contract(os.urandom(32))
NEW_KEY = "04" + "11" * 64


def b64(obj) -> str:
    return base64.b64encode(obj.to_xdr_bytes()).decode()


def signer(key_hex: str) -> xdr.SCVal:
    return scval.to_vec([scval.to_symbol("External"), scval.to_address(VERIFIER), scval.to_bytes(bytes.fromhex(key_hex))])


def call(account=ACCOUNT, fn="propose_recovery") -> xdr.InvokeContractArgs:
    return xdr.InvokeContractArgs(contract_address=Address(account).to_xdr_sc_address(), function_name=xdr.SCSymbol(fn.encode()),
                                  args=[signer(NEW_KEY)])


def func(c) -> str:
    return b64(xdr.HostFunction(type=xdr.HostFunctionType.HOST_FUNCTION_TYPE_INVOKE_CONTRACT, invoke_contract=c))


def entry(c, nonce=7, signers=None, rule=3) -> str:
    smap = lambda entries: xdr.SCVal(xdr.SCValType.SCV_MAP, map=xdr.SCMap([xdr.SCMapEntry(k, v) for k, v in entries]))
    sig = scval.to_void() if signers is None else smap([
        (scval.to_symbol("context_rule_ids"), scval.to_vec([scval.to_uint32(rule)])),
        (scval.to_symbol("signers"), smap([(signer(k), scval.to_bytes(v)) for k, v in signers.items()])),
    ])
    creds = xdr.SorobanAddressCredentials(address=Address(ACCOUNT).to_xdr_sc_address(), nonce=xdr.Int64(nonce),
                                          signature_expiration_ledger=xdr.Uint32(5_000), signature=sig)
    return b64(xdr.SorobanAuthorizationEntry(
        credentials=xdr.SorobanCredentials(type=xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS, address=creds),
        root_invocation=xdr.SorobanAuthorizedInvocation(
            function=xdr.SorobanAuthorizedFunction(
                type=xdr.SorobanAuthorizedFunctionType.SOROBAN_AUTHORIZED_FUNCTION_TYPE_CONTRACT_FN, contract_fn=c),
            sub_invocations=[])))


G1, G2, G3 = "04" + "aa" * 64, "04" + "22" * 64, "04" + "cc" * 64


def signer_keys(entry_b64: str) -> list[str]:
    e = xdr.SorobanAuthorizationEntry.from_xdr(entry_b64)
    top = {scval.to_native(m.key): m.val for m in e.credentials.address.signature.map.sc_map}
    return [bytes(scval.to_native(m.key)[2]).hex() for m in top["signers"].map.sc_map]


def test_signatures_merge_into_one_entry_in_the_order_the_chain_requires():
    c = call()
    merged = merge_signed(entry(c), [entry(c, signers={G3: b"s3"}), entry(c, signers={G1: b"s1"})])
    assert signer_keys(merged) == [G1, G3], "map keys sorted (Soroban ScMap order)"


def test_a_signature_over_a_different_entry_is_refused():
    c = call()
    for bad in [entry(c, nonce=8, signers={G1: b"s"}),                  # another nonce
                entry(call(account=OTHER), signers={G1: b"s"}),         # another account's call
                entry(call(fn="cancel_recovery"), signers={G1: b"s"}),  # another function
                entry(c, signers={G1: b"s"}, rule=4)]:                  # another rule
        with pytest.raises(Rejected):
            merge_signed(entry(c), [entry(c, signers={G2: b"x"}), bad])


def test_the_mailbox_collects_approvals_until_the_threshold(tmp_path):
    c = call()
    box = RecoveryMailbox(str(tmp_path / "rec.json"), clock=lambda: 1000.0)
    rid = box.open(ACCOUNT, NEW_KEY, 2, func(c), entry(c))["id"]
    assert box.approve(rid, entry(c, signers={G1: b"s1"}))["ready"] is False
    assert box.approve(rid, entry(c, signers={G1: b"s1"}))["approvals"] == 1, "the same guardian counts once"
    out = box.approve(rid, entry(c, signers={G2: b"s2"}))
    assert out["ready"] is True and out["approvals"] == 2
    keys = signer_keys(out["entry"])
    assert sorted(keys) == sorted([G1, G2]) and keys == sorted(keys)
    assert out["func"] == func(c)


def test_requests_survive_a_restart_and_expire_after_seven_days(tmp_path):
    c = call()
    path = str(tmp_path / "rec.json")
    now = [1000.0]
    box = RecoveryMailbox(path, clock=lambda: now[0])
    rid = box.open(ACCOUNT, NEW_KEY, 1, func(c), entry(c))["id"]
    box.approve(rid, entry(c, signers={G1: b"s1"}))
    again = RecoveryMailbox(path, clock=lambda: now[0])
    assert again.get(rid)["approvals"] == [G1]
    now[0] += 7 * 86_400 + 1
    with pytest.raises(Rejected):
        again.get(rid)


def test_only_a_propose_recovery_on_the_named_account_can_be_opened(tmp_path):
    box = RecoveryMailbox(str(tmp_path / "rec.json"))
    with pytest.raises(Rejected):
        box.open(ACCOUNT, NEW_KEY, 1, func(call(fn="cancel_recovery")), entry(call(fn="cancel_recovery")))
    with pytest.raises(Rejected):
        box.open(ACCOUNT, NEW_KEY, 1, func(call(account=OTHER)), entry(call(account=OTHER)))
    with pytest.raises(Rejected):
        box.open(ACCOUNT, NEW_KEY, 0, func(call()), entry(call()))
