use axum::body::Body;
use axum::extract::{Path as AxPath, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use rust_embed::RustEmbed;
use serde_json::json;

use crate::error::{err_json, ok_json};
use crate::mime;
use crate::state::AppState;

const CACHE_CONTROL: &str = "no-store";

#[derive(RustEmbed)]
#[folder = "web/"]
struct Assets;

fn embed_file(path: &str) -> Response {
    match Assets::get(path) {
        Some(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime::mime_of(&crate::path::ext_of(path)))
            .header(header::CACHE_CONTROL, CACHE_CONTROL)
            .body(Body::from(data.data.into_owned()))
            .unwrap(),
        None => err_json(StatusCode::NOT_FOUND, "not found"),
    }
}

pub async fn api_config(State(st): State<AppState>) -> Response {
    ok_json(json!({
        "chunk_size": st.chunk_size,
        "ffmpeg": st.ffmpeg.is_some(),
    }))
}

pub async fn index() -> Response {
    embed_file("index.html")
}

pub async fn static_file(AxPath(path): AxPath<String>) -> Response {
    embed_file(&path)
}