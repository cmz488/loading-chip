//! WebSocket /api/rtt — RTT 实时数据流

use axum::{
    extract::{
        ws::{Message, WebSocket},
        WebSocketUpgrade,
        State,
    },
    response::IntoResponse,
    Router,
};
use futures_util::SinkExt;
use futures_util::StreamExt;
use tokio::select;
use tokio::sync::broadcast;

use crate::app::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/rtt", axum::routing::get(rtt_handler))
}

async fn rtt_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_rtt_socket(socket, state))
}

async fn handle_rtt_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    let mut rtt_rx = state.rtt_tx.subscribe();

    let welcome = serde_json::json!({
        "type": "connected",
        "message": "RTT 数据流已连接"
    })
    .to_string();
    if sender.send(Message::Text(welcome.into())).await.is_err() {
        return;
    }

    loop {
        select! {
            rtt_msg = rtt_rx.recv() => {
                match rtt_msg {
                    Ok(event) => {
                        let json = serde_json::json!({
                            "type": "rtt",
                            "channel": event.channel,
                            "data": event.text,
                        }).to_string();
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let warn = serde_json::json!({
                            "type": "warning",
                            "message": format!("丢弃了 {} 条 RTT 消息", n)
                        }).to_string();
                        let _ = sender.send(Message::Text(warn.into())).await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            client_msg = receiver.next() => {
                match client_msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }
}
