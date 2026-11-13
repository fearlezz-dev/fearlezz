//! Web Crypto, the runtime's own: RS256 to check OIDC tokens and sign the app's JWTs, HMAC for
//! the session cookie, and random ids. No crypto crate is compiled into the worker.

use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Crypto, CryptoKey, SubtleCrypto};
use worker::{Error, Result};

fn crypto() -> Result<Crypto> {
    Ok(Reflect::get(&js_sys::global(), &"crypto".into())?.unchecked_into())
}

fn subtle() -> Result<SubtleCrypto> {
    Ok(crypto()?.subtle())
}

/// A JS object from string fields, for algorithm parameters and JWKs.
fn object(fields: &[(&str, JsValue)]) -> Result<Object> {
    let object = Object::new();
    for (name, value) in fields {
        Reflect::set(&object, &(*name).into(), value)?;
    }
    Ok(object)
}

fn rs256() -> Result<Object> {
    object(&[
        ("name", "RSASSA-PKCS1-v1_5".into()),
        ("hash", "SHA-256".into()),
    ])
}

fn hmac() -> Result<Object> {
    object(&[("name", "HMAC".into()), ("hash", "SHA-256".into())])
}

fn usages(usage: &str) -> JsValue {
    Array::of1(&usage.into()).into()
}

async fn import(format: &str, data: &Object, algorithm: &Object, usage: &str) -> Result<CryptoKey> {
    let promise =
        subtle()?.import_key_with_object(format, data, algorithm, false, &usages(usage))?;
    Ok(JsFuture::from(promise).await?.unchecked_into())
}

/// An RSA public key from the `n` and `e` of a JWK, both base64url as JWKs carry them.
pub async fn rsa_public_key(n: &str, e: &str) -> Result<CryptoKey> {
    let jwk = object(&[
        ("kty", "RSA".into()),
        ("alg", "RS256".into()),
        ("ext", true.into()),
        ("n", n.into()),
        ("e", e.into()),
    ])?;
    import("jwk", &jwk, &rs256()?, "verify").await
}

pub async fn rs256_verify(key: &CryptoKey, signature: &[u8], data: &[u8]) -> Result<bool> {
    let promise =
        subtle()?.verify_with_object_and_u8_array_and_u8_array(&rs256()?, key, signature, data)?;
    Ok(JsFuture::from(promise).await?.as_bool().unwrap_or(false))
}

/// An RSA private key from PEM: PKCS#8, or the PKCS#1 GitHub hands out for app keys, which Web
/// Crypto cannot import until it is wrapped as PKCS#8.
pub async fn rsa_private_key(pem: &str) -> Result<CryptoKey> {
    let pkcs1 = pem.contains("BEGIN RSA PRIVATE KEY");
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    let der = base64_decode(body.trim())?;
    let der = if pkcs1 { pkcs1_to_pkcs8(&der) } else { der };
    import(
        "pkcs8",
        &Uint8Array::from(der.as_slice()),
        &rs256()?,
        "sign",
    )
    .await
}

pub async fn rs256_sign(key: &CryptoKey, data: &[u8]) -> Result<Vec<u8>> {
    let promise = subtle()?.sign_with_object_and_u8_array(&rs256()?, key, data)?;
    Ok(Uint8Array::new(&JsFuture::from(promise).await?).to_vec())
}

async fn hmac_key(secret: &str) -> Result<CryptoKey> {
    let raw = Uint8Array::from(secret.as_bytes());
    import("raw", &raw, &hmac()?, "sign").await
}

pub async fn hmac_sign(secret: &str, data: &[u8]) -> Result<Vec<u8>> {
    let key = hmac_key(secret).await?;
    let promise = subtle()?.sign_with_object_and_u8_array(&hmac()?, &key, data)?;
    Ok(Uint8Array::new(&JsFuture::from(promise).await?).to_vec())
}

/// Whether `mac` is the HMAC of `data`, compared in constant time.
pub async fn hmac_verify(secret: &str, data: &[u8], mac: &[u8]) -> Result<bool> {
    let expected = hmac_sign(secret, data).await?;
    if expected.len() != mac.len() {
        return Ok(false);
    }
    Ok(expected
        .iter()
        .zip(mac)
        .fold(0, |diff, (a, b)| diff | (a ^ b))
        == 0)
}

/// 128 random bits as hex: proof ids and sign in states.
pub fn random_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    crypto()?.get_random_values_with_u8_array(&mut bytes)?;
    Ok(hex::encode(bytes))
}

pub fn base64_decode(text: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|e| Error::RustError(format!("bad base64: {e}")))
}

/// Wraps a PKCS#1 RSAPrivateKey in a PKCS#8 PrivateKeyInfo: version 0, the rsaEncryption
/// algorithm, and the key as an octet string.
fn pkcs1_to_pkcs8(pkcs1: &[u8]) -> Vec<u8> {
    const VERSION_AND_ALGORITHM: [u8; 18] = [
        0x02, 0x01, 0x00, // INTEGER 0
        0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05,
        0x00, // SEQUENCE { OID 1.2.840.113549.1.1.1, NULL }
    ];
    let mut octets = vec![0x04];
    octets.extend(der_length(pkcs1.len()));
    octets.extend_from_slice(pkcs1);
    let mut info = vec![0x30];
    info.extend(der_length(VERSION_AND_ALGORITHM.len() + octets.len()));
    info.extend_from_slice(&VERSION_AND_ALGORITHM);
    info.extend(octets);
    info
}

fn der_length(length: usize) -> Vec<u8> {
    if length < 0x80 {
        return vec![length as u8];
    }
    let bytes: Vec<u8> = length
        .to_be_bytes()
        .into_iter()
        .skip_while(|b| *b == 0)
        .collect();
    let mut out = vec![0x80 | bytes.len() as u8];
    out.extend(bytes);
    out
}
