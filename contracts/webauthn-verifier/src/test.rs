#![cfg(test)]
extern crate std;

use super::*;
use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey, VerifyingKey};
use soroban_sdk::{xdr::ToXdr, Bytes, BytesN, Env};

const UP: u8 = 0x01;
const UV: u8 = 0x04;

/// RFC 4648 §5 base64url without padding (independent of the contract code).
fn b64url(bytes: &[u8]) -> std::string::String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = std::string::String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..(chunk.len() + 1) {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn sha(env: &Env, data: &[u8]) -> [u8; 32] {
    env.crypto().sha256(&Bytes::from_slice(env, data)).to_array()
}

struct Passkey {
    sk: SigningKey,
    pk: [u8; 65],
}

fn passkey() -> Passkey {
    let sk = SigningKey::random(&mut rand_core::OsRng);
    let pk: [u8; 65] = VerifyingKey::from(&sk).to_encoded_point(false).as_bytes().try_into().unwrap();
    Passkey { sk, pk }
}

/// W3C WebAuthn L2 assertion over `challenge`, XDR-encoded as the account sends it.
fn assertion(env: &Env, key: &Passkey, challenge: &[u8; 32], flags: u8) -> Bytes {
    let mut auth_data = sha(env, b"constella.app").to_vec();
    auth_data.push(flags);
    auth_data.extend_from_slice(&[0, 0, 0, 9]);
    let client_data = std::format!(
        r#"{{"type":"webauthn.get","challenge":"{}","origin":"https://constella.app","crossOrigin":false}}"#,
        b64url(challenge)
    );
    let mut msg = auth_data.clone();
    msg.extend_from_slice(&sha(env, client_data.as_bytes()));
    let sig: Signature = key.sk.sign_prehash(&sha(env, &msg)).unwrap();
    let sig = sig.normalize_s().unwrap_or(sig);
    WebAuthnSigData {
        signature: BytesN::from_array(env, &sig.to_bytes().into()),
        authenticator_data: Bytes::from_slice(env, &auth_data),
        client_data: Bytes::from_slice(env, client_data.as_bytes()),
    }
    .to_xdr(env)
}

fn setup() -> (Env, WebAuthnVerifierClient<'static>) {
    let env = Env::default();
    let id = env.register(WebAuthnVerifier, ());
    let client = WebAuthnVerifierClient::new(&env, &id);
    (env, client)
}

#[test]
fn verifies_a_real_assertion_over_the_hash() {
    let (env, v) = setup();
    let key = passkey();
    let hash = sha(&env, b"auth digest");
    assert!(v.verify(&Bytes::from_array(&env, &hash), &Bytes::from_array(&env, &key.pk), &assertion(&env, &key, &hash, UP | UV)));
}

#[test]
fn credential_id_suffix_is_metadata_only() {
    let (env, v) = setup();
    let key = passkey();
    let hash = sha(&env, b"auth digest");
    let mut key_data = Bytes::from_array(&env, &key.pk);
    key_data.append(&Bytes::from_array(&env, &[7u8; 16])); // credential ID
    assert!(v.verify(&Bytes::from_array(&env, &hash), &key_data, &assertion(&env, &key, &hash, UP | UV)));
    assert_eq!(v.canonicalize_key(&key_data), Bytes::from_array(&env, &key.pk));
}

#[test]
fn rejects_an_assertion_for_another_hash() {
    let (env, v) = setup();
    let key = passkey();
    let other = sha(&env, b"another digest");
    let hash = sha(&env, b"auth digest");
    let res = v.try_verify(&Bytes::from_array(&env, &hash), &Bytes::from_array(&env, &key.pk), &assertion(&env, &key, &other, UP | UV));
    assert_eq!(res.err().unwrap().unwrap(), soroban_sdk::Error::from_contract_error(3114));
}

#[test]
fn rejects_another_key_and_missing_user_verification() {
    let (env, v) = setup();
    let key = passkey();
    let stranger = passkey();
    let hash = sha(&env, b"auth digest");
    let h = Bytes::from_array(&env, &hash);
    assert!(v.try_verify(&h, &Bytes::from_array(&env, &stranger.pk), &assertion(&env, &key, &hash, UP | UV)).is_err());
    let res = v.try_verify(&h, &Bytes::from_array(&env, &key.pk), &assertion(&env, &key, &hash, UP));
    assert_eq!(res.err().unwrap().unwrap(), soroban_sdk::Error::from_contract_error(3117));
}

#[test]
fn rejects_malformed_key_and_signature_data() {
    let (env, v) = setup();
    let key = passkey();
    let hash = sha(&env, b"auth digest");
    let h = Bytes::from_array(&env, &hash);
    let res = v.try_verify(&h, &Bytes::from_slice(&env, &key.pk[..64]), &assertion(&env, &key, &hash, UP | UV));
    assert_eq!(res.err().unwrap().unwrap(), VerifierError::KeyDataInvalid.into());
    // Not XDR at all: the host's decoder rejects it (fails closed).
    let res = v.try_verify(&h, &Bytes::from_array(&env, &key.pk), &Bytes::from_array(&env, &[1, 2, 3]));
    assert!(res.is_err());
    // Well-formed XDR of another type: rejected by the verifier itself.
    let res = v.try_verify(&h, &Bytes::from_array(&env, &key.pk), &7u32.to_xdr(&env));
    assert_eq!(res.err().unwrap().unwrap(), VerifierError::SigDataInvalid.into());
}
