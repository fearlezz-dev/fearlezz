//! GitHub's answers kept for a while in D1, so a page view does not ask GitHub every time: who
//! may read a repository, whether it is public, and the Actions OIDC keys.

use worker::{Env, Result};

use crate::now;

/// A cached answer, if one is there and not expired.
pub async fn get(env: &Env, key: &str) -> Result<Option<String>> {
    env.d1("DB")?
        .prepare("SELECT value FROM cache WHERE key = ?1 AND expires > ?2")
        .bind(&[key.into(), (now() as f64).into()])?
        .first(Some("value"))
        .await
}

/// Keeps an answer for `seconds`, replacing whatever was kept under its key.
pub async fn put(env: &Env, key: &str, value: &str, seconds: i64) -> Result<()> {
    env.d1("DB")?
        .prepare("INSERT OR REPLACE INTO cache (key, value, expires) VALUES (?1, ?2, ?3)")
        .bind(&[key.into(), value.into(), ((now() + seconds) as f64).into()])?
        .run()
        .await?;
    Ok(())
}
