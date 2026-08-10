use std::io::SeekFrom;
use std::path::Path;

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use crate::error::err_json;

pub fn urlencode(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

pub fn disposition(inline: bool, name: &str) -> String {
    if inline {
        "inline".to_string()
    } else {
        format!("attachment; filename*=UTF-8''{}", utf8_percent_encode(name, NON_ALPHANUMERIC))
    }
}

/// 支持 Range 的文件流。`want_body=false` 时仅回响应头(HEAD)。
pub async fn stream(file: &Path, range: Option<&str>, mime: &str, inline: bool, want_body: bool) -> Response {
    let len = match tokio::fs::metadata(file).await {
        Ok(m) => m.len(),
        Err(_) => return err_json(StatusCode::NOT_FOUND, "文件不存在"),
    };
    let (start, end, status, content_range) = match range.and_then(|r| crate::mime::parse_range(r, len)) {
        Some((s, e)) => (s, e, StatusCode::PARTIAL_CONTENT, Some(format!("bytes {s}-{e}/{len}"))),
        None => {
            if range.is_some() {
                let mut resp = Response::new(Body::empty());
                *resp.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
                resp.headers_mut()
                    .insert("Content-Range", format!("bytes */{len}").parse().unwrap());
                return resp;
            }
            (0, len.saturating_sub(1), StatusCode::OK, None)
        }
    };
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut resp = Response::builder()
        .status(status)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_TYPE, mime.to_string())
        .header(header::CONTENT_DISPOSITION, disposition(inline, &name));
    if let Some(cr) = content_range {
        resp = resp.header("Content-Range", cr);
    }
    let content_length = if len == 0 { 0 } else { end - start + 1 };
    resp = resp.header(header::CONTENT_LENGTH, content_length.to_string());
    if !want_body || len == 0 {
        return resp.body(Body::empty()).unwrap();
    }
    let mut f = match tokio::fs::File::open(file).await {
        Ok(f) => f,
        Err(e) => return err_json(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")),
    };
    if start > 0 {
        let _ = f.seek(SeekFrom::Start(start)).await;
    }
    let body = Body::from_stream(ReaderStream::with_capacity(f.take(content_length), 64 * 1024));
    resp.body(body).unwrap()
}