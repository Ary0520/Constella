"""Gateway validation on real Soroban XDR (built with stellar-sdk, no mocks)."""
from __future__ import annotations

import base64
import os

import pytest
from stellar_sdk import Address, Keypair, StrKey, scval, xdr

from constella_relayer.gateway import MAX_AUTH_ENTRIES, Rejected, validate

VAULT = StrKey.encode_contract(os.urandom(32))
USDC = StrKey.encode_contract(os.urandom(32))
USER = StrKey.encode_contract(os.urandom(32))  # a passkey smart account
RELAYER = Keypair.random().public_key
ALLOWED = {VAULT: frozenset({"deposit", "redeem"})}
RELAYERS = frozenset({RELAYER})


def b64(obj) -> str:
    return base64.b64encode(obj.to_xdr_bytes()).decode()


def call(contract=VAULT, fn="deposit", args=None) -> xdr.InvokeContractArgs:
    if args is None:
        args = [scval.to_int128(500), scval.to_address(USER), scval.to_address(USER), scval.to_address(USER)]
    return xdr.InvokeContractArgs(
        contract_address=Address(contract).to_xdr_sc_address(),
        function_name=xdr.SCSymbol(fn.encode()),
        args=args,
    )


def func(c: xdr.InvokeContractArgs) -> str:
    return b64(xdr.HostFunction(type=xdr.HostFunctionType.HOST_FUNCTION_TYPE_INVOKE_CONTRACT, invoke_contract=c))


def addr_creds(who: str) -> xdr.SorobanAddressCredentials:
    return xdr.SorobanAddressCredentials(
        address=Address(who).to_xdr_sc_address(),
        nonce=xdr.Int64(7),
        signature_expiration_ledger=xdr.Uint32(1_000),
        signature=scval.to_void(),
    )


def delegate(who: str, nested=()) -> xdr.SorobanDelegateSignature:
    return xdr.SorobanDelegateSignature(address=Address(who).to_xdr_sc_address(), signature=scval.to_void(),
                                        nested_delegates=list(nested))


def entry(c: xdr.InvokeContractArgs, who: str = USER, source_account: bool = False, kind: str = "v1",
          delegates=()) -> str:
    T = xdr.SorobanCredentialsType
    if source_account:
        creds = xdr.SorobanCredentials(type=T.SOROBAN_CREDENTIALS_SOURCE_ACCOUNT)
    elif kind == "v2":
        creds = xdr.SorobanCredentials(type=T.SOROBAN_CREDENTIALS_ADDRESS_V2, address_v2=addr_creds(who))
    elif kind == "delegates":
        creds = xdr.SorobanCredentials(
            type=T.SOROBAN_CREDENTIALS_ADDRESS_WITH_DELEGATES,
            address_with_delegates=xdr.SorobanAddressCredentialsWithDelegates(
                address_credentials=addr_creds(who), delegates=list(delegates)),
        )
    else:
        creds = xdr.SorobanCredentials(type=T.SOROBAN_CREDENTIALS_ADDRESS, address=addr_creds(who))
    return b64(
        xdr.SorobanAuthorizationEntry(
            credentials=creds,
            root_invocation=xdr.SorobanAuthorizedInvocation(
                function=xdr.SorobanAuthorizedFunction(
                    type=xdr.SorobanAuthorizedFunctionType.SOROBAN_AUTHORIZED_FUNCTION_TYPE_CONTRACT_FN,
                    contract_fn=c,
                ),
                sub_invocations=[],
            ),
        )
    )


def code(f, a) -> str:
    with pytest.raises(Rejected) as e:
        validate(f, a, ALLOWED, RELAYERS)
    return e.value.code


def test_a_users_signed_deposit_is_accepted():
    c = call()
    v = validate(func(c), [entry(c)], ALLOWED, RELAYERS)
    assert (v.contract, v.function, v.user) == (VAULT, "deposit", USER)
    assert len(v.auth) == 1


def test_source_account_credentials_are_refused_the_relayer_never_authorizes():
    # The classic relayer drain: an entry authorized by the tx source (the
    # relayer) would let the request spend the relayer's own funds.
    c = call()
    assert code(func(c), [entry(c, source_account=True)]) == "SOURCE_ACCOUNT_AUTH_FORBIDDEN"
    assert code(func(c), [entry(c), entry(c, source_account=True)]) == "SOURCE_ACCOUNT_AUTH_FORBIDDEN"


def test_relayer_addresses_may_not_appear_in_auth_or_arguments():
    c = call()
    assert code(func(c), [entry(c, who=RELAYER)]) == "RELAYER_ADDRESS_IN_AUTH"
    pull = call(args=[scval.to_int128(500), scval.to_address(USER), scval.to_address(RELAYER), scval.to_address(USER)])
    assert code(func(pull), [entry(pull)]) == "RELAYER_ADDRESS_IN_ARGS"
    nested = call(args=[scval.to_vec([scval.to_map({scval.to_symbol("to"): scval.to_address(RELAYER)})])])
    assert code(func(nested), [entry(nested)]) == "RELAYER_ADDRESS_IN_ARGS"


def test_only_allow_listed_contracts_and_functions():
    other = call(contract=USDC, fn="transfer")
    assert code(func(other), [entry(other)]) == "TARGET_NOT_ALLOWED"
    admin = call(fn="set_keeper", args=[scval.to_address(USER)])
    assert code(func(admin), [entry(admin)]) == "FUNCTION_NOT_ALLOWED"


def test_only_contract_calls_no_deploys_or_uploads():
    upload = b64(xdr.HostFunction(type=xdr.HostFunctionType.HOST_FUNCTION_TYPE_UPLOAD_CONTRACT_WASM, wasm=b"\x00asm"))
    assert code(upload, [entry(call())]) == "NOT_A_CONTRACT_CALL"


def test_the_auth_must_be_for_exactly_this_call():
    c = call()
    signed_for_less = call(args=[scval.to_int128(1), scval.to_address(USER), scval.to_address(USER), scval.to_address(USER)])
    assert code(func(c), [entry(signed_for_less)]) == "AUTH_MISMATCH"
    assert code(func(c), [entry(call(fn="redeem"))]) == "AUTH_MISMATCH"


def test_auth_is_required_and_bounded():
    c = call()
    assert code(func(c), []) == "AUTH_REQUIRED"
    assert code(func(c), [entry(c)] * (MAX_AUTH_ENTRIES + 1)) == "TOO_MANY_AUTH_ENTRIES"


def test_malformed_and_oversized_input_is_refused():
    c = call()
    assert code("not base64!!", [entry(c)]) == "INVALID_XDR"
    assert code(base64.b64encode(b"\x01\x02").decode(), [entry(c)]) == "INVALID_XDR"
    assert code(func(c), ["AAAA"]) == "INVALID_XDR"
    assert code(base64.b64encode(b"\x00" * (17 * 1024)).decode(), [entry(c)]) == "INVALID_PARAMS"
    assert code(func(c), "not-a-list") == "AUTH_REQUIRED"


def test_cap71_address_bound_credentials_are_accepted():
    c = call()
    assert validate(func(c), [entry(c, kind="v2")], ALLOWED, RELAYERS).user == USER
    other = StrKey.encode_contract(os.urandom(32))
    assert validate(func(c), [entry(c, kind="delegates", delegates=[delegate(other)])], ALLOWED, RELAYERS).user == USER


def test_a_relayer_address_is_refused_as_a_delegate_at_any_depth():
    c = call()
    other = StrKey.encode_contract(os.urandom(32))
    assert code(func(c), [entry(c, kind="delegates", delegates=[delegate(RELAYER)])]) == "RELAYER_ADDRESS_IN_AUTH"
    deep = delegate(other, [delegate(other, [delegate(RELAYER)])])
    assert code(func(c), [entry(c, kind="delegates", delegates=[deep])]) == "RELAYER_ADDRESS_IN_AUTH"


def test_delegate_trees_are_bounded():
    c = call()
    other = StrKey.encode_contract(os.urandom(32))
    many = [delegate(other) for _ in range(9)]
    assert code(func(c), [entry(c, kind="delegates", delegates=many)]) == "TOO_MANY_DELEGATES"


# --- Step 3: users' own factory vaults (new addresses, not on the static list) ---
VAULT_FNS = frozenset({"deposit", "redeem", "withdraw", "redeem_frozen"})
OWN_VAULT = StrKey.encode_contract(os.urandom(32))


def test_vault_user_functions_are_accepted_on_an_unlisted_contract_for_the_on_chain_proof():
    # The gateway lets the vault user functions through on any contract; the
    # relayer then requires the fee-sponsor's on-chain is_whitelisted, which
    # holds only for vaults proven by a trusted Constella factory.
    for fn in sorted(VAULT_FNS):
        c = call(contract=OWN_VAULT, fn=fn)
        v = validate(func(c), [entry(c)], ALLOWED, RELAYERS, vault_functions=VAULT_FNS)
        assert (v.contract, v.function, v.needs_chain_proof) == (OWN_VAULT, fn, True)


def test_statically_listed_calls_need_no_extra_proof():
    c = call()
    v = validate(func(c), [entry(c)], ALLOWED, RELAYERS, vault_functions=VAULT_FNS)
    assert v.needs_chain_proof is False


def test_other_functions_on_an_unlisted_contract_stay_refused():
    for fn, args in [("set_keeper", [scval.to_address(USER)]), ("transfer", None), ("propose_upgrade", [scval.to_bytes(b"\0" * 32)])]:
        c = call(contract=OWN_VAULT, fn=fn, args=args)
        with pytest.raises(Rejected) as e:
            validate(func(c), [entry(c)], ALLOWED, RELAYERS, vault_functions=VAULT_FNS)
        assert e.value.code == "TARGET_NOT_ALLOWED"


def test_unlisted_contracts_are_refused_when_no_vault_functions_are_configured():
    c = call(contract=OWN_VAULT, fn="deposit")
    assert code(func(c), [entry(c)]) == "TARGET_NOT_ALLOWED"


def test_every_other_rule_still_applies_to_own_vault_calls():
    pull = call(contract=OWN_VAULT, fn="deposit",
                args=[scval.to_int128(500), scval.to_address(USER), scval.to_address(RELAYER), scval.to_address(USER)])
    with pytest.raises(Rejected) as e:
        validate(func(pull), [entry(pull)], ALLOWED, RELAYERS, vault_functions=VAULT_FNS)
    assert e.value.code == "RELAYER_ADDRESS_IN_ARGS"
    c = call(contract=OWN_VAULT, fn="deposit")
    with pytest.raises(Rejected) as e:
        validate(func(c), [entry(c, source_account=True)], ALLOWED, RELAYERS, vault_functions=VAULT_FNS)
    assert e.value.code == "SOURCE_ACCOUNT_AUTH_FORBIDDEN"


def test_recovery_calls_on_an_unlisted_account_need_the_on_chain_proof():
    acct_fns = frozenset({"propose_recovery", "cancel_recovery"})
    c = call(contract=OWN_VAULT, fn="propose_recovery", args=[scval.to_bytes(b"\x04" * 65)])
    v = validate(func(c), [entry(c)], ALLOWED, RELAYERS, vault_functions=VAULT_FNS | acct_fns)
    assert v.needs_chain_proof is True and v.function == "propose_recovery"
    other = call(contract=OWN_VAULT, fn="add_signer", args=[scval.to_bytes(b"\x04" * 65)])
    with pytest.raises(Rejected):
        validate(func(other), [entry(other)], ALLOWED, RELAYERS, vault_functions=VAULT_FNS | acct_fns)
