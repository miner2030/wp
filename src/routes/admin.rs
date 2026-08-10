use std::path::PathBuf;

use axum::extract::{Path as AxPath, Query, State};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::auth::{Session, UserFlags};
use crate::authz::{access_of, private, require_admin, Access};
use crate::error::{ok_json, ApiError, ApiResult};
use crate::state::AppState;

#[derive(serde::Deserialize)]
pub struct NewUserReq {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub permit: Option<UserFlags>,
}

#[derive(serde::Deserialize)]
pub struct PatchUserReq {
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub is_admin: Option<bool>,
    #[serde(default)]
    pub permit: Option<UserFlags>,
}

#[derive(serde::Deserialize)]
pub struct NewShareReq {
    pub name: String,
    pub host_path: String,
}

#[derive(serde::Deserialize)]
pub struct PatchShareReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub host_path: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct RuleReq {
    #[serde(default)]
    pub rel_path: String,
    pub access: String,
}

#[derive(serde::Deserialize)]
pub struct HostBrowseQ {
    #[serde(default)]
    pub path: String,
    /// 为 true 时同时列出证书相关文件(.crt/.cer/.pem/.key),供 HTTPS 证书选择使用。
    #[serde(default, deserialize_with = "de_boolish")]
    pub files: Option<bool>,
}

/// 把 "1"/"true"/"yes"(大小写不敏感)接受为 true,其余视为 false。
fn de_boolish<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let s = String::deserialize(d)?;
    let v = matches!(s.trim().to_lowercase().as_str(), "1" | "true" | "yes");
    Ok(Some(v))
}

const CERT_FILE_EXTS: [&str; 4] = ["crt", "cer", "pem", "key"];

// ---------------- users ----------------

pub async fn api_users_list(State(st): State<AppState>, session: Session) -> ApiResult<Response> {
    require_admin(&session)?;
    let users = st.db.list_users().await?;
    Ok(ok_json(json!(users.iter().map(|u| u.public()).collect::<Vec<_>>())))
}

pub async fn api_users_create(State(st): State<AppState>, session: Session, Json(req): Json<NewUserReq>) -> ApiResult<Response> {
    require_admin(&session)?;
    let name = crate::path::sanitize_name(&req.username).map_err(|_| ApiError::bad_request("非法用户名"))?;
    if st.db.user_by_name(&name).await?.is_some() {
        return Err(ApiError::conflict("用户名已存在"));
    }
    let hash = crate::auth::hash_password(&req.password)?;
    let flags = req.permit.unwrap_or_default();
    let id = st.db.create_user(&name, &hash, req.is_admin, flags).await?;
    if !req.is_admin {
        let dir = st.users_dir.join(&name);
        std::fs::create_dir_all(&dir)?;
        let _ = st
            .db
            .create_share(&format!("{name} 的个人空间"), &dir.to_string_lossy(), Some(id), "home")
            .await;
    }
    Ok(ok_json(json!({ "id": id, "ok": true })))
}

pub async fn api_user_patch(State(st): State<AppState>, session: Session, AxPath(id): AxPath<i64>, Json(req): Json<PatchUserReq>) -> ApiResult<Response> {
    require_admin(&session)?;
    let target = st.db.user_by_id(id).await?.ok_or_else(|| ApiError::not_found("用户不存在"))?;
    if let Some(pw) = &req.password {
        let hash = crate::auth::hash_password(pw)?;
        st.db.set_password_hash(target.id, &hash).await?;
    }
    let new_admin = req.is_admin.unwrap_or(target.is_admin);
    let new_flags = req.permit.unwrap_or_else(|| target.flags());
    st.db.update_user_flags(target.id, new_admin, new_flags).await?;
    Ok(ok_json(json!({ "ok": true })))
}

pub async fn api_user_delete(State(st): State<AppState>, session: Session, AxPath(id): AxPath<i64>) -> ApiResult<Response> {
    let me = require_admin(&session)?.clone();
    if me.id == id {
        return Err(ApiError::bad_request("不能删除自己"));
    }
    let target = st.db.user_by_id(id).await?.ok_or_else(|| ApiError::not_found("用户不存在"))?;
    if target.is_admin && st.db.list_users().await?.iter().filter(|u| u.is_admin).count() <= 1 {
        return Err(ApiError::bad_request("最后一个管理员不能删除"));
    }
    let home_root = st.users_dir.canonicalize().unwrap_or_else(|_| st.users_dir.clone());
    for s in st.db.all_shares().await? {
        if s.kind == "home" && s.owner_id == Some(id) {
            let _ = st.db.delete_share(s.id).await;
            if let Ok(p) = PathBuf::from(&s.host_path).canonicalize() {
                if p.starts_with(&home_root) && p.is_dir() {
                    let _ = std::fs::remove_dir_all(&p);
                }
            }
        }
    }
    st.db.delete_user(id).await?;
    Ok(ok_json(json!({ "ok": true })))
}

// ---------------- shares ----------------

fn path_ok(dir: &PathBuf) -> Result<PathBuf, ApiError> {
    if dir.is_file() {
        return Err(ApiError::bad_request("路径是文件而非目录"));
    }
    std::fs::create_dir_all(dir).map_err(|e| ApiError::bad_request(format!("无法创建目录: {e}")))?;
    if let Ok(c) = dir.canonicalize() {
        return Ok(c);
    }
    if dir.is_dir() {
        return Ok(dir.clone());
    }
    Err(ApiError::bad_request("路径无效"))
}

pub async fn api_shares_list(State(st): State<AppState>, session: Session) -> ApiResult<Response> {
    let mut out = Vec::new();
    for s in st.db.all_shares().await? {
        let rules = st.db.rules(s.id).await?;
        let visible = if private(&s) {
            match session.user() {
                Some(u) => crate::authz::is_owner(&s, u),
                None => false,
            }
        } else {
            match access_of(&rules, "") {
                Access::Guest => true,
                Access::Login => session.user().is_some(),
                Access::Admin => session.is_admin(),
            }
        };
        if !visible {
            continue;
        }
        out.push(json!({
            "id": s.id,
            "name": s.name,
            "kind": s.kind,
            "owner_id": s.owner_id,
            "host_path": if session.is_admin() { s.host_path.clone() } else { String::new() },
            "owner": session.user().map(|u| crate::authz::is_owner(&s, u)).unwrap_or(false),
            "access": if private(&s) { "private" } else { access_of(&rules, "").as_str() },
            "home": s.kind == "home",
            "created_at": s.created_at,
        }));
    }
    Ok(ok_json(json!({ "shares": out })))
}

pub async fn api_shares_create(State(st): State<AppState>, session: Session, Json(req): Json<NewShareReq>) -> ApiResult<Response> {
    require_admin(&session)?;
    let canonical = path_ok(&PathBuf::from(&req.host_path))?;
    let id = st
        .db
        .create_share(&req.name, &crate::path::display_path(&canonical), None, "custom")
        .await?;
    let _ = st.db.insert_rule(id, "", Access::Login).await;
    Ok(ok_json(json!({ "id": id, "ok": true })))
}

pub async fn api_share_patch(State(st): State<AppState>, session: Session, AxPath(id): AxPath<i64>, Json(req): Json<PatchShareReq>) -> ApiResult<Response> {
    require_admin(&session)?;
    let share = st.db.share(id).await?.ok_or_else(|| ApiError::not_found("共享不存在"))?;
    let name = req.name.unwrap_or(share.name);
    let host_path = match req.host_path {
        Some(h) => crate::path::display_path(&path_ok(&PathBuf::from(&h))?),
        None => share.host_path,
    };
    st.db.update_share(id, &name, &host_path).await?;
    Ok(ok_json(json!({ "ok": true })))
}

pub async fn api_share_delete(State(st): State<AppState>, session: Session, AxPath(id): AxPath<i64>) -> ApiResult<Response> {
    require_admin(&session)?;
    st.db.delete_share(id).await?;
    Ok(ok_json(json!({ "ok": true })))
}

// ---------------- rules ----------------

pub async fn api_rules_list(State(st): State<AppState>, session: Session, AxPath(id): AxPath<i64>) -> ApiResult<Response> {
    require_admin(&session)?;
    let out = st
        .db
        .rules(id)
        .await?
        .iter()
        .map(|r| json!({ "id": r.id, "rel_path": r.rel_path, "access": r.access.as_str() }))
        .collect::<Vec<_>>();
    Ok(ok_json(json!({ "rules": out })))
}

pub async fn api_rule_add(State(st): State<AppState>, session: Session, AxPath(id): AxPath<i64>, Json(req): Json<RuleReq>) -> ApiResult<Response> {
    require_admin(&session)?;
    let access = Access::from_str(&req.access).ok_or_else(|| ApiError::bad_request("access 必须是 guest/login/admin"))?;
    let rule_id = st.db.insert_rule(id, &req.rel_path, access).await.map_err(|e| ApiError::bad_request(e))?;
    Ok(ok_json(json!({ "id": rule_id, "ok": true })))
}

pub async fn api_rule_delete(State(st): State<AppState>, session: Session, AxPath((_id, rid)): AxPath<(i64, i64)>) -> ApiResult<Response> {
    require_admin(&session)?;
    st.db.delete_rule(rid).await?;
    Ok(ok_json(json!({ "ok": true })))
}

// ---------------- host browse ----------------

pub async fn api_host_browse(State(_st): State<AppState>, session: Session, Query(q): Query<HostBrowseQ>) -> ApiResult<Response> {
    require_admin(&session)?;
    let raw = q.path.trim();
    let is_root = raw.is_empty() || raw == "/" || raw == "\\";
    if is_root && cfg!(windows) {
        return Ok(ok_json(json!({
            "path": "/",
            "parent": null,
            "dirs": crate::path::list_drives(),
            "files": [],
        })));
    }
    let full = PathBuf::from(if is_root {
        "/".to_string()
    } else {
        crate::path::drive_of(&raw)
    });
    let canon = match full.canonicalize() {
        Ok(c) => c,
        Err(_e) if full.is_absolute() && full.is_dir() => full,
        Err(e) => return Err(ApiError::bad_request(format!("路径不存在: {e}"))),
    };
    if !canon.is_dir() {
        return Err(ApiError::bad_request("不是目录"));
    }
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    if let Ok(read) = std::fs::read_dir(&canon) {
        for de in read.flatten() {
            let name = de.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if de.path().is_dir() {
                dirs.push(name);
            } else if q.files.unwrap_or(false)
                && de.path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| CERT_FILE_EXTS.contains(&e.to_lowercase().as_str()))
                    .unwrap_or(false)
            {
                files.push(name);
            }
        }
    }
    dirs.sort_by_key(|d| d.to_lowercase());
    files.sort_by_key(|f| f.to_lowercase());
    let parent = if crate::path::is_drive_root(&canon) {
        Some("/".to_string())
    } else {
        canon.parent().map(crate::path::display_path)
    };
    Ok(ok_json(json!({
        "path": crate::path::display_path(&canon),
        "parent": parent,
        "dirs": dirs,
        "files": files,
    })))
}