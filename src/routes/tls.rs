use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::json;

use crate::auth::Session;
use crate::authz::require_admin;
use crate::error::{err_json, ok_json, ApiError, ApiResult};
use crate::state::AppState;

#[derive(serde::Deserialize)]
pub struct PutCertReq {
    /// 证书对应的域名/备注,仅用于展示。
    pub domain: String,
    /// 粘贴模式:证书与私钥内容。
    #[serde(default)]
    pub cert_pem: Option<String>,
    #[serde(default)]
    pub key_pem: Option<String>,
    /// 文件模式:服务器上的证书与私钥文件路径。与 cert_pem/key_pem 任一组合,优先使用路径模式。
    #[serde(default)]
    pub cert_path: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
}

/// 当前证书与 HTTPS 状态(不返回私钥内容)。
pub async fn api_tls_get(State(st): State<AppState>, session: Session) -> ApiResult<Response> {
    require_admin(&session)?;
    let port = *st.tls.port.lock().await;
    let cert = st
        .db
        .tls_cert()
        .await
        .map_err(ApiError::from)?
        .map(|c| {
            json!({
                "id": c.id,
                "domain": c.domain,
                "created_at": c.created_at,
                "updated_at": c.updated_at,
            })
        });
    Ok(ok_json(json!({
        "enabled": port.is_some(),
        "tls_listen": st.tls_listen,
        "port": port,
        "cert": cert,
    })))
}

/// 读取服务器上的证书/私钥文件并返回内容。
fn read_cert_file(p: &str) -> Result<String, ApiError> {
    let full = std::path::PathBuf::from(p.trim());
    let canon = full
        .canonicalize()
        .map_err(|e| ApiError::bad_request(format!("文件不存在: {e}")))?;
    if !canon.is_file() {
        return Err(ApiError::bad_request("不是文件"));
    }
    std::fs::read_to_string(&canon)
        .map_err(|e| ApiError::bad_request(format!("读取文件失败: {e}")))
}

/// 校验并保存证书(粘贴内容或服务器文件路径二选一):解析失败直接 400,保存成功后 HTTPS 立即生效。
pub async fn api_tls_put(State(st): State<AppState>, session: Session, Json(req): Json<PutCertReq>) -> ApiResult<Response> {
    require_admin(&session)?;
    let domain = req.domain.trim().to_string();
    if domain.is_empty() {
        return Err(ApiError::bad_request("请填写证书对应的域名"));
    }
    let file_mode = req.cert_path.is_some() || req.key_path.is_some();
    let (cert_pem, key_pem) = if file_mode {
        let cp = req.cert_path.as_deref().unwrap_or("");
        let kp = req.key_path.as_deref().unwrap_or("");
        if cp.trim().is_empty() || kp.trim().is_empty() {
            return Err(ApiError::bad_request("请填写证书与私钥的服务器文件路径"));
        }
        (read_cert_file(cp)?, read_cert_file(kp)?)
    } else {
        let cp = req.cert_pem.as_deref().unwrap_or("").trim().to_string();
        let kp = req.key_pem.as_deref().unwrap_or("").trim().to_string();
        if cp.is_empty() || kp.is_empty() {
            return Err(ApiError::bad_request("请填写证书与私钥内容"));
        }
        (cp, kp)
    };
    crate::tls::build_server_config(&cert_pem, &key_pem).map_err(ApiError::bad_request)?;
    st.db.tls_upsert(&domain, &cert_pem, &key_pem).await.map_err(ApiError::from)?;
    st.tls.changed.notify_one();
    let port = *st.tls.port.lock().await;
    Ok(ok_json(json!({ "ok": true, "port": port })))
}

/// 删除证书,HTTPS 随之停用。
pub async fn api_tls_delete(State(st): State<AppState>, session: Session) -> ApiResult<Response> {
    require_admin(&session)?;
    st.db.tls_delete().await.map_err(ApiError::from)?;
    st.tls.changed.notify_one();
    Ok(ok_json(json!({ "ok": true })))
}

pub async fn api_tls_caption() -> Response {
    err_json(StatusCode::METHOD_NOT_ALLOWED, "仅支持 GET/PUT/DELETE")
}