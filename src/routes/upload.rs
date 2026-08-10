use std::io::Write;
use std::path::Path;

use axum::body::Bytes;
use axum::extract::{Path as AxPath, State};
use axum::response::Response;
use axum::Json;
use serde_json::json;

use crate::auth::{gen_upload_token, Session};
use crate::authz::{check_mutate, require_login};
use crate::db::UploadSession;
use crate::error::{ok_json, ApiError, ApiResult};
use crate::fsops::locate;
use crate::state::AppState;

#[derive(serde::Deserialize)]
pub struct UploadInitReq {
    pub share_id: i64,
    #[serde(default)]
    pub path: String,
    pub filename: String,
    pub size: i64,
}

pub async fn api_upload_init(State(st): State<AppState>, session: Session, Json(req): Json<UploadInitReq>) -> ApiResult<Response> {
    let u = require_login(&session)?.clone();
    if !u.can_upload {
        return Err(ApiError::forbidden("无上传权限"));
    }
    if req.size < 0 {
        return Err(ApiError::bad_request("文件大小非法"));
    }
    let filename = crate::path::sanitize_name(&req.filename).map_err(|e| ApiError::bad_request(e))?;
    let located = locate(&st, req.share_id, &req.path).await?;
    if !located.full.is_dir() {
        return Err(ApiError::not_found("目标目录不存在"));
    }
    let rules = st.db.rules(req.share_id).await?;
    check_mutate(&session, &located.share, &rules, &located.rel, |x| x.can_upload)?;
    let chunk = st.chunk_size;
    let num_parts = if req.size == 0 { 1 } else { (req.size + chunk - 1) / chunk };
    let token = gen_upload_token();
    let id = st
        .db
        .create_upload(&token, u.id, req.share_id, &located.rel, &filename, req.size, chunk, num_parts)
        .await?;
    std::fs::create_dir_all(st.uploads_dir.join(&token))?;
    Ok(ok_json(json!({
        "id": id, "token": token, "filename": filename,
        "share_id": req.share_id, "path": located.rel,
        "size": req.size, "chunk_size": chunk, "num_parts": num_parts,
    })))
}

fn received_parts(st: &AppState, token: &str) -> Vec<i64> {
    let mut v: Vec<i64> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(st.uploads_dir.join(token)) {
        for de in rd.flatten() {
            if let Some(rest) = de.file_name().to_string_lossy().strip_prefix("part") {
                if let Ok(i) = rest.parse() {
                    v.push(i);
                }
            }
        }
    }
    v.sort();
    v
}

fn session_info(st: &AppState, s: &UploadSession) -> serde_json::Value {
    json!({
        "id": s.id, "token": s.token, "filename": s.filename,
        "share_id": s.share_id, "path": s.rel_path,
        "size": s.total_size, "chunk_size": s.chunk_size, "num_parts": s.num_parts,
        "received": received_parts(st, &s.token), "updated_at": s.updated_at,
    })
}

async fn owned_session(st: &AppState, session: &Session, token: &str) -> ApiResult<UploadSession> {
    let s = st.db.upload_by_token(token).await?.ok_or_else(|| ApiError::not_found("上传会话不存在"))?;
    if s.user_id != session.user().map(|u| u.id).unwrap_or(-1) && !session.is_admin() {
        return Err(ApiError::forbidden("无权访问该上传会话"));
    }
    Ok(s)
}

pub async fn api_uploads_list(State(st): State<AppState>, session: Session) -> ApiResult<Response> {
    let u = require_login(&session)?;
    let out: Vec<serde_json::Value> = st.db.uploads_of(u.id).await?.iter().map(|s| session_info(&st, s)).collect();
    Ok(ok_json(json!({ "uploads": out })))
}

pub async fn api_upload_status(State(st): State<AppState>, session: Session, AxPath(token): AxPath<String>) -> ApiResult<Response> {
    let s = owned_session(&st, &session, &token).await?;
    Ok(ok_json(json!({ "session": session_info(&st, &s) })))
}

pub async fn api_upload_parts(State(st): State<AppState>, session: Session, AxPath(token): AxPath<String>) -> ApiResult<Response> {
    let s = owned_session(&st, &session, &token).await?;
    Ok(ok_json(json!({ "parts": received_parts(&st, &s.token) })))
}

pub async fn api_upload_part(State(st): State<AppState>, session: Session, AxPath((token, part)): AxPath<(String, i64)>, body: Bytes) -> ApiResult<Response> {
    let s = owned_session(&st, &session, &token).await?;
    if part < 0 || part >= s.num_parts {
        return Err(ApiError::bad_request("分片序号越界"));
    }
    if body.len() > s.chunk_size as usize {
        return Err(ApiError::bad_request("分片过大"));
    }
    let dir = st.uploads_dir.join(&token);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("part{part}")), &body)?;
    let _ = st.db.touch_upload(s.id).await;
    Ok(ok_json(json!({ "received": part, "size": body.len() })))
}

pub async fn api_upload_abort(State(st): State<AppState>, session: Session, AxPath(token): AxPath<String>) -> ApiResult<Response> {
    let s = owned_session(&st, &session, &token).await?;
    let _ = st.db.delete_upload(s.id).await;
    let _ = std::fs::remove_dir_all(st.uploads_dir.join(&token));
    Ok(ok_json(json!({ "ok": true })))
}

fn expected_part_len(s: &UploadSession, i: i64) -> i64 {
    if i + 1 == s.num_parts {
        let last = s.total_size % s.chunk_size;
        if last == 0 { s.chunk_size } else { last }
    } else {
        s.chunk_size
    }
}

fn merge_parts(dir: &Path, merged_tmp: &Path, target: &Path, num: usize, total: i64) -> Result<(), String> {
    let mut out = std::fs::File::create(merged_tmp).map_err(|e| e.to_string())?;
    for i in 0..num {
        let p = dir.join(format!("part{i}"));
        let mut f = std::fs::File::open(&p).map_err(|e| format!("缺少分片 part{i}: {e}"))?;
        std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);
    let sz = std::fs::metadata(merged_tmp).map(|m| m.len() as i64).unwrap_or(-1);
    if sz != total {
        let _ = std::fs::remove_file(merged_tmp);
        return Err(format!("合并后大小 {sz} 与预期 {total} 不符"));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(merged_tmp, target).map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn api_upload_complete(State(st): State<AppState>, session: Session, AxPath(token): AxPath<String>) -> ApiResult<Response> {
    let s = owned_session(&st, &session, &token).await?;
    let dir = st.uploads_dir.join(&token);
    for i in 0..s.num_parts {
        let want = expected_part_len(&s, i);
        let got = tokio::fs::metadata(dir.join(format!("part{i}"))).await.map(|m| m.len() as i64).unwrap_or(-1);
        if got != want {
            return Err(ApiError::conflict(format!("分片不完整,缺失或损坏的分片: {i}")));
        }
    }
    let located = locate(&st, s.share_id, &s.rel_path).await?;
    if !located.full.is_dir() {
        return Err(ApiError::not_found("目标目录不存在"));
    }
    if located.full.join(&s.filename).is_dir() {
        return Err(ApiError::conflict("同名文件夹已存在"));
    }
    let target = located.full.join(&s.filename);
    let merged_tmp = st.uploads_dir.join(format!("{token}.merge"));
    let dir_b = dir.clone();
    let target_b = target.clone();
    let num = s.num_parts as usize;
    let total = s.total_size;
    let result = tokio::task::spawn_blocking(move || merge_parts(&dir_b, &merged_tmp, &target_b, num, total))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    result.map_err(|e| ApiError::internal(e))?;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = st.db.delete_upload(s.id).await;
    let rel_target = if located.rel.is_empty() { s.filename.clone() } else { format!("{}/{}", located.rel, s.filename) };
    Ok(ok_json(json!({
        "ok": true,
        "share_id": s.share_id,
        "path": rel_target,
        "size": s.total_size,
    })))
}