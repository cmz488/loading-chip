//! WebSocket /api/rtt — RTT 实时数据流
//!
//! 客户端连接后持续接收 RTT 输出事件，直到断开。

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

/// WebSocket 升级 handler
async fn rtt_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_rtt_socket(socket, state))
}

/// WebSocket 连接处理
async fn handle_rtt_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // 订阅 RTT 广播
    let mut rtt_rx = state.rtt_tx.subscribe();

    // 发送欢迎消息
    let welcome = serde_json::json!({
        "type": "connected",
        "message": "RTT 数据流已连接"
    })
    .to_string();
    if sender
        .send(Message::Text(welcome.into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        select! {
            // 接收 RTT 广播
            rtt_msg = rtt_rx.recv() => {
                match rtt_msg {
                    Ok(event) => {
                        let json = serde_json::json!({
                            "type": "rtt",
                            "channel": event.channel,
                            "data": event.data,
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
            // 接收客户端消息（用于心跳/ping）
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
