/// 为集成测试在 `127.0.0.1:0` 上启动服务,返回 (base_url, JoinHandle)。
pub fn spawn_test_server(admin_user: &str, admin_pass: &str, data_dir: &str) -> Result<(String, std::thread::JoinHandle<()>), String> {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let addr = std_listener.local_addr().map_err(|e| e.to_string())?;
    let url = format!("http://{addr}");

    let cfg = crate::config::Config {
        listen: String::new(),
        data_dir: data_dir.into(),
        admin_user: admin_user.into(),
        admin_pass: admin_pass.into(),
        chunk_size: 1024 * 1024,
        tls_listen: "127.0.0.1:0".into(),
    };

    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            let state = match crate::state::AppState::boot(&cfg).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("server boot failed: {e}");
                    return;
                }
            };
            let _ = std_listener.set_nonblocking(true);
            let listener = match tokio::net::TcpListener::from_std(std_listener) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("listener bind failed: {e}");
                    return;
                }
            };
            let app = crate::router::build(state);
            let _ = axum::serve(listener, app).await;
        });
    });

    Ok((url, handle))
}