//! Rooms: one Durable Object per piece of work and one per repository, each holding the
//! WebSockets of the pages open on it and nothing else. A page that connects gets the snapshot
//! from D1, the one record (a work's, or a repository's listing); each event the API wrote to D1
//! then reaches every open page. The sockets hibernate with the object, so an idle room costs
//! nothing while its pages stay connected.
//!
//! A work's room is named by the work's id, a repository's by `repo:<owner/name>`; ids are hex,
//! so the two never meet. The class keeps its first name, `WorkRoom`.

use fearlezz_proof::events::Live;
use worker::{
    DurableObject, Env, Method, Request, RequestInit, Response, Result, State, WebSocket,
    WebSocketIncomingMessage, WebSocketPair, durable_object,
};

use crate::{repository, work};

/// The room of a repository (lowercased).
pub fn of_repository(repo: &str) -> String {
    format!("repo:{repo}")
}

/// Sends `text` to every page open in room `room`.
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
    /// the snapshot; `/event` sends an event to every page; `/close?tag=` ends the pages with
    /// that tag. Only the worker reaches these, never a page directly.
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
                let event = req.text().await?;
                for socket in self.state.get_websockets() {
                    // A page that went away is cleaned up by its close; the rest still get it.
                    if let Err(error) = socket.send_with_str(&event) {
                        worker::console_log!("a page missed an event: {error}");
                    }
                }
                Response::empty()
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
