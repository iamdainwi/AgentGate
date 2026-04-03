use super::state::DashboardState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

#[derive(Deserialize)]
pub struct WsQuery {
    /// Bearer token passed as a query parameter because the browser WebSocket API
    /// does not support setting custom request headers.
    pub token: Option<String>,
}

pub async fn ws_live_handler(
    ws: WebSocketUpgrade,
    State(state): State<DashboardState>,
    Query(q): Query<WsQuery>,
) -> impl IntoResponse {
    // Validate token before the HTTP→WS upgrade. A missing or wrong token gets a
    // plain 401 HTTP response — the connection is never upgraded.
    match q.token.as_deref() {
        Some(t) if t == state.auth_token => {}
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                "Invalid or missing ?token= query parameter",
            )
                .into_response();
        }
    }

    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: DashboardState) {
    let mut rx = state.live_tx.subscribe();
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(record) => {
                        match serde_json::to_string(&record) {
                            Ok(json) => {
                                if sender.send(Message::Text(json)).await.is_err() {
                                    return;
                                }
                            }
                            Err(e) => tracing::warn!("WS serialisation failed: {e}"),
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::warn!("Live stream receiver lagged, skipped {n} records");
                    }
                    Err(RecvError::Closed) => return,
                }
            }
            msg = receiver.next() => {
                // Any message from the client (including close frames) terminates the loop.
                if msg.is_none() {
                    return;
                }
            }
        }
    }
}
