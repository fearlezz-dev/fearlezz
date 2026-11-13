//! The GitHub Actions OIDC token every API call carries: signed by one of GitHub's published
//! keys, from GitHub's issuer, for this host, not expired. Its claims say which repository and
//! ref the run is for, and a token only ever acts for its own repository.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::Value;
use worker::{Fetch, Request, Url};

use crate::{Config, Refusal, crypto, now};

/// What a token says about the run that holds it.
pub struct Run {
    /// `owner/name` as GitHub spells it.
    pub repository: String,
    /// The ref the run is for: `refs/pull/<n>/merge` on a pull request.
    pub git_ref: String,
}

impl Run {
    /// The repository as the host keys it.
    pub fn repo(&self) -> String {
        self.repository.to_lowercase()
    }

    /// Whether the run is for this pull request.
    pub fn is_for_pull(&self, number: u64) -> bool {
        self.git_ref == format!("refs/pull/{number}/merge")
    }
}

fn refused(why: &str) -> Refusal {
    Refusal::new(401, format!("the OIDC token is refused: {why}"))
}

/// The run a request's token speaks for, or a refusal with `401`.
pub async fn check(req: &Request, config: &Config) -> Result<Run, Refusal> {
    let header = req.headers().get("Authorization")?.unwrap_or_default();
    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| refused("send it as Authorization: Bearer <token>"))?;
    let mut parts = token.split('.');
    let (Some(head), Some(body), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(refused("not a JWT"));
    };
    let decode = |part: &str| {
        URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|_| refused("not a JWT"))
    };
    let header: Header =
        serde_json::from_slice(&decode(head)?).map_err(|_| refused("not a JWT"))?;
    if header.alg != "RS256" {
        return Err(refused("not signed with RS256"));
    }
    let key = key(config, &header.kid).await?;
    let signed = format!("{head}.{body}");
    if !crypto::rs256_verify(&key, &decode(signature)?, signed.as_bytes()).await? {
        return Err(refused("bad signature"));
    }
    let claims: Claims =
        serde_json::from_slice(&decode(body)?).map_err(|_| refused("missing claims"))?;
    if claims.iss != config.oidc_issuer {
        return Err(refused("wrong issuer"));
    }
    let audience = match &claims.aud {
        Value::String(aud) => aud == &config.audience,
        Value::Array(auds) => auds.iter().any(|aud| aud == config.audience.as_str()),
        _ => false,
    };
    if !audience {
        return Err(refused("wrong audience"));
    }
    // A minute of leeway for clocks that drift.
    let now = now();
    if claims.exp <= now - 60 || claims.nbf.is_some_and(|nbf| nbf > now + 60) {
        return Err(refused("expired or not yet valid"));
    }
    Ok(Run {
        repository: claims.repository,
        git_ref: claims.git_ref,
    })
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    #[serde(default)]
    kid: String,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: Value,
    exp: i64,
    nbf: Option<i64>,
    repository: String,
    #[serde(rename = "ref")]
    git_ref: String,
}

/// GitHub's published key with this id.
async fn key(config: &Config, kid: &str) -> Result<web_sys::CryptoKey, Refusal> {
    #[derive(Deserialize)]
    struct Jwks {
        keys: Vec<Jwk>,
    }
    #[derive(Deserialize)]
    struct Jwk {
        #[serde(default)]
        kid: String,
        kty: String,
        n: Option<String>,
        e: Option<String>,
    }
    let url = Url::parse(&config.oidc_jwks).map_err(worker::Error::from)?;
    let mut response = Fetch::Url(url).send().await?;
    let jwks: Jwks = response.json().await?;
    let jwk = jwks
        .keys
        .into_iter()
        .find(|jwk| jwk.kid == kid && jwk.kty == "RSA")
        .ok_or_else(|| refused("signed by a key GitHub does not publish"))?;
    let (Some(n), Some(e)) = (jwk.n, jwk.e) else {
        return Err(refused("signed by a key GitHub does not publish"));
    };
    Ok(crypto::rsa_public_key(&n, &e).await?)
}
