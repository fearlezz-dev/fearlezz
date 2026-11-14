//! A fake GitHub on a local port: the parts the proof host asks (the Actions OIDC keys, the app's
//! installation on a repository and its installation tokens, a repository as a token sees it,
//! the OAuth code exchange and the signed in user, a person's permission, a pull request's head
//! and the comments people post on it), kept in memory. It checks the app's JWT with
//! the app's public key, so a test proves the host signs it as GitHub expects.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::RsaPublicKey;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier as _;
use rsa::traits::PublicKeyParts as _;
use serde_json::{Value, json};
use sha2::Sha256;

/// The GitHub App's client id, which also names it in its JWTs.
pub const CLIENT_ID: &str = "Iv1.fearlezz";
pub const CLIENT_SECRET: &str = "client-secret";
/// The key id of the Actions OIDC signing key.
pub const KID: &str = "actions-key";

pub struct Repo {
    pub private: bool,
    /// The app's installation on it, if any.
    pub installation: Option<u64>,
    /// Logins with at least read access.
    pub readers: BTreeSet<String>,
    /// Logins with write access.
    pub writers: BTreeSet<String>,
    /// Logins with admin rights.
    pub admins: BTreeSet<String>,
}

/// A pull request's head: its commit and the repository it comes from.
#[derive(Clone)]
pub struct Head {
    pub sha: String,
    pub from: String,
}

/// A comment someone posted on a pull request.
#[derive(Clone, Debug)]
pub struct Posted {
    pub repo: String,
    pub pull: u64,
    /// Whose token posted it.
    pub login: String,
    pub body: String,
}

pub struct State {
    pub repos: BTreeMap<String, Repo>,
    /// OAuth codes and the login each signs in.
    pub codes: BTreeMap<String, String>,
    /// Person tokens and their login.
    pub tokens: BTreeMap<String, String>,
    /// The Actions OIDC keys GitHub publishes, by key id; a test can rotate one in.
    pub oidc_keys: Vec<(String, RsaPublicKey)>,
    /// Every request asked, as method and path, so a test can count what the host asks.
    pub asked: Vec<(String, String)>,
    /// Pull request heads by repository and number; any other is `HEAD` from its own repository.
    pub heads: BTreeMap<(String, u64), Head>,
    pub posted: Vec<Posted>,
}

/// The head of a pull request nobody set one for: the head `proof.json` names in the tests.
pub const HEAD: &str = "abc1234abc1234abc1234abc1234abc1234abc1";

pub struct FakeGitHub {
    pub url: String,
    state: Arc<Mutex<State>>,
}

struct Keys {
    app: RsaPublicKey,
}

impl FakeGitHub {
    pub fn start(oidc: RsaPublicKey, app: RsaPublicKey) -> FakeGitHub {
        let set = |logins: &[&str]| logins.iter().map(|l| (*l).to_owned()).collect();
        let repo = |private: bool, installation: Option<u64>, readers: &[&str]| Repo {
            private,
            installation,
            readers: set(readers),
            writers: BTreeSet::new(),
            admins: BTreeSet::new(),
        };
        // Where people approve: a writer, an admin, and a reader who may only look.
        let team = |private: bool, installation: Option<u64>| Repo {
            private,
            installation,
            readers: set(&["reader", "writer", "admin"]),
            writers: set(&["writer", "admin"]),
            admins: set(&["admin"]),
        };
        let state = State {
            repos: BTreeMap::from([
                ("owner/repo".to_owned(), repo(false, Some(1), &["reader"])),
                ("owner/secret".to_owned(), repo(true, Some(2), &["reader"])),
                ("owner/bare".to_owned(), repo(false, None, &[])),
                ("other/repo".to_owned(), repo(false, Some(3), &[])),
                // Only the caching tests view these, so they can count what the host asks.
                ("owner/cached".to_owned(), repo(true, Some(4), &["reader"])),
                ("owner/open".to_owned(), repo(false, Some(5), &[])),
                ("owner/team".to_owned(), team(false, Some(6))),
                ("owner/vault".to_owned(), team(true, Some(7))),
            ]),
            codes: BTreeMap::from([
                ("reader-code".to_owned(), "reader".to_owned()),
                ("stranger-code".to_owned(), "stranger".to_owned()),
                ("writer-code".to_owned(), "writer".to_owned()),
                ("admin-code".to_owned(), "admin".to_owned()),
            ]),
            tokens: BTreeMap::new(),
            oidc_keys: vec![(KID.to_owned(), oidc)],
            asked: Vec::new(),
            heads: BTreeMap::new(),
            posted: Vec::new(),
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(state));
        let keys = Arc::new(Keys { app });
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = Arc::clone(&shared);
                let keys = Arc::clone(&keys);
                std::thread::spawn(move || serve(stream, &state, &keys));
            }
        });
        FakeGitHub { url, state }
    }

    /// How many times the host asked for this method and path.
    pub fn asked(&self, method: &str, path: &str) -> usize {
        let state = self.state.lock().unwrap();
        state
            .asked
            .iter()
            .filter(|(m, p)| m == method && p == path)
            .count()
    }

    /// Sets the head of a pull request, as a push or a fork would.
    pub fn set_head(&self, repo: &str, pull: u64, sha: &str, from: &str) {
        let mut state = self.state.lock().unwrap();
        let head = Head {
            sha: sha.to_owned(),
            from: from.to_owned(),
        };
        state.heads.insert((repo.to_owned(), pull), head);
    }

    /// The comments posted on a pull request, oldest first.
    pub fn posted(&self, repo: &str, pull: u64) -> Vec<Posted> {
        let state = self.state.lock().unwrap();
        state
            .posted
            .iter()
            .filter(|p| p.repo == repo && p.pull == pull)
            .cloned()
            .collect()
    }

    /// Publishes another Actions OIDC key, as GitHub does when it rotates them.
    pub fn publish(&self, kid: &str, key: RsaPublicKey) {
        let mut state = self.state.lock().unwrap();
        state.oidc_keys.push((kid.to_owned(), key));
    }
}

fn serve(stream: TcpStream, state: &Mutex<State>, keys: &Keys) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut length = 0;
    let mut auth = String::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap(),
            "authorization" => value.trim().clone_into(&mut auth),
            _ => {}
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let token = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("token "))
        .unwrap_or_default();
    let (status, answer) = answer(
        &mut state.lock().unwrap(),
        keys,
        &method,
        &path,
        token,
        &body,
    );
    let answer = answer.to_string();
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
        answer.len()
    );
}

fn answer(
    state: &mut State,
    keys: &Keys,
    method: &str,
    path: &str,
    token: &str,
    body: &Value,
) -> (u16, Value) {
    let path = path.split('?').next().unwrap_or_default();
    state.asked.push((method.to_owned(), path.to_owned()));
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, segments.as_slice()) {
        ("GET", [".well-known", "jwks"]) => {
            let keys: Vec<Value> = state
                .oidc_keys
                .iter()
                .map(|(kid, key)| jwk(kid, key))
                .collect();
            (200, json!({ "keys": keys }))
        }
        ("GET", ["repos", owner, name, "installation"]) => {
            if !app_jwt_valid(token, &keys.app) {
                return (
                    401,
                    json!({"message": "A JSON web token could not be decoded"}),
                );
            }
            match state
                .repos
                .get(&format!("{owner}/{name}"))
                .and_then(|r| r.installation)
            {
                Some(id) => (200, json!({"id": id})),
                None => (404, json!({"message": "Not Found"})),
            }
        }
        ("POST", ["app", "installations", id, "access_tokens"]) => {
            if !app_jwt_valid(token, &keys.app) {
                return (
                    401,
                    json!({"message": "A JSON web token could not be decoded"}),
                );
            }
            (201, json!({"token": format!("installation-{id}")}))
        }
        ("GET", ["repos", owner, name]) => {
            let full = format!("{owner}/{name}");
            let Some(repo) = state.repos.get(&full) else {
                return (404, json!({"message": "Not Found"}));
            };
            let installation = token
                .strip_prefix("installation-")
                .and_then(|id| id.parse::<u64>().ok());
            let person = state.tokens.get(token);
            let reads = match (installation, person) {
                (Some(id), _) => repo.installation == Some(id),
                (None, Some(login)) => repo.readers.contains(login),
                (None, None) => false,
            };
            if repo.private && !reads {
                return (404, json!({"message": "Not Found"}));
            }
            (
                200,
                json!({
                    "full_name": full,
                    "private": repo.private,
                    "permissions": {"admin": false, "push": false, "pull": reads},
                }),
            )
        }
        ("GET", ["repos", owner, name, "collaborators", login, "permission"]) => {
            let full = format!("{owner}/{name}");
            let Some(caller) = state.tokens.get(token).cloned() else {
                return (401, json!({"message": "Bad credentials"}));
            };
            let Some(repo) = state.repos.get(&full) else {
                return (404, json!({"message": "Not Found"}));
            };
            if repo.private && !repo.readers.contains(&caller) {
                return (404, json!({"message": "Not Found"}));
            }
            let login = (*login).to_owned();
            let permission = if repo.admins.contains(&login) {
                "admin"
            } else if repo.writers.contains(&login) {
                "write"
            } else if repo.readers.contains(&login) || !repo.private {
                "read"
            } else {
                "none"
            };
            (
                200,
                json!({"permission": permission, "user": {"login": login}}),
            )
        }
        ("GET", ["repos", owner, name, "pulls", number]) => {
            let full = format!("{owner}/{name}");
            // A person's token, or the token of the app's installation on this repository.
            let installed = token
                .strip_prefix("installation-")
                .and_then(|id| id.parse::<u64>().ok())
                .is_some_and(|id| {
                    state
                        .repos
                        .get(&full)
                        .is_some_and(|r| r.installation == Some(id))
                });
            if !installed && !state.tokens.contains_key(token) {
                return (401, json!({"message": "Bad credentials"}));
            }
            let Ok(number) = number.parse::<u64>() else {
                return (404, json!({"message": "Not Found"}));
            };
            let head = state
                .heads
                .get(&(full.clone(), number))
                .cloned()
                .unwrap_or(Head {
                    sha: HEAD.to_owned(),
                    from: full.clone(),
                });
            (
                200,
                json!({
                    "number": number,
                    "state": "open",
                    "head": {"sha": head.sha, "repo": {"full_name": head.from}},
                    "base": {"repo": {"full_name": full}},
                }),
            )
        }
        ("POST", ["repos", owner, name, "issues", number, "comments"]) => {
            let full = format!("{owner}/{name}");
            let Some(login) = state.tokens.get(token).cloned() else {
                return (401, json!({"message": "Bad credentials"}));
            };
            let readable = state
                .repos
                .get(&full)
                .is_some_and(|r| !r.private || r.readers.contains(&login));
            let (Ok(number), true) = (number.parse::<u64>(), readable) else {
                return (404, json!({"message": "Not Found"}));
            };
            let Some(body) = body.get("body").and_then(Value::as_str) else {
                return (422, json!({"message": "body is missing"}));
            };
            let id = state.posted.len() + 1;
            state.posted.push(Posted {
                repo: full.clone(),
                pull: number,
                login: login.clone(),
                body: body.to_owned(),
            });
            let url = format!("https://github.com/{full}/pull/{number}#issuecomment-{id}");
            (
                201,
                json!({"id": id, "body": body, "html_url": url, "user": {"login": login, "type": "User"}}),
            )
        }
        ("POST", ["login", "oauth", "access_token"]) => {
            let field = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or_default();
            if field("client_id") != CLIENT_ID || field("client_secret") != CLIENT_SECRET {
                return (200, json!({"error": "incorrect_client_credentials"}));
            }
            let Some(login) = state.codes.get(field("code")).cloned() else {
                return (200, json!({"error": "bad_verification_code"}));
            };
            let token = format!("person-{login}");
            state.tokens.insert(token.clone(), login);
            (200, json!({"access_token": token, "token_type": "bearer"}))
        }
        ("GET", ["user"]) => match state.tokens.get(token) {
            Some(login) => (200, json!({"login": login})),
            None => (401, json!({"message": "Bad credentials"})),
        },
        _ => (404, json!({"message": "Not Found"})),
    }
}

/// A public key as a JWKS entry, the way GitHub publishes its Actions keys.
fn jwk(kid: &str, key: &RsaPublicKey) -> Value {
    json!({
        "kty": "RSA",
        "alg": "RS256",
        "use": "sig",
        "kid": kid,
        "n": URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),
        "e": URL_SAFE_NO_PAD.encode(key.e().to_bytes_be()),
    })
}

/// Whether a token is a JWT the app signed, naming it and not expired, as GitHub requires.
fn app_jwt_valid(token: &str, key: &RsaPublicKey) -> bool {
    let mut parts = token.split('.');
    let (Some(header), Some(claims), Some(signature)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Ok(signature) = URL_SAFE_NO_PAD.decode(signature) else {
        return false;
    };
    let Ok(signature) = Signature::try_from(signature.as_slice()) else {
        return false;
    };
    let signed = format!("{header}.{claims}");
    if VerifyingKey::<Sha256>::new(key.clone())
        .verify(signed.as_bytes(), &signature)
        .is_err()
    {
        return false;
    }
    let Ok(claims) = URL_SAFE_NO_PAD.decode(claims) else {
        return false;
    };
    let claims: Value = serde_json::from_slice(&claims).unwrap_or(Value::Null);
    let now = crate::now();
    claims["iss"] == CLIENT_ID
        && claims["exp"]
            .as_i64()
            .is_some_and(|exp| exp > now && exp <= now + 600)
        && claims["iat"].as_i64().is_some_and(|iat| iat <= now)
}
