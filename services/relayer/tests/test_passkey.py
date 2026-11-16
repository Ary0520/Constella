"""Passkey signer: cross-implementation vectors, real P-256 verification, and
the exact auth-payload structure the smart account verifies."""
from __future__ import annotations

import base64
import hashlib
import json
import os

import pytest
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import Prehashed, encode_dss_signature
from stellar_sdk import Address, StrKey, scval, xdr
from stellar_sdk.auth import authorization_payload_hash, build_authorization_preimage

from constella_relayer.passkey import P256_N, UP, UV, SoftwarePasskey, auth_digest, b64url, sign_auth_entry

PASS = "Standalone Network ; February 2017"
ACCOUNT = StrKey.encode_contract(os.urandom(32))
VERIFIER = StrKey.encode_contract(os.urandom(32))
VAULT = StrKey.encode_contract(os.urandom(32))
USDC = StrKey.encode_contract(os.urandom(32))


def test_digest_vectors_match_openzeppelin_rust():
    # Same constants as smart-account `f1_auth_digest_vectors_match_the_python_client`.
    acct = StrKey.encode_contract(bytes([0x11] * 32))
    assert acct == "CAIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRDB3V"
    assert auth_digest(acct, bytes(range(32)), [0, 0]).hex() == "9b19bf4175973da39fd001b8251d481ef104e9672369c0ee611e68450611210a"
    assert auth_digest(acct, bytes(range(32)), [3]).hex() == "d506511b29a2196dba8d9145cfa66841c45e54ba8a9298e5683a608cb008859b"


def test_an_assertion_is_a_valid_low_s_webauthn_signature():
    pk = SoftwarePasskey()
    challenge = os.urandom(32)
    ad, cd, sig = pk.get_assertion(challenge)
    client = json.loads(cd)
    assert client["type"] == "webauthn.get" and client["challenge"] == b64url(challenge)
    assert ad[32] & UP and ad[32] & UV and ad[:32] == hashlib.sha256(b"constella.app").digest()
    r, s = int.from_bytes(sig[:32], "big"), int.from_bytes(sig[32:], "big")
    assert s <= P256_N // 2, "low-S"
    msg = hashlib.sha256(ad + hashlib.sha256(cd).digest()).digest()
    pub = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), pk.public_key)
    pub.verify(encode_dss_signature(r, s), msg, ec.ECDSA(Prehashed(hashes.SHA256())))
    with pytest.raises(InvalidSignature):
        pub.verify(encode_dss_signature(r, s), hashlib.sha256(b"other").digest(), ec.ECDSA(Prehashed(hashes.SHA256())))
    assert len(pk.public_key) == 65 and pk.public_key[0] == 4
    assert SoftwarePasskey.from_pem(pk.private_pem()).public_key == pk.public_key


def _call(contract, fn, args):
    return xdr.InvokeContractArgs(contract_address=Address(contract).to_xdr_sc_address(),
                                  function_name=xdr.SCSymbol(fn.encode()), args=args)


def _inv(c, subs=()):
    return xdr.SorobanAuthorizedInvocation(
        function=xdr.SorobanAuthorizedFunction(
            type=xdr.SorobanAuthorizedFunctionType.SOROBAN_AUTHORIZED_FUNCTION_TYPE_CONTRACT_FN, contract_fn=c),
        sub_invocations=list(subs))


def _deposit_entry(v2: bool) -> xdr.SorobanAuthorizationEntry:
    a = scval.to_address(ACCOUNT)
    root = _inv(_call(VAULT, "deposit", [scval.to_int128(500), a, a, a]),
                [_inv(_call(USDC, "transfer", [a, scval.to_address(VAULT), scval.to_int128(500)]))])
    creds = xdr.SorobanAddressCredentials(address=Address(ACCOUNT).to_xdr_sc_address(), nonce=xdr.Int64(42),
                                          signature_expiration_ledger=xdr.Uint32(0), signature=scval.to_void())
    T = xdr.SorobanCredentialsType
    c = (xdr.SorobanCredentials(type=T.SOROBAN_CREDENTIALS_ADDRESS_V2, address_v2=creds) if v2
         else xdr.SorobanCredentials(type=T.SOROBAN_CREDENTIALS_ADDRESS, address=creds))
    return xdr.SorobanAuthorizationEntry(credentials=c, root_invocation=root)


def _decode_signature(entry, v2):
    creds = entry.credentials.address_v2 if v2 else entry.credentials.address
    fields = {e.key.sym.sc_symbol.decode(): e.val for e in creds.signature.map.sc_map}
    rule_ids = [v.u32.uint32 for v in fields["context_rule_ids"].vec.sc_vec]
    (signer_entry,) = fields["signers"].map.sc_map
    signer = signer_entry.key.vec.sc_vec
    sig_data = xdr.SCVal.from_xdr_bytes(signer_entry.val.bytes.sc_bytes)
    sd = {e.key.sym.sc_symbol.decode(): e.val.bytes.sc_bytes for e in sig_data.map.sc_map}
    return creds, rule_ids, signer, sd


@pytest.mark.parametrize("v2", [False, True])
def test_a_signed_entry_carries_exactly_what_the_account_verifies(v2):
    pk = SoftwarePasskey()
    entry = _deposit_entry(v2)
    before = entry.to_xdr_bytes()
    signed = sign_auth_entry(entry, pk, VERIFIER, rule_id=0, valid_until_ledger=1234, network_passphrase=PASS)
    assert entry.to_xdr_bytes() == before, "input not modified"
    creds, rule_ids, signer, sd = _decode_signature(signed, v2)
    assert creds.signature_expiration_ledger.uint32 == 1234
    assert rule_ids == [0, 0], "deposit + its USDC transfer: two contexts"
    assert signer[0].sym.sc_symbol == b"External"
    assert Address.from_xdr_sc_address(signer[1].address).address == VERIFIER
    assert signer[2].bytes.sc_bytes == pk.public_key
    # The challenge is the OZ digest over the host payload for THIS credential type.
    payload = authorization_payload_hash(build_authorization_preimage(signed, 1234, PASS))
    challenge = json.loads(sd["client_data"])["challenge"]
    assert challenge == b64url(auth_digest(ACCOUNT, payload, [0, 0]))


def test_v2_payload_is_bound_to_the_address_and_differs_from_legacy():
    legacy = authorization_payload_hash(build_authorization_preimage(_deposit_entry(False), 1234, PASS))
    v2 = authorization_payload_hash(build_authorization_preimage(_deposit_entry(True), 1234, PASS))
    assert legacy != v2


def test_only_plain_address_credentials_are_signed():
    entry = _deposit_entry(False)
    entry.credentials = xdr.SorobanCredentials(type=xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_SOURCE_ACCOUNT)
    with pytest.raises(ValueError):
        sign_auth_entry(entry, SoftwarePasskey(), VERIFIER, 0, 1234, PASS)
