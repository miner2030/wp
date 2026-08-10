use std::io::Cursor;
use std::sync::Arc;

use axum::http::Request;
use axum::response::Response;
use tower::Service;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio_rustls::TlsAcceptor;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;

use crate::db::TlsCert;
use crate::state::AppState;

/// TLS 运行态:当前生效的 rustls 配置(listener 每次 accept 前读取)+ 实际监听端口。
pub struct TlsRuntime {
    pub config: RwLock<Option<Arc<rustls::ServerConfig>>>,
    pub port: Mutex<Option<u16>>,
    pub(crate) changed: Notify,
}

impl Default for TlsRuntime {
    fn default() -> Self {
        TlsRuntime {
            config: RwLock::new(None),
            port: Mutex::new(None),
            changed: Notify::new(),
        }
    }
}

/// 把 PEM 证书链 + 私钥解析成 rustls 配置;任何一步失败都拒绝(证书与私钥不匹配也会失败)。
pub fn build_server_config(cert_pem: &str, key_pem: &str) -> Result<Arc<rustls::ServerConfig>, String> {
    let mut cert_rdr = Cursor::new(cert_pem.as_bytes());
    let certs: Vec<rustls::Certificate> = rustls_pemfile::certs(&mut cert_rdr)
        .map_err(|e| format!("证书解析失败: {e}"))?
        .into_iter()
        .map(rustls::Certificate)
        .collect();
    if certs.is_empty() {
        return Err("证书内容中没有找到 PEM 证书".into());
    }
    let mut key_rdr = Cursor::new(key_pem.as_bytes());
    let key_der = [rustls_pemfile::pkcs8_private_keys, rustls_pemfile::rsa_private_keys, rustls_pemfile::ec_private_keys]
        .into_iter()
        .map(|f| f(&mut key_rdr))
        .collect::<Result<Vec<_>, _>>()
        .map(|v| v.into_iter().flatten().next())
        .map_err(|e| format!("私钥解析失败: {e}"))?
        .ok_or_else(|| "私钥内容中没有找到 PEM 私钥".to_string())?;
    let key = rustls::PrivateKey(key_der);
    let cfg = rustls::ServerConfig::builder()
        .with_safe_default_cipher_suites()
        .with_safe_default_kx_groups()
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| format!("TLS 版本配置失败: {e}"))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("证书与私钥不匹配: {e}"))?;
    Ok(Arc::new(cfg))
}

/// HTTPS 后台任务:轮询数据库证书变化,启停 TLS 监听,并按最新配置为每个连接握手。
pub async fn run(st: AppState) {
    let mut serving: Option<Serving> = None;
    loop {
        let enabled = reload_config(&st).await;
        if enabled {
            if serving.is_none() {
                match tokio::net::TcpListener::bind(st.tls_listen.as_str()).await {
                    Ok(l) => {
                        let port = l.local_addr().map(|a| a.port()).ok();
                        *st.tls.port.lock().await = port;
                        tracing::info!("HTTPS 已启用: https://{}", st.tls_listen);
                        let stop = Arc::new(Notify::new());
                        let handle = tokio::spawn(accept_loop(st.clone(), Arc::new(l), stop.clone()));
                        serving = Some(Serving { stop, handle });
                    }
                    Err(e) => tracing::warn!("HTTPS 监听 {} 失败: {e}", st.tls_listen),
                }
            }
        } else if let Some(s) = serving.take() {
            s.stop.notify_waiters();
            let _ = s.handle.await;
            *st.tls.port.lock().await = None;
            tracing::info!("HTTPS 已停用");
        }
        st.tls.changed.notified().await;
    }
}

async fn reload_config(st: &AppState) -> bool {
    let row: Option<TlsCert> = match st.db.tls_cert().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("读取证书失败: {e}");
            None
        }
    };
    let cfg = match &row {
        Some(c) => match build_server_config(&c.cert_pem, &c.key_pem) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                tracing::error!("当前证书无效,HTTPS 不可用: {e}");
                None
            }
        },
        None => None,
    };
    *st.tls.config.write().await = cfg.clone();
    cfg.is_some()
}

struct Serving {
    stop: Arc<Notify>,
    handle: tokio::task::JoinHandle<()>,
}

async fn accept_loop(st: AppState, listener: Arc<tokio::net::TcpListener>, stop: Arc<Notify>) {
    loop {
        tokio::select! {
            _ = stop.notified() => break,
            res = listener.accept() => {
                match res {
                    Ok((tcp, _)) => {
                        let st = st.clone();
                        tokio::spawn(async move { handle_conn(st, tcp).await });
                    }
                    Err(e) => {
                        tracing::debug!("TLS accept 失败: {e}");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_conn(st: AppState, tcp: TcpStream) {
    let cfg = match st.tls.config.read().await.as_ref() {
        Some(c) => c.clone(),
        None => return,
    };
    let tls = match TlsAcceptor::from(cfg).accept(tcp).await {
        Ok(t) => t,
        Err(_) => return,
    };
    let app = crate::router::build(st.clone());
    let service = tower::service_fn(move |req: Request<hyper::body::Incoming>| {
        let mut app = app.clone();
        async move {
            let resp: Response = app
                .call(req.map(axum::body::Body::new))
                .await
                .expect("router 不会失败");
            Ok::<_, std::convert::Infallible>(resp)
        }
    });
    let hyper_service = TowerToHyperService::new(service);
    let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(tls), hyper_service)
        .await;
}