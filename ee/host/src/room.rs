//! Rooms: one Durable Object per piece of work and one per repository, each holding the
//! WebSockets of the pages open on it. A page that connects gets the snapshot from D1, the one
//! record (a work's, or a repository's listing). A work's room also runs every write to its
//! work (`work::Write`), one at a time, so its pages get the events in the order of their
//! sequence numbers and the review is derived from the state the writes before it left. A
//! repository's room sends each work as it now stands. The sockets hibernate with the object,
//! so an idle room costs nothing while its pages stay connected.
//!
//! A work's room is named by the work's id, a repository's by `repo:<owner/name>`; ids are hex,
//! so the two never meet. The class keeps its first name, `WorkRoom`.

use fearlezz_proof::events::Live;
use wasm_bindgen::{JsCast as _, JsValue};
use worker::worker_sys::DurableObjectState;
use worker::{
    DurableObject, Env, Method, Request, RequestInit, Response, Result, State, WebSocket,
    WebSocketIncomingMessage, WebSocketPair, durable_object,
};

use crate::{repository, work};

/// The pages open in a room, owned, so a write the room runs holds them.
pub struct Pages(State);

impl Pages {
    fn of(state: &State) -> Pages {
        let raw: &JsValue = state.as_raw().as_ref();
        Pages(State::from(
            raw.clone().unchecked_into::<DurableObjectState>(),
        ))
    }

    /// Sends `text` to every open page.
    pub async fn send(&self, text: &str) {
        for socket in self.0.get_websockets() {
            // A page that went away is cleaned up by its close; the rest still get it.
            if let Err(error) = socket.send_with_str(text) {
                worker::console_log!("a page missed an event: {error}");
            }
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
/// (`work=<id>` or `repo=<owner/name>`). `tag`, when given, marks the socket so `close` can
/// end it.
pub async fn connect(
    env: &Env,
    req: &Request,
    room: &str,
    of: (&str, &str),
    tag: Option<&str>,
) -> Result<Response> {
    let mut url = url::Url::parse("https://room/connect").map_err(worker::Error::from)?;
    url.query_pairs_mut().append_pair(of.0, of.1);
    if let Some(tag) = tag {
        url.query_pairs_mut().append_pair("tag", tag);
    }
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

#[durable_object]
pub struct WorkRoom {
    state: State,
    env: Env,
}

impl WorkRoom {
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
                let pair = WebSocketPair::new()?;
                // Accepted before the snapshot is read: an event that lands meanwhile reaches
                // this socket too, and the page drops what its snapshot already holds by
                // sequence number, so nothing between the two is missed.
                match param("tag") {
                    Some(tag) => self
                        .state
                        .accept_websocket_with_tags(&pair.server, &[tag.as_str()]),
                    None => self.state.accept_web_socket(&pair.server),
                }
                match self.snapshot(of, &name).await? {
                    Some(text) => pair.server.send_with_str(text)?,
                    None => pair.server.close(Some(1008), Some("no such work"))?,
                }
                Response::from_websocket(pair.client)
            }
            "/event" => {
                Pages::of(&self.state).send(&req.text().await?).await;
                Response::empty()
            }
            "/write" => {
                let Some(work) = param("work") else {
                    return Response::error("write names no work", 400);
                };
                let write: work::Write = req.json().await?;
                let (pages, env) = (Pages::of(&self.state), self.env.clone());
                // No other event reaches the room until the write is done: writes to the work
                // run one at a time. A failure is the answer's, never the gate's, which would
                // reset the object and drop its pages.
                let done = self
                    .state
                    .block_concurrency_while(async move {
                        Ok(work::apply(&env, &pages, &work, write)
                            .await
                            .map_err(|e| e.to_string()))
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
