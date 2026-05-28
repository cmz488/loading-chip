//! Axum HTTP 服务器
//!
//! 启动后台 tokio runtime，绑定 TCP 端口，注册 API 路由。

use crate::app::state::AppState;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};

use super::routes;

/// 启动 API 服务器（阻塞当前线程直到 shutdown）
#[allow(dead_code)]
///
/// 调用方式：
/// ```ignore
/// let state = AppState::new(registry);
/// start_server(state, "127.0.0.1:9876").await?;
/// ```
pub async fn start_server(state: AppState, addr: &str) -> Result<(), String> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 设置全局 RTT broadcast
    crate::debug::rtt::set_global_broadcast(state.rtt_tx.clone());

    let app = routes::api_router()
        .layer(cors)
        .with_state(state);

    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("绑定 {}: {}", addr, e))?;

    let actual_addr = listener.local_addr().map_err(|e| format!("获取地址: {}", e))?;
    eprintln!("🔌 API 服务已启动: http://{}", actual_addr);

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("服务器错误: {}", e))?;

    Ok(())
}

/// 在后台 tokio runtime 上启动 API 服务器（不阻塞）
///
/// 返回 shutdown 信号 sender，主线程可通过此关闭服务器。
pub fn spawn_server(
    state: AppState,
    addr: String,
) -> tokio::sync::oneshot::Sender<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");

        rt.block_on(async {
            let app = routes::api_router().with_state(state.clone());

            let listener = match TcpListener::bind(&addr).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("❌ API 绑定失败: {}", e);
                    return;
                }
            };

            let actual = listener.local_addr().unwrap_or_else(|_| addr.parse().unwrap());
            eprintln!("🔌 API 服务已启动: http://{}", actual);

            // 设置全局 RTT broadcast — TUI 和 API 共享同一通道
            crate::debug::rtt::set_global_broadcast(state.rtt_tx.clone());

            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap_or_else(|e| eprintln!("⚠️ API 服务器: {}", e));
        });
    });

    shutdown_tx
}
