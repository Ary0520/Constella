//! Stateless passkey (WebAuthn / secp256r1) verifier for OpenZeppelin smart
//! accounts (audit W4.3b). One deployment serves every Constella account: an
//! account registers each passkey as `Signer::External(verifier, key_data)`.
//!
//! * `key_data`: 65-byte uncompressed SEC-1 P-256 public key, optionally
//!   followed by the WebAuthn credential ID (metadata, ignored for identity).
//! * `sig_data`: XDR encoding of OpenZeppelin `WebAuthnSigData`
//!   (`signature`, `authenticator_data`, `client_data`) - exactly what
//!   `navigator.credentials.get()` returns, with a low-S signature.
//! * `hash`: the 32-byte digest the account asks to be signed (the
//!   `AuthDigestPreimage` digest), carried as the WebAuthn challenge.
//!
//! All checks are OpenZeppelin's `webauthn::verify`: type `webauthn.get`,
//! challenge == base64url(hash), User Present and User Verified flags,
//! BE/BS consistency, P-256 over SHA-256(authenticatorData || SHA-256(clientDataJSON)).
//! Any failure aborts the call, which fails the authorization.
#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, panic_with_error, xdr::FromXdr, Bytes, BytesN, Env, Vec};
use stellar_accounts::verifiers::{
    webauthn::{self, WebAuthnSigData},
    Verifier,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum VerifierError {
    /// `key_data` is shorter than a 65-byte public key.
    KeyDataInvalid = 1,
    /// `sig_data` is not an XDR-encoded `WebAuthnSigData`.
    SigDataInvalid = 2,
}

#[contract]
pub struct WebAuthnVerifier;

fn public_key(e: &Env, key_data: &Bytes) -> BytesN<65> {
    if key_data.len() < 65 {
        panic_with_error!(e, VerifierError::KeyDataInvalid);
    }
    match BytesN::<65>::try_from(key_data.slice(0..65)) {
        Ok(pk) => pk,
        Err(_) => panic_with_error!(e, VerifierError::KeyDataInvalid),
    }
}

#[contractimpl]
impl Verifier for WebAuthnVerifier {
    type KeyData = Bytes;
    type SigData = Bytes;

    fn verify(e: &Env, hash: Bytes, key_data: Bytes, sig_data: Bytes) -> bool {
        let pk = public_key(e, &key_data);
        let sig = match WebAuthnSigData::from_xdr(e, &sig_data) {
            Ok(s) => s,
            Err(_) => panic_with_error!(e, VerifierError::SigDataInvalid),
        };
        webauthn::verify(e, &hash, &pk, &sig)
    }

    fn canonicalize_key(e: &Env, key_data: Bytes) -> Bytes {
        webauthn::canonicalize_key(e, &key_data)
    }

    fn batch_canonicalize_key(e: &Env, key_data: Vec<Bytes>) -> Vec<Bytes> {
        webauthn::batch_canonicalize_key(e, &key_data)
    }
}

#[cfg(test)]
mod test;
