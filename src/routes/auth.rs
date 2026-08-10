use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::json;

use crate::auth::{self, verify_password, Session};
use crate::error::{err_json, ok_json, ApiError, ApiResult};
use crate::state::{AppState, SESSION_TTL};

#[derive(serde::Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub captcha_id: Option<String>,
    #[serde(default)]
    pub captcha: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct PasswordReq {
    #[serde(default)]
    pub old_password: Option<String>,
    pub password: String,
}

pub async fn api_captcha(State(st): State<AppState>) -> Json<serde_json::Value> {
    let (id, code) = st.issue_captcha();
    Json(json!({ "id": id, "svg": captcha_svg(&code) }))
}

/// 渲染 4 字符验证码 SVG:随机位置/旋转/颜色 + 干扰线。
fn captcha_svg(code: &str) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let palette = ["#4e6ef2", "#e5484d", "#1fa66e", "#f59e0b", "#7c6cf0", "#12a5b0"];
    let mut svg = String::from("<svg xmlns='http://www.w3.org/2000/svg' width='132' height='42' viewBox='0 0 132 42'>");
    svg.push_str("<rect width='132' height='42' fill='#f2f5fc' rx='8'/>");
    for _ in 0..6 {
        let x1 = rng.gen_range(0..132);
        let y1 = rng.gen_range(0..42);
        let x2 = rng.gen_range(0..132);
        let y2 = rng.gen_range(0..42);
        let c = palette[rng.gen_range(0..palette.len())];
        svg.push_str(&format!(
            "<line x1='{x1}' y1='{y1}' x2='{x2}' y2='{y2}' stroke='{c}' stroke-opacity='0.28' stroke-width='1'/>"
        ));
    }
    for (i, ch) in code.chars().enumerate() {
        let x = 17 + (i as i32) * 28 + rng.gen_range(-3..4);
        let y = 27 + rng.gen_range(-4..5);
        let rot = rng.gen_range(-22..23);
        let c = palette[rng.gen_range(0..palette.len())];
        svg.push_str(&format!(
            "<text x='{x}' y='{y}' fill='{c}' font-size='21' font-family='Consolas,Menlo,monospace' font-weight='bold' transform='rotate({rot} {x} {y})'>{ch}</text>"
        ));
    }
    svg.push_str("</svg>");
    svg
}

#[axum::debug_handler]
pub async fn api_login(State(st): State<AppState>, Json(req): Json<LoginReq>) -> Response {
    let cap_ok = match (&req.captcha_id, &req.captcha) {
        (Some(id), Some(code)) => st.verify_captcha(id, code),
        _ => false,
    };
    if !cap_ok {
        return err_json(StatusCode::FORBIDDEN, "验证码错误或已过期,请重试");
    }
    match st.db.user_by_name(&req.username).await.ok().flatten() {
        Some(user) if verify_password(&req.password, &user.password_hash) => {
            let token = auth::gen_token();
            if st.db.create_session(user.id, &token, SESSION_TTL).await.is_ok() {
                ok_json(json!({ "token": token, "user": user.public() }))
            } else {
                err_json(StatusCode::INTERNAL_SERVER_ERROR, "登录失败")
            }
        }
        _ => err_json(StatusCode::UNAUTHORIZED, "用户名或密码错误"),
    }
}

pub async fn api_logout(State(st): State<AppState>, session: Session) -> Response {
    if let Some(t) = session.token {
        let _ = st.db.delete_session(&t).await;
    }
    ok_json(json!({ "ok": true }))
}

pub async fn api_me(State(_st): State<AppState>, session: Session) -> Response {
    match session.user {
        Some(u) => ok_json(json!(u.public())),
        None => err_json(StatusCode::UNAUTHORIZED, "not logged in"),
    }
}

pub async fn api_password(State(st): State<AppState>, session: Session, Json(req): Json<PasswordReq>) -> ApiResult<Response> {
    let u = crate::authz::require_login(&session)?.clone();
    if let Some(old) = &req.old_password {
        if !verify_password(old, &u.password_hash) {
            return Err(ApiError::bad_request("原密码错误"));
        }
    }
    let hash = auth::hash_password(&req.password)?;
    st.db.set_password_hash(u.id, &hash).await?;
    st.db.delete_other_sessions(u.id, session.token.as_deref().unwrap_or("")).await?;
    Ok(ok_json(json!({ "ok": true })))
}