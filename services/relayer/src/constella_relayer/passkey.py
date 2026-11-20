"""Passkey signing for Constella smart accounts (OpenZeppelin `stellar-accounts`
0.8.0 context rules + the shared `webauthn-verifier`).

What the account verifies (contracts/smart-account, contracts/webauthn-verifier):
  1. The host gives `__check_auth` the payload SHA-256(HashIdPreimage) of the
     auth entry. The preimage depends on the credential type (legacy ADDRESS
     vs CAP-71 ADDRESS_V2, which binds the address); we take it from
     stellar-sdk's `build_authorization_preimage` so it is never re-derived.
  2. Each passkey signs a WebAuthn assertion whose challenge is
     base64url(SHA-256(XDR(AuthDigestPreimage{account, signature_payload,
     context_rule_ids}))) - type "webauthn.get", User Present + User Verified,
     P-256 over SHA-256(authenticatorData || SHA-256(clientDataJSON)), low-S.
  3. The credential signature is AuthPayload{context_rule_ids, signers:
     {Signer::External(verifier, public_key): XDR(WebAuthnSigData)}}.

`SoftwarePasskey` stands in for a platform authenticator (Face ID / Touch ID)
in tests and the local-network e2e; a browser client produces the same
`(authenticator_data, client_data, signature)` from navigator.credentials.get().
"""
from __future__ import annotations

import base64
import hashlib
import json

from stellar_sdk import Address, scval, xdr
from stellar_sdk.auth import authorization_payload_hash, build_authorization_preimage

from .gateway import address_credentials

P256_N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
UP = 0x01  # WebAuthn flag: user present
UV = 0x04  # WebAuthn flag: user verified (biometric / PIN)


def _sha(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def struct(fields: dict[str, xdr.SCVal]) -> xdr.SCVal:
    """A Soroban #[contracttype] struct: a map keyed by field-name symbols in
    sorted order (the host requires sorted map keys)."""
    entries = [xdr.SCMapEntry(key=scval.to_symbol(k), val=fields[k]) for k in sorted(fields)]
    return xdr.SCVal(type=xdr.SCValType.SCV_MAP, map=xdr.SCMap(entries))


def u32_vec(values: list[int]) -> xdr.SCVal:
    return scval.to_vec([scval.to_uint32(v) for v in values])


def auth_digest(account: str, signature_payload: bytes, context_rule_ids: list[int]) -> bytes:
    """OpenZeppelin `AuthDigestPreimage::digest`: SHA-256 of its XDR."""
    preimage = struct({
        "account": scval.to_address(account),
        "context_rule_ids": u32_vec(context_rule_ids),
        "signature_payload": scval.to_bytes(signature_payload),
    })
    return _sha(preimage.to_xdr_bytes())


class SoftwarePasskey:
    """A P-256 WebAuthn authenticator in software (tests / e2e only)."""

    def __init__(self, private_key=None, rp_id: str = "constella.app", origin: str = "https://constella.app"):
        from cryptography.hazmat.primitives.asymmetric import ec

        self._key = private_key or ec.generate_private_key(ec.SECP256R1())
        self.rp_id = rp_id
        self.origin = origin
        self.sign_count = 0

    @property
    def public_key(self) -> bytes:
        """65-byte uncompressed SEC-1 point (0x04 || X || Y)."""
        from cryptography.hazmat.primitives import serialization

        return self._key.public_key().public_bytes(
            serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint
        )

    def private_pem(self) -> bytes:
        from cryptography.hazmat.primitives import serialization

        return self._key.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
        )

    @classmethod
    def from_pem(cls, pem: bytes) -> "SoftwarePasskey":
        from cryptography.hazmat.primitives import serialization

        return cls(serialization.load_pem_private_key(pem, password=None))

    def get_assertion(self, challenge: bytes, flags: int = UP | UV) -> tuple[bytes, bytes, bytes]:
        """(authenticator_data, client_data_json, 64-byte low-S r||s)."""
        from cryptography.hazmat.primitives import hashes
        from cryptography.hazmat.primitives.asymmetric import ec
        from cryptography.hazmat.primitives.asymmetric.utils import Prehashed, decode_dss_signature

        self.sign_count += 1
        authenticator_data = _sha(self.rp_id.encode()) + bytes([flags]) + self.sign_count.to_bytes(4, "big")
        client_data = json.dumps(
            {"type": "webauthn.get", "challenge": b64url(challenge), "origin": self.origin, "crossOrigin": False},
            separators=(",", ":"),
        ).encode()
        message_hash = _sha(authenticator_data + _sha(client_data))
        r, s = decode_dss_signature(self._key.sign(message_hash, ec.ECDSA(Prehashed(hashes.SHA256()))))
        if s > P256_N // 2:  # the verifier accepts low-S only
            s = P256_N - s
        return authenticator_data, client_data, r.to_bytes(32, "big") + s.to_bytes(32, "big")


def count_invocations(inv: xdr.SorobanAuthorizedInvocation) -> int:
    """Auth contexts the account is asked about: the root and every nested
    invocation it authorizes (e.g. deposit + the USDC transfer it triggers)."""
    return 1 + sum(count_invocations(s) for s in inv.sub_invocations)


def sign_auth_entry(
    entry: xdr.SorobanAuthorizationEntry,
    passkey: SoftwarePasskey,
    verifier: str,
    rule_id: int,
    valid_until_ledger: int,
    network_passphrase: str,
) -> xdr.SorobanAuthorizationEntry:
    """Sign an address auth entry for a Constella smart account with one
    passkey under rule `rule_id` (one id per authorized context). Returns a
    signed copy; the input is not modified."""
    signed = xdr.SorobanAuthorizationEntry.from_xdr_bytes(entry.to_xdr_bytes())
    creds = address_credentials(signed.credentials)
    if creds is None:
        raise ValueError("only address credentials can be signed by a passkey")
    if signed.credentials.type == xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS_WITH_DELEGATES:
        raise ValueError("delegated credentials are not used by Constella accounts")
    creds.signature_expiration_ledger = xdr.Uint32(valid_until_ledger)
    account = Address.from_xdr_sc_address(creds.address).address
    payload = authorization_payload_hash(build_authorization_preimage(signed, valid_until_ledger, network_passphrase))
    rule_ids = [rule_id] * count_invocations(signed.root_invocation)
    authenticator_data, client_data, signature = passkey.get_assertion(auth_digest(account, payload, rule_ids))
    sig_data = struct({
        "authenticator_data": scval.to_bytes(authenticator_data),
        "client_data": scval.to_bytes(client_data),
        "signature": scval.to_bytes(signature),
    })
    signer = scval.to_vec([scval.to_symbol("External"), scval.to_address(verifier), scval.to_bytes(passkey.public_key)])
    signers = xdr.SCVal(
        type=xdr.SCValType.SCV_MAP,
        map=xdr.SCMap([xdr.SCMapEntry(key=signer, val=scval.to_bytes(sig_data.to_xdr_bytes()))]),
    )
    creds.signature = struct({"context_rule_ids": u32_vec(rule_ids), "signers": signers})
    return signed
