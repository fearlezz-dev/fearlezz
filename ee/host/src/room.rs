//! A work's room: one Durable Object per piece of work, holding the WebSockets of the pages
//! open on it and nothing else. A page that connects gets the work's snapshot from D1, the one
//! record; each event the API wrote to D1 then reaches every open page. The sockets hibernate
//! with the object, so an idle work costs nothing while its pages stay connected.

use worker::{
    DurableObject, Env, Request, Response, Result, State, WebSocket, WebSocketIncomingMessage,
    WebSocketPair, durable_object,
};

use crate::work;

#[durable_object]
pub struct WorkRoom {
    state: State,
    env: Env,
}

impl DurableObject for WorkRoom {
    fn new(state: State, env: Env) -> Self {
        WorkRoom { state, env }
    }

    /// `/connect?work=<id>` takes a page's socket and sends it the snapshot; `/event` sends an
    /// event to every page. Only the worker reaches these, never a page directly.
    async fn fetch(&self, mut req: Request) -> Result<Response> {
        match req.path().as_str() {
            "/connect" => {
                let url = req.url()?;
                let Some((_, work)) = url.query_pairs().find(|(name, _)| name == "work") else {
                    return Response::error("connect names no work", 400);
                };
                let pair = WebSocketPair::new()?;
                // Accepted before the snapshot is read: an event that lands meanwhile reaches
                // this socket too, and the page drops what its snapshot already holds by
                // sequence number, so nothing between the two is missed.
                self.state.accept_web_socket(&pair.server);
                let db = self.env.d1("DB")?;
                match work::snapshot(&db, &work).await? {
                    Some(snapshot) => {
                        let live = fearlezz_proof::events::Live::Snapshot(snapshot);
                        pair.server.send_with_str(serde_json::to_string(&live)?)?;
                    }
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
