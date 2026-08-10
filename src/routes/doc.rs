use axum::body::Bytes;
use axum::extract::{Path as AxPath, Query, State};
use axum::response::Response;
use axum::Json;
use serde_json::json;

use crate::auth::Session;
use crate::authz::{can_mutate, check_download, check_mutate};
use crate::error::{ok_json, ApiError, ApiResult};
use crate::fsops::resolve;
use crate::mime;
use crate::state::AppState;

/// 文本在线查看上限(超过则提示下载后处理)。
const TEXT_CAP: usize = 8 * 1024 * 1024;
/// 表格二进制在线编辑上限。
const BIN_CAP: usize = 32 * 1024 * 1024;

#[derive(serde::Deserialize)]
pub struct Q {
    #[serde(default)]
    pub path: String,
}

#[derive(serde::Deserialize)]
pub struct TextSaveReq {
    pub content: String,
}

/// 读取纯文本内容(仅文本类扩展名)。
pub async fn api_file_text_get(State(st): State<AppState>, session: Session, AxPath(share_id): AxPath<i64>, Query(q): Query<Q>) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, share_id, &q.path).await?;
    check_download(&session, &r.share, &rules, &r.rel)?;
    if !r.full.is_file() {
        return Err(ApiError::not_found("文件不存在"));
    }
    let ext = crate::path::ext_of(&r.rel);
    if !mime::editable_text(&ext) {
        return Err(ApiError::bad_request("该类型不支持文本预览"));
    }
    let len = tokio::fs::metadata(&r.full).await?.len() as usize;
    if len > TEXT_CAP {
        return Err(ApiError::bad_request("文件过大,请下载后编辑"));
    }
    let raw = tokio::fs::read(&r.full).await?;
    let text = String::from_utf8_lossy(&raw).to_string();
    Ok(ok_json(json!({
        "text": text,
        "size": len,
        "editable": can_mutate(&session, &r.share, &rules, &r.rel, |u| u.can_upload),
        "ext": ext,
    })))
}

/// 保存文本内容(需登录且有上传/写入权限)。
pub async fn api_file_text_put(State(st): State<AppState>, session: Session, AxPath(share_id): AxPath<i64>, Query(q): Query<Q>, Json(req): Json<TextSaveReq>) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, share_id, &q.path).await?;
    check_mutate(&session, &r.share, &rules, &r.rel, |u| u.can_upload)?;
    if !r.full.is_file() {
        return Err(ApiError::not_found("文件不存在"));
    }
    let ext = crate::path::ext_of(&r.rel);
    if !mime::editable_text(&ext) {
        return Err(ApiError::bad_request("该类型不支持在线编辑"));
    }
    if req.content.len() > TEXT_CAP {
        return Err(ApiError::bad_request("内容过大"));
    }
    atomic_write(&r.full, req.content.as_bytes()).await?;
    Ok(ok_json(json!({ "ok": true, "size": req.content.len() })))
}

/// 保存表格二进制(需登录且有上传/写入权限);仅允许覆盖已有表格文件。
pub async fn api_file_binary_put(State(st): State<AppState>, session: Session, AxPath(share_id): AxPath<i64>, Query(q): Query<Q>, body: Bytes) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, share_id, &q.path).await?;
    check_mutate(&session, &r.share, &rules, &r.rel, |u| u.can_upload)?;
    if !r.full.is_file() {
        return Err(ApiError::not_found("文件不存在"));
    }
    let ext = crate::path::ext_of(&r.rel);
    if !mime::office_sheet(&ext) {
        return Err(ApiError::bad_request("该类型不支持在线保存"));
    }
    if body.len() > BIN_CAP {
        return Err(ApiError::bad_request("文件过大"));
    }
    atomic_write(&r.full, &body).await?;
    Ok(ok_json(json!({ "ok": true, "size": body.len() })))
}

/// 同目录临时文件 + 原子重命名,避免写坏原文件。
async fn atomic_write(full: &std::path::Path, data: &[u8]) -> ApiResult<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dir = full.parent().ok_or_else(|| ApiError::internal("非法路径"))?;
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros()).unwrap_or(0);
    let tmp = dir.join(format!(".wp-save-{}-{ts}.tmp", std::process::id()));
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, full).await.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ApiError::internal(format!("保存失败: {e}"))
    })
}