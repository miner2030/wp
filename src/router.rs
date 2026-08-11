use axum::extract::DefaultBodyLimit;
use axum::routing::{get, patch, post, put};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub const MAX_BODY: usize = 256 * 1024 * 1024;

pub fn build(state: AppState) -> Router {
    use crate::routes as r;
    Router::new()
        // ---- 认证 ----
        .route("/api/auth/login", post(r::auth::api_login))
        .route("/api/auth/logout", post(r::auth::api_logout))
        .route("/api/auth/me", get(r::auth::api_me))
        .route("/api/auth/password", post(r::auth::api_password))
        .route("/api/captcha", get(r::auth::api_captcha))
        // ---- 用户 / 共享 / 规则(管理) ----
        .route("/api/users", get(r::admin::api_users_list).post(r::admin::api_users_create))
        .route("/api/users/:id", patch(r::admin::api_user_patch).delete(r::admin::api_user_delete))
        .route("/api/shares", get(r::admin::api_shares_list).post(r::admin::api_shares_create))
        .route("/api/shares/:id", patch(r::admin::api_share_patch).delete(r::admin::api_share_delete))
        .route("/api/shares/:id/rules", get(r::admin::api_rules_list).post(r::admin::api_rule_add))
        .route("/api/shares/:id/rules/:rid", axum::routing::delete(r::admin::api_rule_delete))
        .route("/api/browse-host", get(r::admin::api_host_browse))
        .route("/api/dl/:share_id/:name", get(r::files::api_dl_get))
        .route("/dl/*path", get(r::files::api_dl_short))
        // ---- 文件操作 ----
        .route("/api/browse/:share_id", get(r::files::api_browse))
        .route("/api/file/meta/:share_id", get(r::files::api_file_meta))
        .route("/api/file/text/:share_id", get(r::doc::api_file_text_get).post(r::doc::api_file_text_put))
        .route("/api/file/binary/:share_id", post(r::doc::api_file_binary_put))
        .route("/api/file/dlticket", post(r::files::api_dlticket))
        .route("/api/mkdir", post(r::files::api_mkdir))
        .route("/api/rename", post(r::files::api_rename))
        .route("/api/delete", post(r::files::api_delete))
        .route("/api/zip", get(r::files::api_zip_get).post(r::files::api_zip))
        // ---- 上传 ----
        .route("/api/upload/init", post(r::upload::api_upload_init))
        .route("/api/uploads", get(r::upload::api_uploads_list))
        .route("/api/upload/:token", get(r::upload::api_upload_status).delete(r::upload::api_upload_abort))
        .route("/api/upload/parts/:token", get(r::upload::api_upload_parts))
        .route("/api/upload/part/:token/:part", put(r::upload::api_upload_part))
        .route("/api/upload/complete/:token", post(r::upload::api_upload_complete))
        // ---- 媒体 ----
        .route("/api/media/:share_id", get(r::media::api_media_get).head(r::media::api_media_head))
        .route("/api/media/status/:share_id", get(r::media::api_media_status))
        .route("/api/media/thumb/:share_id", get(r::media::api_media_thumb))
        // ---- 分享链接 ----
        .route("/api/fileshares", get(r::share::api_fileshare_list).post(r::share::api_fileshare_create))
        .route("/api/fileshares/:token", axum::routing::delete(r::share::api_fileshare_delete))
        // ---- HTTPS 证书(管理) ----
        .route("/api/tls", get(r::tls::api_tls_get).put(r::tls::api_tls_put).delete(r::tls::api_tls_delete))
        .route("/s/:token", get(r::share::api_share_link_page))
        .route("/s/:token/api/download", get(r::share::api_share_link_download))
        // ---- 页面与配置 ----
        .route("/api/config", get(r::web::api_config))
        .route("/", get(r::web::index))
        .route("/static/*path", get(r::web::static_file))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}