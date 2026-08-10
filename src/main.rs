#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "wp=info,tower_http=info".to_string()),
        )
        .init();

    let Some(cfg) = wp::config::parse_args() else { return };

    if let Err(e) = wp::serve(&cfg).await {
        eprintln!("启动失败: {e}");
        std::process::exit(1);
    }
}