//! Rooms: one Durable Object per piece of work and one per repository, each holding the
//! WebSockets of the pages open on it. A page that connects gets the snapshot from D1, the one
//! record (a work's, or a repository's listing). A work's room also runs every write to its
//! work (`work::Write`), one at a time, so its pages get the events in the order of their
//! sequence numbers and the review is derived from the state the writes before it left. A
//! repository's room sends each work as it now stands. The sockets hibernate with the object,
//! so an idle room costs nothing while its pages stay connected.
//!
//! Nothing a write runs waits for GitHub. A write that can move the review asks the room's
//! alarm to publish it, right after: the alarm never runs twice at once and reads the review
//! last stored, so the pull request gets one check run and one comment and always the newest
//! review, and a GitHub that hangs or fails only delays it.
//!
//! A page stays open only while its viewer may still see it, the rule every page follows
//! (`access::sees`, GitHub's word cached 5 minutes): before each event the room asks the cache
//! alone, and on an alarm, at least every 5 minutes and right after an event the
//! cache could not answer for, it asks GitHub for each page a person opened, and closes with
//! `1008` the pages whose session ended or whose access GitHub took away. A share link's pages
//! stay until the link is revoked.
//!
//! A work's room is named by the work's id, a repository's by `repo:<owner/name>`; ids are hex,
//! so the two never meet. The class keeps its first name, `WorkRoom`.

use std::time::Duration;

use fearlezz_proof::events::Live;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast as _, JsValue};
use worker::worker_sys::DurableObjectState;
use worker::{
    DurableObject, Env, Method, Request, RequestInit, Response, Result, State, WebSocket,
    WebSocketIncomingMessage, WebSocketPair, durable_object,
};

use crate::access::{self, Asking, GITHUB_SECONDS, Viewer};
use crate::work::Derive;
use crate::{Config, repository, review, session, share, work};

/// Who a page's live connection is held for, which says how long it stays open: the viewer as
/// the socket keeps it, with no token, so a session that ends ends its pages.
#[derive(Serialize, Deserialize)]
pub(crate) enum Held {
    /// Someone the repository's rule let in, with their session when signed in: open while
    /// the rule still lets them in.
    Person {
        session: Option<String>,
        repo: String,
    },
    /// Someone holding a share link: open until the link is revoked, which closes the sockets
    /// carrying its `tag`.
    Link { tag: String },
}

impl Held {
    /// How a page of the repository `repo` (lowercased) the policy let `viewer` see is held.
    pub(crate) fn of(viewer: Viewer<'_>, repo: &str) -> Held {
        match viewer {
            Viewer::Share(link) => Held::Link {
                tag: share::tag(&link.id),
            },
            Viewer::Person(person) => Held::Person {
                session: person.session().map(str::to_owned),
                repo: repo.to_owned(),
            },
            Viewer::Anonymous => Held::Person {
                session: None,
                repo: repo.to_owned(),
            },
        }
    }
}

/// Why a page whose viewer may no longer see it is closed.
const ACCESS_ENDED: &str = "Your access to this page ended; sign in again to see it.";

/// The review publish the room's alarm owes GitHub, kept in its storage.
const PUBLISH: &str = "publish";

/// A publish the alarm owes: the work's newest review, and whether a CI run opened since the
/// last one, which creates the pull request's comment when it has none.
#[derive(Serialize, Deserialize)]
struct Publish {
    work: String,
    first_run: bool,
}

/// The room's own state, owned, so a write the room runs holds it.
fn own(state: &State) -> State {
    let raw: &JsValue = state.as_raw().as_ref();
    State::from(raw.clone().unchecked_into::<DurableObjectState>())
}

/// Has the room's alarm run now, before the 5 minute one it may have set.
async fn alarm_now(state: &State) -> Result<()> {
    state.storage().set_alarm(Duration::ZERO).await
}

/// The pages open in a room, owned, so a write the room runs holds them.
pub struct Pages {
    state: State,
    env: Env,
}

impl Pages {
    fn of(state: &State, env: &Env) -> Pages {
        Pages {
            state: own(state),
            env: env.clone(),
        }
    }

    /// Sends `text` to every open page whose viewer may still see it, as the cache says. When
    /// it cannot say, the page gets it and the alarm asks GitHub right after.
    pub async fn send(&self, text: &str) {
        let (open, unknown) = self.admitted(Asking::Cache).await;
        for socket in open {
            // A page that went away is cleaned up by its close; the rest still get it.
            if let Err(error) = socket.send_with_str(text) {
                worker::console_log!("a page missed an event: {error}");
            }
        }
        if unknown && let Err(error) = alarm_now(&self.state).await {
            worker::console_error!("the room did not ask again who sees its pages: {error}");
        }
    }

    /// The open pages whose viewers may still see them, after closing the rest, and whether
    /// any was kept only because `asking` could not say. A person's access is asked once per
    /// session for all their pages; when the asking itself fails (D1 or GitHub down), the page
    /// stays and is asked again next time.
    async fn admitted(&self, asking: Asking) -> (Vec<WebSocket>, bool) {
        let config = match Config::from(&self.env) {
            Ok(config) => config,
            Err(error) => {
                worker::console_error!("the room cannot read its settings: {error}");
                return (self.state.get_websockets(), false);
            }
        };
        let mut asked: Vec<(Option<String>, String, Option<bool>)> = Vec::new();
        let mut open = Vec::new();
        let mut unknown = false;
        for socket in self.state.get_websockets() {
            let Ok(Some(Held::Person { session, repo })) = socket.deserialize_attachment() else {
                open.push(socket);
                continue;
            };
            let known = asked
                .iter()
                .find(|(s, r, _)| *s == session && *r == repo)
                .map(|(_, _, sees)| *sees);
            let sees = match known {
                Some(sees) => sees,
                None => {
                    let sees = sees(&self.env, &config, session.as_deref(), &repo, asking).await;
                    asked.push((session, repo, sees));
                    sees
                }
            };
            if sees == Some(false) {
                if let Err(error) = socket.close(Some(1008), Some(ACCESS_ENDED)) {
                    worker::console_log!("a page did not close: {error}");
                }
            } else {
                unknown |= sees.is_none();
                open.push(socket);
            }
        }
        (open, unknown)
    }

    /// Whether a person still has a page open here, whose access the alarm asks again.
    fn has_people(&self) -> bool {
        self.state.get_websockets().iter().any(|socket| {
            matches!(
                socket.deserialize_attachment(),
                Ok(Some(Held::Person { .. }))
            )
        })
    }
}

/// Whether the person of session `session` (gone once they log out or it expires), or anyone
/// without one, may still see the repository `repo`'s pages, as `asking` can say: None when it
/// cannot, so the page stays until it is asked again.
async fn sees(
    env: &Env,
    config: &Config,
    session: Option<&str>,
    repo: &str,
    asking: Asking,
) -> Option<bool> {
    let asked = async {
        let person = match session {
            Some(id) => session::find(env, id).await?,
            None => None,
        };
        access::sees(env, config, Viewer::of(person.as_ref()), repo, asking).await
    };
    match asked.await {
        Ok(sees) => sees,
        Err(refusal) if refusal.status < 500 => Some(false),
        Err(refusal) => {
            worker::console_error!("whether a page may stay open was not known: {refusal:?}");
            Some(true)
        }
    }
}

/// Runs `write` on work `work` in its room, after every write to it sent before.
pub async fn write(env: &Env, work: &str, write: &work::Write) -> Result<()> {
    let mut url = url::Url::parse("https://room/write").map_err(worker::Error::from)?;
    url.query_pairs_mut().append_pair("work", work);
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_body(Some(serde_json::to_string(write)?.into()));
    let mut answer = fetch(env, work, url.as_str(), &init).await?;
    if !(200..300).contains(&answer.status_code()) {
        return Err(worker::Error::RustError(format!(
            "the room of work {work} did not write: {}",
            answer.text().await?
        )));
    }
    Ok(())
}

/// The room of a repository (lowercased).
pub fn of_repository(repo: &str) -> String {
    format!("repo:{repo}")
}

/// Sends `text` to every page open in room `room`: a repository's, whose pages keep the newest
/// of each work by its sequence number.
pub async fn send(env: &Env, room: &str, text: &str) -> Result<()> {
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_body(Some(text.into()));
    fetch(env, room, "https://room/event", &init).await?;
    Ok(())
}

/// Hands a page's live connection to room `room`, which answers it with what `of` names
/// (`work=<id>` or `repo=<owner/name>`) and holds it as `held`.
pub(crate) async fn connect(
    env: &Env,
    req: &Request,
    room: &str,
    of: (&str, &str),
    held: &Held,
) -> Result<Response> {
    let mut url = url::Url::parse("https://room/connect").map_err(worker::Error::from)?;
    url.query_pairs_mut()
        .append_pair(of.0, of.1)
        .append_pair("viewer", &serde_json::to_string(held)?);
    let mut init = RequestInit::new();
    init.with_headers(req.headers().clone());
    fetch(env, room, url.as_str(), &init).await
}

/// Ends every page of room `room` whose socket carries `tag`, saying `why`.
pub async fn close(env: &Env, room: &str, tag: &str, why: &str) -> Result<()> {
    let mut url = url::Url::parse("https://room/close").map_err(worker::Error::from)?;
    url.query_pairs_mut()
        .append_pair("tag", tag)
        .append_pair("why", why);
    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    fetch(env, room, url.as_str(), &init).await?;
    Ok(())
}

async fn fetch(env: &Env, room: &str, url: &str, init: &RequestInit) -> Result<Response> {
    let stub = env.durable_object("WORK")?.id_from_name(room)?.get_stub()?;
    stub.fetch_with_request(Request::new_with_init(url, init)?)
        .await
}

/// Has the room's alarm publish the newest review of work `work` right after this write;
/// `first_run` stays owed until a publish runs.
async fn owe(state: &State, work: String, first_run: bool) -> Result<()> {
    let storage = state.storage();
    let owed = storage.get::<Publish>(PUBLISH).await?;
    let first_run = first_run || owed.is_some_and(|p| p.first_run);
    storage.put(PUBLISH, Publish { work, first_run }).await?;
    alarm_now(state).await
}

#[durable_object]
pub struct WorkRoom {
    state: State,
    env: Env,
}

impl WorkRoom {
    /// Makes sure the alarm that asks again about people's access is set.
    async fn watch(&self) -> Result<()> {
        let storage = self.state.storage();
        if storage.get_alarm().await?.is_none() {
            storage
                .set_alarm(Duration::from_secs(GITHUB_SECONDS as u64))
                .await?;
        }
        Ok(())
    }

    /// What a page gets first: the work's snapshot or the repository's listing, as JSON; none
    /// when there is no such work or repository.
    async fn snapshot(&self, of: &str, name: &str) -> Result<Option<String>> {
        let db = self.env.d1("DB")?;
        Ok(match of {
            "work" => match work::snapshot(&db, name).await? {
                Some(snapshot) => Some(serde_json::to_string(&Live::Snapshot(snapshot))?),
                None => None,
            },
            "repo" => match repository::listing(&db, name).await? {
                Some(listing) => Some(serde_json::to_string(&listing)?),
                None => None,
            },
            _ => None,
        })
    }
}

impl DurableObject for WorkRoom {
    fn new(state: State, env: Env) -> Self {
        WorkRoom { state, env }
    }

    /// `/connect?work=<id>` or `/connect?repo=<owner/name>` takes a page's socket and sends it
    /// the snapshot; `/event` sends an event to every page (a repository's); `/write?work=<id>`
    /// runs a write to the work; `/close?tag=` ends the pages with that tag. Only the worker
    /// reaches these, never a page directly.
    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let url = req.url()?;
        let param = |name: &str| {
            url.query_pairs()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.into_owned())
        };
        match req.path().as_str() {
            "/connect" => {
                let Some((of, name)) = ["work", "repo"]
                    .into_iter()
                    .find_map(|of| param(of).map(|name| (of, name)))
                else {
                    return Response::error("connect names no work or repository", 400);
                };
                let Some(held) =
                    param("viewer").and_then(|v| serde_json::from_str::<Held>(&v).ok())
                else {
                    return Response::error("connect names no viewer", 400);
                };
                let pair = WebSocketPair::new()?;
                // Accepted before the snapshot is read: an event that lands meanwhile reaches
                // this socket too, and the page drops what its snapshot already holds by
                // sequence number, so nothing between the two is missed.
                match &held {
                    Held::Link { tag } => self
                        .state
                        .accept_websocket_with_tags(&pair.server, &[tag.as_str()]),
                    Held::Person { .. } => self.state.accept_web_socket(&pair.server),
                }
                pair.server.serialize_attachment(&held)?;
                if matches!(held, Held::Person { .. }) {
                    self.watch().await?;
                }
                match self.snapshot(of, &name).await? {
                    Some(text) => pair.server.send_with_str(text)?,
                    None => pair.server.close(Some(1008), Some("no such work"))?,
                }
                Response::from_websocket(pair.client)
            }
            "/event" => {
                Pages::of(&self.state, &self.env)
                    .send(&req.text().await?)
                    .await;
                Response::empty()
            }
            "/write" => {
                let Some(work) = param("work") else {
                    return Response::error("write names no work", 400);
                };
                let write: work::Write = req.json().await?;
                let (pages, env) = (Pages::of(&self.state, &self.env), self.env.clone());
                let state = own(&self.state);
                // No other event reaches the room until the write is done: writes to the work
                // run one at a time, and nothing in them waits for GitHub. A failure is the
                // answer's, never the gate's, which would reset the object and drop its pages.
                let done = self
                    .state
                    .block_concurrency_while(async move {
                        let derive = write.derive;
                        let applied = async {
                            work::apply(&env, &pages, &work, write).await?;
                            if derive != Derive::No {
                                owe(&state, work, derive == Derive::FirstRun).await?;
                            }
                            Ok::<_, worker::Error>(())
                        };
                        Ok(applied.await.map_err(|e| e.to_string()))
                    })
                    .await?;
                match done {
                    Ok(()) => Response::empty(),
                    Err(error) => Response::error(error, 500),
                }
            }
            "/close" => {
                let (Some(tag), why) = (param("tag"), param("why").unwrap_or_default()) else {
                    return Response::error("close names no tag", 400);
                };
                for socket in self.state.get_websockets_with_tag(&tag) {
                    if let Err(error) = socket.close(Some(1008), Some(why.as_str())) {
                        worker::console_log!("a page did not close: {error}");
                    }
                }
                Response::empty()
            }
            _ => Response::error("not found", 404),
        }
    }

    /// Publishes the review it owes GitHub, if any, then asks GitHub again whether each person
    /// with a page open may still see it, and keeps asking while one has.
    async fn alarm(&self) -> Result<Response> {
        let storage = self.state.storage();
        if let Some(publish) = storage.get::<Publish>(PUBLISH).await? {
            // Taken before it runs: a write meanwhile owes the next one, with its newer review.
            storage.delete(PUBLISH).await?;
            review::publish(&self.env, &publish.work, publish.first_run).await;
        }
        let pages = Pages::of(&self.state, &self.env);
        pages.admitted(Asking::GitHub).await;
        if pages.has_people() {
            self.watch().await?;
        }
        Response::empty()
    }

    /// Pages only listen; what they send is ignored.
    async fn websocket_message(
        &self,
        _socket: WebSocket,
        _message: WebSocketIncomingMessage,
    ) -> Result<()> {
        Ok(())
    }

    async fn websocket_close(
        &self,
        socket: WebSocket,
        code: usize,
        reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        // 1005 and 1006 say no code was sent; they may not be sent back.
        let code = u16::try_from(code)
            .ok()
            .filter(|c| !matches!(c, 1005 | 1006))
            .unwrap_or(1000);
        socket.close(Some(code), Some(reason))
    }
}
