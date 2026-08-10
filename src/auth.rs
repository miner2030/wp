use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::response::Response;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use rusqlite::Row;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub is_admin: bool,
    pub can_upload: bool,
    pub can_download: bool,
    pub can_delete: bool,
    pub can_mkdir: bool,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserFlags {
    pub can_upload: bool,
    pub can_download: bool,
    pub can_delete: bool,
    pub can_mkdir: bool,
}

#[derive(Serialize)]
pub struct UserPublic {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
    pub can_upload: bool,
    pub can_download: bool,
    pub can_delete: bool,
    pub can_mkdir: bool,
    pub created_at: i64,
}

impl User {
    pub fn from_row(row: &Row) -> rusqlite::Result<User> {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            password_hash: row.get(2)?,
            is_admin: row.get::<_, i64>(3)? != 0,
            can_upload: row.get::<_, i64>(4)? != 0,
            can_download: row.get::<_, i64>(5)? != 0,
            can_delete: row.get::<_, i64>(6)? != 0,
            can_mkdir: row.get::<_, i64>(7)? != 0,
            created_at: row.get(8)?,
        })
    }

    pub fn flags(&self) -> UserFlags {
        UserFlags {
            can_upload: self.can_upload,
            can_download: self.can_download,
            can_delete: self.can_delete,
            can_mkdir: self.can_mkdir,
        }
    }

    pub fn public(&self) -> UserPublic {
        UserPublic {
            id: self.id,
            username: self.username.clone(),
            is_admin: self.is_admin,
            can_upload: self.can_upload,
            can_download: self.can_download,
            can_delete: self.can_delete,
            can_mkdir: self.can_mkdir,
            created_at: self.created_at,
        }
    }
}

pub fn hash_password(password: &str) -> Result<String, String> {
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    use argon2::Argon2;
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    use argon2::Argon2;
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// 32 字节随机令牌(会话 / 分享链接)。
pub fn gen_token() -> String {
    let mut buf = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// 12 字节短随机令牌(上传会话)。
pub fn gen_upload_token() -> String {
    let mut buf = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// 请求身份:`user == None` 表示未登录(访客)。
#[derive(Clone, Debug)]
pub struct Session {
    pub user: Option<User>,
    pub token: Option<String>,
}

impl Session {
    pub fn user(&self) -> Option<&User> {
        self.user.as_ref()
    }

    pub fn is_admin(&self) -> bool {
        self.user.as_ref().map(|u| u.is_admin).unwrap_or(false)
    }
}

#[async_trait::async_trait]
impl<S> FromRequestParts<S> for Session
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app: AppState = FromRef::from_ref(state);
        let header_token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.trim().to_string());
        let cookie_token = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|c| {
                c.split(';').find_map(|kv| {
                    let kv = kv.trim();
                    kv.strip_prefix("wp_token=").map(|s| s.trim().to_string())
                })
            });
        let token = header_token.or(cookie_token);
        match token {
            Some(t) if !t.is_empty() => {
                let user = app.db.user_by_token(&t).await.ok().flatten();
                Ok(Session { user, token: Some(t) })
            }
            _ => Ok(Session { user: None, token: None }),
        }
    }
}