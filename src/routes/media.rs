use std::path::PathBuf;

use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::auth::Session;
use crate::authz::check_download;
use crate::error::{err_json, ok_json, ApiError, ApiResult};
use crate::fsops::resolve;
use crate::mime;
use crate::state::AppState;
use crate::stream::{stream, urlencode};

#[derive(serde::Deserialize)]
pub struct Q {
    #[serde(default)]
    pub path: String,
}

#[derive(serde::Deserialize)]
pub struct TQ {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub t: i64,
    #[serde(default)]
    pub dl: i64,
}

async fn file_source(st: &AppState, session: &Session, share_id: i64, rel: &str, use_transcoded: bool) -> Result<(crate::fsops::Located, String), Response> {
    let (r, rules, _) = resolve(st, share_id, rel).await.map_err(|e| e.into_response())?;
    check_download(session, &r.share, &rules, &r.rel).map_err(|e| e.into_response())?;
    let mut file = r.full.clone();
    if use_transcoded {
        let tc = st.db.transcode(r.share.id, &r.rel).await.map_err(|e| err_json(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        match tc {
            Some(t) if t.status == "done" && PathBuf::from(&t.out_path).is_file() => file = PathBuf::from(t.out_path),
            _ => return Err(err_json(StatusCode::NOT_FOUND, "转码尚未完成")),
        }
    }
    if !file.is_file() {
        return Err(err_json(StatusCode::NOT_FOUND, "文件不存在"));
    }
    let mime = if use_transcoded { "video/mp4".to_string() } else { mime::mime_of(&crate::path::ext_of(&r.rel)) };
    Ok((crate::fsops::Located { share: r.share, root: r.root, rel: r.rel, full: file }, mime))
}

pub async fn api_media_status(State(st): State<AppState>, session: Session, AxPath(share_id): AxPath<i64>, Query(q): Query<Q>) -> ApiResult<Response> {
    let (r, rules, _) = resolve(&st, share_id, &q.path).await?;
    check_download(&session, &r.share, &rules, &r.rel)?;
    if !r.full.is_file() {
        return Err(ApiError::not_found("文件不存在"));
    }
    let ext = crate::path::ext_of(&r.rel);
    let kind = mime::kind_of(&ext);
    let resp = match kind {
        mime::MediaKind::Image => json!({ "kind": "image", "direct": true }),
        mime::MediaKind::Audio => json!({ "kind": "audio", "direct": true }),
        mime::MediaKind::Video if mime::native_video(&ext) => {
            json!({ "kind": "video", "direct": true, "src": format!("/api/media/{share_id}?path={}", urlencode(&r.rel)) })
        }
        mime::MediaKind::Video => {
            if st.ffmpeg.is_none() {
                json!({ "kind": "video", "direct": false, "state": "unsupported" })
            } else {
                let tc = st.db.transcode(r.share.id, &r.rel).await?;
                match tc {
                    Some(t) if t.status == "done" && PathBuf::from(&t.out_path).is_file() => {
                        json!({ "kind": "video", "direct": false, "state": "ready", "src": format!("/api/media/{}?path={}&t=1", share_id, urlencode(&r.rel)) })
                    }
                    Some(t) if t.status == "running" => json!({ "kind": "video", "direct": false, "state": "converting" }),
                    _ => {
                        crate::media::ensure(&st, share_id, &r.rel, &r.full).await.map_err(ApiError::internal)?;
                        json!({ "kind": "video", "direct": false, "state": "converting" })
                    }
                }
            }
        }
        _ => json!({ "kind": "other", "direct": false }),
    };
    Ok(ok_json(resp))
}

fn previewable(ext: &str) -> bool {
    mime::previewable(ext)
}

pub async fn api_media_get(State(st): State<AppState>, session: Session, headers: HeaderMap, AxPath(share_id): AxPath<i64>, Query(q): Query<TQ>) -> Response {
    let (r, mime) = match file_source(&st, &session, share_id, &q.path, q.t != 0).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    let inline = q.dl == 0 && previewable(&crate::path::ext_of(&r.rel));
    stream(&r.full, range.as_deref(), &mime, inline, true).await
}

pub async fn api_media_head(State(st): State<AppState>, session: Session, AxPath(share_id): AxPath<i64>, Query(q): Query<TQ>) -> Response {
    let (r, mime) = match file_source(&st, &session, share_id, &q.path, q.t != 0).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let inline = q.dl == 0 && previewable(&crate::path::ext_of(&r.rel));
    stream(&r.full, None, &mime, inline, false).await
}

/// 视频封面:懒生成抽帧图(排除 avi,避免对其转码成本高的容器抽帧)。
pub async fn api_media_thumb(State(st): State<AppState>, session: Session, AxPath(share_id): AxPath<i64>, Query(q): Query<Q>) -> Response {
    let (r, rules, _) = match resolve(&st, share_id, &q.path).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = check_download(&session, &r.share, &rules, &r.rel) {
        return e.into_response();
    }
    if !r.full.is_file() {
        return err_json(StatusCode::NOT_FOUND, "文件不存在");
    }
    if crate::path::ext_of(&r.rel) == "avi" {
        return err_json(StatusCode::NOT_FOUND, "该格式不支持封面");
    }
    if !matches!(mime::kind_of(&crate::path::ext_of(&r.rel)), mime::MediaKind::Video) {
        return err_json(StatusCode::BAD_REQUEST, "仅视频支持封面");
    }
    let out = match crate::media::ensure_thumb(&st, r.share.id, &r.rel, &r.full).await {
        Ok(p) => p,
        Err(e) => return err_json(StatusCode::NOT_FOUND, e),
    };
    let bytes = match std::fs::read(&out) {
        Ok(b) => b,
        Err(_) => return err_json(StatusCode::NOT_FOUND, "封面不可用"),
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| err_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败"))
}