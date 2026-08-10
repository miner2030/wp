pub mod auth;
pub mod authz;
pub mod config;
pub mod db;
pub mod error;
pub mod fsops;
pub mod media;
pub mod mime;
pub mod path;
pub mod router;
pub mod routes;
pub mod state;
pub mod stream;
pub mod testing;
pub mod ticket;
pub mod tls;
pub mod zip;

pub use testing::spawn_test_server;

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub async fn serve(cfg: &config::Config) -> Result<(), String> {
    let state = state::AppState::boot(cfg).await?;
    let app = router::build(state);
    let listener = tokio::net::TcpListener::bind(&cfg.listen)
        .await
        .map_err(|e| format!("监听 {} 失败: {e}", cfg.listen))?;
    tracing::info!("WebDisk 已启动: http://{}", cfg.listen);
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}