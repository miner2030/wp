use std::path::PathBuf;

use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::auth::Session;
use crate::authz::{access_of, can_mutate, can_read, check_download, check_mutate, check_read, private, Access};
use crate::error::{err_json, ok_json, ApiError, ApiResult};
use crate::fsops::{collect_dir, locate, remove_dir_all_count, resolve};
use crate::mime;
use crate::state::AppState;

#[derive(serde::Deserialize)]
pub struct Q {
    #[serde(default)]
    pub path: String,
}

#[derive(serde::Deserialize)]
pub struct MkdirReq {
    pub share_id: i64,
    #[serde(default)]
    pub path: String,
    pub name: String,
}

#[derive(serde::Deserialize)]
pub struct RenameReq {
    pub share_id: i64,
    #[serde(default)]
    pub path: String,
    pub old: String,
    pub new: String,
}

#[derive(serde::Deserialize)]
pub struct DeleteReq {
    pub share_id: i64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(serde::Deserialize)]
pub struct ZipReq {
    pub share_id: i64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub names: Vec<String>,
}

#[derive(serde::Deserialize)]
pub struct ZipGetQ {
    pub share_id: i64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub names: String,
    #[serde(default)]
    pub exp: i64,
    #[serde(default)]
    pub sig: String,
}

/// 复制下载链接签发请求:names 为空 = 单文件,否则为批量 zip。
#[derive(serde::Deserialize)]
pub struct TicketReq {
    pub share_id: i64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub names: Vec<String>,
}

/// 匿名下载 URL 查询参数。
#[derive(serde::Deserialize)]
pub struct DlQ {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub exp: i64,
    #[serde(default)]
    pub sig: String,
}

#[derive(serde::Serialize)]
struct Entry {
    name: String,
    is_dir: bool,
    size: i64,
    mtime: i64,
    ext: String,
    kind: &'static str,
}

pub async fn api_browse(State(st): State<AppState>, session: Session, AxPath(share_id): AxPath<i64>, Query(q): Query<Q>) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, share_id, &q.path).await?;
    check_read(&session, &r.share, &rules, &r.rel)?;
    if !r.full.is_dir() {
        return Err(ApiError::not_found("目录不存在"));
    }
    let mut entries = Vec::new();
    let rd = std::fs::read_dir(&r.full).map_err(|e| ApiError::internal(format!("读取失败: {e}")))?;
    for de in rd.flatten() {
        let name = de.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let child_rel = if r.rel.is_empty() { name.clone() } else { format!("{}/{}", r.rel, name) };
        if !can_read(&session, &r.share, &rules, &child_rel) {
            continue;
        }
        let md = std::fs::metadata(de.path()).ok();
        let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let ext = if is_dir { String::new() } else { crate::path::ext_of(&name) };
        let kind = if is_dir { "dir" } else { mime::kind_label(mime::kind_of(&ext)) };
        entries.push(Entry {
            name,
            is_dir,
            size: md.as_ref().map(|m| m.len() as i64).unwrap_or(0),
            mtime: md.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0),
            ext,
            kind,
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));

    Ok(ok_json(json!({
        "path": r.rel,
        "share": { "id": r.share.id, "name": r.share.name, "owner_id": r.share.owner_id, "home": r.share.kind == "home" },
        "can": {
            "upload": can_mutate(&session, &r.share, &rules, &r.rel, |u| u.can_upload),
            "mkdir": can_mutate(&session, &r.share, &rules, &r.rel, |u| u.can_mkdir),
            "delete": can_mutate(&session, &r.share, &rules, &r.rel, |u| u.can_delete),
            "download": crate::authz::can_download(&session, &r.share, &rules, &r.rel),
        },
        "entries": entries,
    })))
}

pub async fn api_file_meta(State(st): State<AppState>, session: Session, AxPath(share_id): AxPath<i64>, Query(q): Query<Q>) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, share_id, &q.path).await?;
    check_download(&session, &r.share, &rules, &r.rel)?;
    if !r.full.is_file() {
        return Err(ApiError::not_found("文件不存在"));
    }
    let meta = tokio::fs::metadata(&r.full).await?;
    let ext = crate::path::ext_of(&r.rel);
    let name = r.rel.rsplit('/').next().unwrap_or(&r.rel).to_string();
    Ok(ok_json(json!({
        "share_id": share_id,
        "path": r.rel,
        "name": name,
        "size": meta.len(),
        "mtime": meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0),
        "ext": ext,
        "mime": mime::mime_of(&ext),
        "kind": mime::kind_label(mime::kind_of(&ext)),
        "writable": can_mutate(&session, &r.share, &rules, &r.rel, |u| u.can_upload),
    })))
}

pub async fn api_mkdir(State(st): State<AppState>, session: Session, Json(req): Json<MkdirReq>) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, req.share_id, &req.path).await?;
    if !r.full.is_dir() {
        return Err(ApiError::not_found("父目录不存在"));
    }
    check_mutate(&session, &r.share, &rules, &r.rel, |u| u.can_mkdir)?;
    let name = crate::path::sanitize_name(&req.name).map_err(|e| ApiError::bad_request(e))?;
    match std::fs::create_dir(r.full.join(&name)) {
        Ok(_) => Ok(ok_json(json!({ "ok": true }))),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(ApiError::conflict("同名文件或文件夹已存在")),
        Err(e) => Err(ApiError::internal(format!("创建失败: {e}"))),
    }
}

pub async fn api_rename(State(st): State<AppState>, session: Session, Json(req): Json<RenameReq>) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, req.share_id, &req.path).await?;
    check_mutate(&session, &r.share, &rules, &r.rel, |x| x.can_delete)?;
    let old = crate::path::sanitize_name(&req.old).map_err(|e| ApiError::bad_request(e))?;
    let new = crate::path::sanitize_name(&req.new).map_err(|e| ApiError::bad_request(e))?;
    let from = r.full.join(&old);
    let to = r.full.join(&new);
    if !from.exists() {
        return Err(ApiError::not_found("原文件不存在"));
    }
    if to.exists() {
        return Err(ApiError::conflict("目标已存在"));
    }
    std::fs::rename(&from, &to).map_err(|e| ApiError::internal(format!("重命名失败: {e}")))?;
    Ok(ok_json(json!({ "ok": true })))
}

pub async fn api_delete(State(st): State<AppState>, session: Session, Json(req): Json<DeleteReq>) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, req.share_id, &req.path).await?;
    check_mutate(&session, &r.share, &rules, &r.rel, |x| x.can_delete)?;
    if !r.full.exists() {
        return Err(ApiError::not_found("目标不存在"));
    }
    let removed = if r.full.is_dir() {
        if !req.recursive {
            let mut it = std::fs::read_dir(&r.full).map_err(|e| ApiError::internal(e.to_string()))?;
            if it.next().is_some() {
                return Err(ApiError::conflict("文件夹非空,需要确认递归删除"));
            }
            std::fs::remove_dir(&r.full).map_err(|e| ApiError::internal(e.to_string()))?;
            1
        } else {
            remove_dir_all_count(&r.full) as i64
        }
    } else {
        std::fs::remove_file(&r.full).map_err(|e| ApiError::internal(e.to_string()))?;
        crate::media::purge(&st, req.share_id, &r.rel).await;
        1
    };
    Ok(ok_json(json!({ "ok": true, "removed": removed })))
}

pub async fn api_zip(State(st): State<AppState>, session: Session, Json(req): Json<ZipReq>) -> Response {
    let names = req.names.clone();
    zip_response(&st, &session, false, req.share_id, &req.path, &names).await.into_response()
}

pub async fn api_zip_get(State(st): State<AppState>, session: Session, Query(q): Query<ZipGetQ>) -> Response {
    let names: Vec<String> = q
        .names
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let authorized = if !q.sig.is_empty() {
        let rel = format!("{}|{}", q.path, names.join(","));
        match crate::ticket::verify(&st, "zip", q.share_id, &rel, q.exp, &q.sig) {
            Ok(()) => true,
            Err(e) => return e.into_response(),
        }
    } else {
        false
    };
    zip_response(&st, &session, authorized, q.share_id, &q.path, &names).await.into_response()
}

/// 签发复制下载链接(按空间规则授权,游客可签发 guest 可见文件):单文件 URL 以文件名结尾,批量多选为签名 zip URL。
pub async fn api_dlticket(State(st): State<AppState>, session: Session, Json(req): Json<TicketReq>) -> ApiResult<Response> {
    let url = if req.names.is_empty() {
        let (r, rules, _) = resolve(&st, req.share_id, &req.path).await?;
        check_download(&session, &r.share, &rules, &r.rel)?;
        if !r.full.is_file() {
            return Err(ApiError::not_found("文件不存在"));
        }
        let guest_ok = !private(&r.share) && access_of(&rules, &r.rel) == Access::Guest;
        crate::ticket::file_url(&st, r.share.id, &r.rel, !guest_ok)
    } else {
        let located = locate(&st, req.share_id, &req.path).await?;
        let rules = st.db.rules(located.share.id).await?;
        for name in &req.names {
            let Ok(name) = crate::path::sanitize_name(name) else { continue };
            let rel = if located.rel.is_empty() { name.clone() } else { format!("{}/{}", located.rel, name) };
            check_download(&session, &located.share, &rules, &rel)?;
        }
        crate::ticket::zip_url(&st, located.share.id, &located.rel, &req.names)
    };
    Ok(ok_json(json!({ "url": url, "expires_in": crate::ticket::TICKET_TTL })))
}

/// 下载响应管线:有签名 -> 验签放行;无签名 -> 按当前会话执行规则授权(guest 目录游客可下)。
async fn serve_dl(st: &AppState, headers: HeaderMap, session: Session, share_id: i64, rel: &str, exp: i64, sig: &str) -> Response {
    if !sig.is_empty() {
        if let Err(e) = crate::ticket::verify(st, "file", share_id, rel, exp, sig) {
            return e.into_response();
        }
    } else {
        let (r0, rules, _) = match resolve(st, share_id, rel).await {
            Ok(v) => v,
            Err(e) => return e.into_response(),
        };
        if let Err(e) = check_download(&session, &r0.share, &rules, &r0.rel) {
            return e.into_response();
        }
    }
    let (r, _, _) = match resolve(st, share_id, rel).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if !r.full.is_file() {
        return err_json(StatusCode::NOT_FOUND, "文件不存在或已失效");
    }
    let mime = mime::mime_of(&crate::path::ext_of(&r.rel));
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    crate::stream::stream(&r.full, range.as_deref(), &mime, false, true).await
}

/// 签名下载(匿名可用):URL 末段为真实文件名,curl/wget 直接保存为正确文件名。
pub async fn api_dl_get(State(st): State<AppState>, headers: HeaderMap, session: Session, AxPath((share_id, _name)): AxPath<(i64, String)>, Query(q): Query<DlQ>) -> Response {
    serve_dl(&st, headers, session, share_id, &q.path, q.exp, &q.sig).await
}

/// 短路径下载端点 /dl/:share_id/...:guest 可见文件无签名(短链接),私有文件带 exp/sig 签名。
pub async fn api_dl_short(State(st): State<AppState>, headers: HeaderMap, session: Session, AxPath(rest): AxPath<String>, Query(q): Query<DlQ>) -> Response {
    let mut it = rest.splitn(2, '/');
    let share_id: i64 = match it.next().and_then(|s| s.parse().ok()) {
        Some(v) => v,
        None => return err_json(StatusCode::NOT_FOUND, "共享不存在"),
    };
    let rel_raw = it.next().unwrap_or("");
    let rel = percent_encoding::percent_decode_str(rel_raw).decode_utf8_lossy();
    serve_dl(&st, headers, session, share_id, &rel, q.exp, &q.sig).await
}

async fn zip_response(st: &AppState, session: &Session, authorized: bool, share_id: i64, path: &str, names: &Vec<String>) -> ApiResult<Response> {
    let located = locate(st, share_id, path).await?;
    let rules = st.db.rules(share_id).await?;
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for name in names {
        let Ok(name) = crate::path::sanitize_name(name) else { continue };
        let full = located.full.join(&name);
        let rel = if located.rel.is_empty() { name } else { format!("{}/{}", located.rel, name) };
        if full.is_dir() {
            collect_dir(&full, &rel, &mut files);
        } else if full.is_file() {
            files.push((rel, full));
        }
    }
    if !authorized {
        for (rel, _) in &files {
            check_download(session, &located.share, &rules, rel)?;
        }
    }
    if files.is_empty() {
        return Err(ApiError::not_found("没有可打包的文件"));
    }
    let buf = crate::zip::build_zip(&files).map_err(|e| ApiError::internal(e))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/zip")
        .header("Content-Disposition", "attachment; filename=\"batch.zip\"")
        .body(axum::body::Body::from(buf))
        .unwrap())
}