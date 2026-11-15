//! Routes for the tests only, compiled in by the `test-hooks` feature, which a deployed worker
//! never has.

use worker::{Env, Request, Response};

use crate::Answer;

/// Runs the body, one SQL statement, on the worker's own D1: how a test makes time pass for a
/// record (a session, a token, a run) without a second writer on the database file, which the
/// worker would find busy.
pub async fn sql(mut req: Request, env: &Env) -> Answer {
    let statement = req.text().await?;
    env.d1("DB")?.prepare(&statement).run().await?;
    Ok(Response::empty()?.with_status(204))
}
