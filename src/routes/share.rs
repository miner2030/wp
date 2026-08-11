use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::auth::{Session};
use crate::authz::{check_download, check_read, require_login};
use crate::error::{err_json, ok_json, ApiError, ApiResult};
use crate::fsops::{locate, resolve};
use crate::mime;
use crate::state::AppState;
use crate::stream::stream;

#[derive(serde::Deserialize)]
pub struct FileShareReq {
    pub share_id: i64,
    #[serde(default)]
    pub path: String,
}

#[derive(serde::Deserialize)]
pub struct ShareDlQ {
    #[serde(default)]
    pub dl: i64,
}

#[derive(serde::Deserialize)]
pub struct FileShareListQ {
    #[serde(default)]
    pub share_id: i64,
}

pub async fn api_fileshare_create(State(st): State<AppState>, session: Session, Json(req): Json<FileShareReq>) -> ApiResult<Response> {
    let u = require_login(&session)?.clone();
    let (r, rules, _) = resolve(&st, req.share_id, &req.path).await?;
    check_download(&session, &r.share, &rules, &r.rel)?;
    if !r.full.is_file() {
        return Err(ApiError::not_found("文件不存在"));
    }
    let fs = st.db.file_share_ensure(r.share.id, &r.rel, Some(u.id)).await.map_err(ApiError::internal)?;
    let meta = tokio::fs::metadata(&r.full).await?;
    let name = r.rel.rsplit('/').next().unwrap_or(&r.rel).to_string();
    Ok(ok_json(json!({
        "token": fs.token,
        "url": format!("/s/{}", fs.token),
        "name": name,
        "size": meta.len(),
        "created_at": fs.created_at,
        "hits": fs.hits,
    })))
}

pub async fn api_fileshare_list(State(st): State<AppState>, session: Session, Query(q): Query<FileShareListQ>) -> ApiResult<Response> {
    let u = require_login(&session)?.clone();
    if q.share_id <= 0 {
        return Err(ApiError::bad_request("缺少 share_id"));
    }
    let (r, rules, _) = resolve(&st, q.share_id, "").await?;
    check_read(&session, &r.share, &rules, &r.rel)?;
    let out = st
        .db
        .file_shares_of(r.share.id)
        .await?
        .into_iter()
        .filter(|f| u.is_admin || f.created_by == Some(u.id))
        .map(|f| json!({
            "token": f.token,
            "share_id": f.share_id,
            "path": f.rel_path,
            "created_at": f.created_at,
            "hits": f.hits,
        }))
        .collect::<Vec<_>>();
    Ok(ok_json(json!({ "links": out })))
}

pub async fn api_fileshare_delete(State(st): State<AppState>, session: Session, AxPath(token): AxPath<String>) -> ApiResult<Response> {
    let u = require_login(&session)?.clone();
    let f = st.db.file_share_by_token(&token).await?.ok_or_else(|| ApiError::not_found("分享不存在"))?;
    if !u.is_admin && f.created_by != Some(u.id) {
        return Err(ApiError::forbidden("无权取消他人创建的分享"));
    }
    st.db.delete_file_share(&token).await?;
    Ok(ok_json(json!({ "ok": true })))
}

async fn link_target(st: &AppState, token: &str) -> Result<(crate::fsops::Located, String), Response> {
    let f = st.db.file_share_by_token(token).await.map_err(|e| err_json(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let f = f.ok_or_else(|| err_json(StatusCode::NOT_FOUND, "分享不存在或已失效"))?;
    let located = locate(st, f.share_id, &f.rel_path).await.map_err(|e| e.into_response())?;
    let name = f.rel_path.rsplit('/').next().unwrap_or(&f.rel_path).to_string();
    Ok((located, name))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn human_size(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", n)
    } else {
        format!("{:.1} {}", v, units[i])
    }
}

pub async fn api_share_link_page(State(st): State<AppState>, AxPath(token): AxPath<String>) -> Response {
    let (r, name) = match link_target(&st, &token).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !r.full.is_file() {
        return err_json(StatusCode::NOT_FOUND, "文件不存在或已失效");
    }
    let size = match tokio::fs::metadata(&r.full).await {
        Ok(m) => m.len(),
        Err(_) => return err_json(StatusCode::NOT_FOUND, "文件不存在或已失效"),
    };
    let dl = format!("/s/{token}/api/download?dl=1");
    let preview = format!("/s/{token}/api/download");
    let inline = matches!(mime::kind_of(&crate::path::ext_of(&name)), mime::MediaKind::Image | mime::MediaKind::Video | mime::MediaKind::Audio);
    let esc = html_escape(&name);
    let dl_svg = r#"<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align:-2px;margin-right:6px"><path d="M12 5v11"/><path d="M7 12l5 5 5-5"/><path d="M5 19h14"/></svg>"#;
    let preview_svg = r#"<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align:-2px;margin-right:6px"><circle cx="11" cy="11" r="7"/><path d="M21 21l-4.4-4.4"/></svg>"#;
    let preview_btn = if inline {
        format!(r#"<a class="btn primary" href="{preview}">{preview_svg}在线预览</a>"#)
    } else {
        String::new()
    };
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>分享文件 - {esc}</title>
<style>
  body {{ margin:0; font-family:system-ui,-apple-system,"PingFang SC","Microsoft YaHei",sans-serif; background:#f2f4fa; color:#1f2430; }}
  .wrap {{ max-width:520px; margin:9vh auto; background:#fff; border-radius:14px; box-shadow:0 12px 34px rgba(20,26,48,.12); padding:34px 32px; }}
  h1 {{ font-size:17px; margin:0 0 6px; }}
  .name {{ font-size:13px; color:#4f6df5; word-break:break-all; margin-bottom:18px; }}
  .meta {{ font-size:13px; color:#8289a2; margin-bottom:22px; }}
  .btns {{ display:flex; gap:12px; flex-wrap:wrap; }}
  .btn {{ flex:1; min-width:180px; text-align:center; padding:12px 16px; border-radius:10px; text-decoration:none; font-size:14px; box-sizing:border-box; display:inline-flex; align-items:center; justify-content:center; }}
  .btn.primary {{ background:#4f6df5; color:#fff; }}
  .btn.ghost {{ background:#eef1fe; color:#3d5bdd; }}
  .tip {{ margin-top:20px; font-size:12px; color:#9aa3bd; }}
</style>
</head>
<body>
  <div class="wrap">
    <h1>文件分享</h1>
    <div class="name">{esc}</div>
    <div class="meta">大小 {size_readable} · 该链接无需登录即可访问</div>
    <div class="btns">
      {preview_btn}
      <a class="btn ghost" href="{dl}">{dl_svg}下载</a>
    </div>
    <div class="tip">由分享者创建,请勿转发给不希望看到此文件的人。</div>
  </div>
</body>
</html>"#,
        esc = esc,
        size_readable = human_size(size),
        preview_btn = preview_btn,
        dl = dl,
    );
    let mut resp = Response::new(axum::body::Body::from(html));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(header::CONTENT_TYPE, "text/html; charset=utf-8".parse().unwrap());
    resp
}

pub async fn api_share_link_download(State(st): State<AppState>, headers: HeaderMap, AxPath(token): AxPath<String>, Query(q): Query<ShareDlQ>) -> Response {
    let (r, _name) = match link_target(&st, &token).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !r.full.is_file() {
        return err_json(StatusCode::NOT_FOUND, "文件不存在或已失效");
    }
    let _ = st.db.file_share_bump(&token).await;
    let mime = mime::mime_of(&crate::path::ext_of(&r.rel));
    let inline = q.dl == 0 && matches!(mime::kind_of(&crate::path::ext_of(&r.rel)), mime::MediaKind::Image | mime::MediaKind::Video | mime::MediaKind::Audio);
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    stream(&r.full, range.as_deref(), &mime, inline, true).await
}