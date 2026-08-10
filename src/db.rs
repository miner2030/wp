use rusqlite::{params, Connection, Row};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::auth::{User, UserFlags};

/// SQLite 访问层:单连接 + 互斥,所有 SQL 固定,运行时错误向调用方传播。
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

/// 行 -> 模型映射,供 `query_all` / `query_one` 复用。
type RowMapper<T> = fn(&Row) -> rusqlite::Result<T>;

impl Db {
    pub async fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).ok();
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();
        let db = Db { conn: Arc::new(Mutex::new(conn)) };
        db.migrate().await?;
        Ok(db)
    }

    async fn tx(&self) -> tokio::sync::MutexGuard<'_, Connection> {
        self.conn.lock().await
    }

    async fn query_all<T>(&self, sql: &str, params: impl rusqlite::Params + Send + Sync, map: RowMapper<T>) -> rusqlite::Result<Vec<T>> {
        let conn = self.tx().await;
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params, map)?;
        rows.collect()
    }

    async fn query_one<T>(&self, sql: &str, params: impl rusqlite::Params + Send + Sync, map: RowMapper<T>) -> rusqlite::Result<Option<T>> {
        Ok(self.query_all(sql, params, map).await?.into_iter().next())
    }

    async fn migrate(&self) -> Result<(), String> {
        let conn = self.tx().await;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                is_admin INTEGER NOT NULL DEFAULT 0,
                can_upload INTEGER NOT NULL DEFAULT 1,
                can_download INTEGER NOT NULL DEFAULT 1,
                can_delete INTEGER NOT NULL DEFAULT 0,
                can_mkdir INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS shares (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                host_path TEXT NOT NULL,
                owner_id INTEGER,
                kind TEXT NOT NULL DEFAULT 'custom',
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS access_rules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                share_id INTEGER NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
                rel_path TEXT NOT NULL DEFAULT '',
                access TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(share_id, rel_path)
            );
            CREATE TABLE IF NOT EXISTS uploads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                token TEXT NOT NULL,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                share_id INTEGER NOT NULL,
                rel_path TEXT NOT NULL,
                filename TEXT NOT NULL,
                total_size INTEGER NOT NULL,
                chunk_size INTEGER NOT NULL,
                num_parts INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'uploading',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS transcodes (
                share_id INTEGER NOT NULL,
                rel_path TEXT NOT NULL,
                hash TEXT NOT NULL,
                status TEXT NOT NULL,
                out_path TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (share_id, rel_path)
            );
            CREATE TABLE IF NOT EXISTS file_shares (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                token TEXT NOT NULL UNIQUE,
                share_id INTEGER NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
                rel_path TEXT NOT NULL,
                created_by INTEGER,
                created_at INTEGER NOT NULL,
                hits INTEGER NOT NULL DEFAULT 0,
                UNIQUE(share_id, rel_path)
            );
            CREATE TABLE IF NOT EXISTS tls_certs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                domain TEXT NOT NULL,
                cert_pem TEXT NOT NULL,
                key_pem TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        drop(conn);
        Ok(())
    }

    // ---------------- users / sessions ----------------

    const USER_COLS: &'static str =
        "id,username,password_hash,is_admin,can_upload,can_download,can_delete,can_mkdir,created_at";

    pub async fn user_by_id(&self, id: i64) -> rusqlite::Result<Option<User>> {
        self.query_one(&format!("SELECT {} FROM users WHERE id=?1", Self::USER_COLS), (id,), User::from_row).await
    }

    pub async fn user_by_name(&self, name: &str) -> rusqlite::Result<Option<User>> {
        self.query_one(&format!("SELECT {} FROM users WHERE username=?1", Self::USER_COLS), (name,), User::from_row).await
    }

    /// 按会话令牌取用户,并清理已过期会话。
    pub async fn user_by_token(&self, token: &str) -> rusqlite::Result<Option<User>> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM sessions WHERE expires_at < ?1", params![crate::now()])?;
        let mut stmt = conn.prepare(&format!(
            "SELECT u.id,u.username,u.password_hash,u.is_admin,u.can_upload,u.can_download,u.can_delete,u.can_mkdir,u.created_at \
             FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token=?1 AND s.expires_at>=?2"
        ))?;
        let mut rows = stmt.query(params![token, crate::now()])?;
        match rows.next()? {
            Some(row) => Ok(Some(User::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn list_users(&self) -> rusqlite::Result<Vec<User>> {
        self.query_all(&format!("SELECT {} FROM users ORDER BY id", Self::USER_COLS), (), User::from_row).await
    }

    pub async fn create_user(&self, username: &str, hash: &str, is_admin: bool, flags: UserFlags) -> rusqlite::Result<i64> {
        let conn = self.tx().await;
        conn.execute(
            "INSERT INTO users(username,password_hash,is_admin,can_upload,can_download,can_delete,can_mkdir,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![username, hash, is_admin as i64, flags.can_upload as i64, flags.can_download as i64, flags.can_delete as i64, flags.can_mkdir as i64, crate::now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub async fn set_password_hash(&self, id: i64, hash: &str) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("UPDATE users SET password_hash=?2 WHERE id=?1", params![id, hash])?;
        Ok(())
    }

    pub async fn update_user_flags(&self, id: i64, is_admin: bool, flags: UserFlags) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute(
            "UPDATE users SET is_admin=?2,can_upload=?3,can_download=?4,can_delete=?5,can_mkdir=?6 WHERE id=?1",
            params![id, is_admin as i64, flags.can_upload as i64, flags.can_download as i64, flags.can_delete as i64, flags.can_mkdir as i64],
        )?;
        Ok(())
    }

    pub async fn delete_user(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM users WHERE id=?1", params![id])?;
        Ok(())
    }

    pub async fn create_session(&self, user_id: i64, token: &str, ttl_secs: i64) -> rusqlite::Result<()> {
        let now = crate::now();
        let conn = self.tx().await;
        conn.execute(
            "INSERT INTO sessions(user_id,token,created_at,expires_at) VALUES(?1,?2,?3,?4)",
            params![user_id, token, now, now + ttl_secs],
        )?;
        Ok(())
    }

    pub async fn delete_session(&self, token: &str) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM sessions WHERE token=?1", params![token])?;
        Ok(())
    }

    pub async fn delete_other_sessions(&self, user_id: i64, keep_token: &str) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM sessions WHERE user_id=?1 AND token<>?2", params![user_id, keep_token])?;
        Ok(())
    }

    // ---------------- shares / rules ----------------

    pub async fn create_share(&self, name: &str, host_path: &str, owner_id: Option<i64>, kind: &str) -> rusqlite::Result<i64> {
        let conn = self.tx().await;
        conn.execute(
            "INSERT INTO shares(name,host_path,owner_id,kind,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![name, host_path, owner_id, kind, crate::now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub async fn share(&self, id: i64) -> rusqlite::Result<Option<Share>> {
        self.query_one(
            "SELECT id,name,host_path,owner_id,kind,created_at FROM shares WHERE id=?1",
            (id,),
            Share::from_row,
        )
        .await
    }

    pub async fn all_shares(&self) -> rusqlite::Result<Vec<Share>> {
        self.query_all(
            "SELECT id,name,host_path,owner_id,kind,created_at FROM shares ORDER BY id",
            (),
            Share::from_row,
        )
        .await
    }

    pub async fn update_share(&self, id: i64, name: &str, host_path: &str) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("UPDATE shares SET name=?2,host_path=?3 WHERE id=?1", params![id, name, host_path])?;
        Ok(())
    }

    pub async fn delete_share(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM shares WHERE id=?1", params![id])?;
        Ok(())
    }

    pub async fn rules(&self, share_id: i64) -> rusqlite::Result<Vec<AccessRule>> {
        self.query_all(
            "SELECT id,share_id,rel_path,access,created_at FROM access_rules WHERE share_id=?1 ORDER BY length(rel_path) DESC, rel_path",
            (share_id,),
            AccessRule::from_row,
        )
        .await
    }

    pub async fn insert_rule(&self, share_id: i64, rel_path: &str, access: crate::authz::Access) -> Result<i64, String> {
        let np = crate::path::norm_rel(rel_path)?;
        let conn = self.tx().await;
        conn.execute(
            "INSERT INTO access_rules(share_id,rel_path,access,created_at) VALUES(?1,?2,?3,?4) \
             ON CONFLICT(share_id,rel_path) DO UPDATE SET access=?3",
            params![share_id, np, access.as_str(), crate::now()],
        )
        .map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT id FROM access_rules WHERE share_id=?1 AND rel_path=?2",
            params![share_id, np],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
    }

    pub async fn delete_rule(&self, rule_id: i64) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM access_rules WHERE id=?1", params![rule_id])?;
        Ok(())
    }

    // ---------------- uploads ----------------

    pub async fn create_upload(
        &self,
        token: &str,
        user_id: i64,
        share_id: i64,
        rel_path: &str,
        filename: &str,
        total_size: i64,
        chunk_size: i64,
        num_parts: i64,
    ) -> rusqlite::Result<i64> {
        let conn = self.tx().await;
        conn.execute(
            "INSERT INTO uploads(token,user_id,share_id,rel_path,filename,total_size,chunk_size,num_parts,status,created_at,updated_at) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'uploading',?9,?9)",
            params![token, user_id, share_id, rel_path, filename, total_size, chunk_size, num_parts, crate::now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub async fn upload_by_token(&self, token: &str) -> rusqlite::Result<Option<UploadSession>> {
        self.query_one(
            "SELECT id,token,user_id,share_id,rel_path,filename,total_size,chunk_size,num_parts,status,created_at,updated_at \
             FROM uploads WHERE token=?1",
            (token,),
            UploadSession::from_row,
        )
        .await
    }

    pub async fn uploads_of(&self, user_id: i64) -> rusqlite::Result<Vec<UploadSession>> {
        self.query_all(
            "SELECT id,token,user_id,share_id,rel_path,filename,total_size,chunk_size,num_parts,status,created_at,updated_at \
             FROM uploads WHERE user_id=?1",
            (user_id,),
            UploadSession::from_row,
        )
        .await
    }

    pub async fn delete_upload(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM uploads WHERE id=?1", params![id])?;
        Ok(())
    }

    pub async fn touch_upload(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("UPDATE uploads SET updated_at=?2 WHERE id=?1", params![id, crate::now()])?;
        Ok(())
    }

    /// 删除超过 `cutoff` 的陈旧记录,返回删除数量。
    pub async fn prune_uploads(&self, cutoff: i64) -> rusqlite::Result<usize> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM uploads WHERE updated_at < ?1", params![cutoff])
    }

    /// 当前 DB 中还在跟踪的上传令牌(用于清理磁盘孤儿目录)。
    pub async fn all_upload_tokens(&self) -> rusqlite::Result<Vec<String>> {
        self.query_all("SELECT token FROM uploads", (), |r| r.get(0)).await
    }

    // ---------------- transcodes ----------------

    pub async fn transcode(&self, share_id: i64, rel_path: &str) -> rusqlite::Result<Option<TranscodeRow>> {
        self.query_one(
            "SELECT share_id,rel_path,hash,status,out_path,updated_at FROM transcodes WHERE share_id=?1 AND rel_path=?2",
            (share_id, rel_path),
            TranscodeRow::from_row,
        )
        .await
    }

    pub async fn transcode_start(&self, tc: &TranscodeRow) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute(
            "INSERT OR REPLACE INTO transcodes(share_id,rel_path,hash,status,out_path,updated_at) VALUES(?1,?2,?3,'running',?4,?5)",
            params![tc.share_id, tc.rel_path, tc.hash, tc.out_path, crate::now()],
        )?;
        Ok(())
    }

    pub async fn transcode_finish(&self, share_id: i64, rel_path: &str, ok: bool) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute(
            "UPDATE transcodes SET status=?3, updated_at=?4 WHERE share_id=?1 AND rel_path=?2",
            params![share_id, rel_path, if ok { "done" } else { "failed" }, crate::now()],
        )?;
        Ok(())
    }

    /// 取出 (share, rel) 的转码产物路径并删除记录,用于删文件时联动清理。
    pub async fn transcode_take_out(&self, share_id: i64, rel_path: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.tx().await;
        let out = conn
            .query_row(
                "SELECT out_path FROM transcodes WHERE share_id=?1 AND rel_path=?2",
                params![share_id, rel_path],
                |r| r.get::<_, String>(0),
            )
            .ok();
        conn.execute("DELETE FROM transcodes WHERE share_id=?1 AND rel_path=?2", params![share_id, rel_path])?;
        Ok(out)
    }

    // ---------------- file share links ----------------

    pub async fn file_share_by_token(&self, token: &str) -> rusqlite::Result<Option<FileShare>> {
        self.query_one(
            "SELECT id,token,share_id,rel_path,created_by,created_at,hits FROM file_shares WHERE token=?1",
            (token,),
            FileShare::from_row,
        )
        .await
    }

    pub async fn file_share_by_path(&self, share_id: i64, rel_path: &str) -> rusqlite::Result<Option<FileShare>> {
        self.query_one(
            "SELECT id,token,share_id,rel_path,created_by,created_at,hits FROM file_shares WHERE share_id=?1 AND rel_path=?2",
            (share_id, rel_path),
            FileShare::from_row,
        )
        .await
    }

    pub async fn file_shares_of(&self, share_id: i64) -> rusqlite::Result<Vec<FileShare>> {
        self.query_all(
            "SELECT id,token,share_id,rel_path,created_by,created_at,hits FROM file_shares WHERE share_id=?1 ORDER BY id",
            (share_id,),
            FileShare::from_row,
        )
        .await
    }

    /// 为 (share, rel) 确保存在一条链接:已存在直接返回,否则新建。
    pub async fn file_share_ensure(&self, share_id: i64, rel_path: &str, created_by: Option<i64>) -> Result<FileShare, String> {
        let np = crate::path::norm_rel(rel_path)?;
        if let Some(existing) = self.file_share_by_path(share_id, &np).await.ok().flatten() {
            return Ok(existing);
        }
        for _ in 0..6 {
            let token = crate::auth::gen_token();
            let conn = self.tx().await;
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO file_shares(token,share_id,rel_path,created_by,created_at) VALUES(?1,?2,?3,?4,?5)",
                params![token, share_id, np, created_by, crate::now()],
            );
            drop(conn);
            match inserted {
                Ok(1) => {
                    let created = self.file_share_by_path(share_id, &np).await.ok().flatten();
                    if let Some(f) = created {
                        return Ok(f);
                    }
                }
                Ok(_) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("创建分享失败".into())
    }

    pub async fn delete_file_share(&self, token: &str) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM file_shares WHERE token=?1", params![token])?;
        Ok(())
    }

    pub async fn file_share_bump(&self, token: &str) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("UPDATE file_shares SET hits=hits+1 WHERE token=?1", params![token])?;
        Ok(())
    }

    // ---------------- tls certificates ----------------

    pub async fn tls_cert(&self) -> rusqlite::Result<Option<TlsCert>> {
        self.query_one("SELECT id,domain,cert_pem,key_pem,created_at,updated_at FROM tls_certs ORDER BY id LIMIT 1", (), TlsCert::from_row).await
    }

    /// 保存(替换)当前证书:已有证书则整体覆盖,否则新增一条。
    pub async fn tls_upsert(&self, domain: &str, cert_pem: &str, key_pem: &str) -> rusqlite::Result<i64> {
        let conn = self.tx().await;
        let now = crate::now();
        if let Some(id) = conn.query_row("SELECT id FROM tls_certs ORDER BY id LIMIT 1", [], |r| r.get::<_, i64>(0)).ok() {
            conn.execute(
                "UPDATE tls_certs SET domain=?2, cert_pem=?3, key_pem=?4, updated_at=?5 WHERE id=?1",
                params![id, domain, cert_pem, key_pem, now],
            )?;
            Ok(id)
        } else {
            conn.execute(
                "INSERT INTO tls_certs(domain,cert_pem,key_pem,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",
                params![domain, cert_pem, key_pem, now],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }

    pub async fn tls_delete(&self) -> rusqlite::Result<()> {
        let conn = self.tx().await;
        conn.execute("DELETE FROM tls_certs", [])?;
        Ok(())
    }
}

// ---------------- models ----------------

#[derive(Clone, Debug)]
pub struct Share {
    pub id: i64,
    pub name: String,
    pub host_path: String,
    pub owner_id: Option<i64>,
    pub kind: String,
    pub created_at: i64,
}

impl Share {
    fn from_row(row: &Row) -> rusqlite::Result<Share> {
        Ok(Share {
            id: row.get(0)?,
            name: row.get(1)?,
            host_path: row.get(2)?,
            owner_id: row.get(3)?,
            kind: row.get(4)?,
            created_at: row.get(5)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct AccessRule {
    pub id: i64,
    pub share_id: i64,
    pub rel_path: String,
    pub access: crate::authz::Access,
    pub created_at: i64,
}

impl AccessRule {
    fn from_row(row: &Row) -> rusqlite::Result<AccessRule> {
        Ok(AccessRule {
            id: row.get(0)?,
            share_id: row.get(1)?,
            rel_path: row.get(2)?,
            access: crate::authz::Access::from_str(&row.get::<_, String>(3)?).unwrap_or(crate::authz::Access::Admin),
            created_at: row.get(4)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct UploadSession {
    pub id: i64,
    pub token: String,
    pub user_id: i64,
    pub share_id: i64,
    pub rel_path: String,
    pub filename: String,
    pub total_size: i64,
    pub chunk_size: i64,
    pub num_parts: i64,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl UploadSession {
    fn from_row(row: &Row) -> rusqlite::Result<UploadSession> {
        Ok(UploadSession {
            id: row.get(0)?,
            token: row.get(1)?,
            user_id: row.get(2)?,
            share_id: row.get(3)?,
            rel_path: row.get(4)?,
            filename: row.get(5)?,
            total_size: row.get(6)?,
            chunk_size: row.get(7)?,
            num_parts: row.get(8)?,
            status: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TranscodeRow {
    pub share_id: i64,
    pub rel_path: String,
    pub hash: String,
    pub status: String,
    pub out_path: String,
    pub updated_at: i64,
}

impl TranscodeRow {
    fn from_row(row: &Row) -> rusqlite::Result<TranscodeRow> {
        Ok(TranscodeRow {
            share_id: row.get(0)?,
            rel_path: row.get(1)?,
            hash: row.get(2)?,
            status: row.get(3)?,
            out_path: row.get(4)?,
            updated_at: row.get(5)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct FileShare {
    pub id: i64,
    pub token: String,
    pub share_id: i64,
    pub rel_path: String,
    pub created_by: Option<i64>,
    pub created_at: i64,
    pub hits: i64,
}

impl FileShare {
    fn from_row(row: &Row) -> rusqlite::Result<FileShare> {
        Ok(FileShare {
            id: row.get(0)?,
            token: row.get(1)?,
            share_id: row.get(2)?,
            rel_path: row.get(3)?,
            created_by: row.get(4)?,
            created_at: row.get(5)?,
            hits: row.get(6)?,
        })
    }
}

/// 当前启用的 HTTPS 证书(数据库只保留一条,重复保存即覆盖)。
#[derive(Clone, Debug)]
pub struct TlsCert {
    pub id: i64,
    pub domain: String,
    pub cert_pem: String,
    pub key_pem: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl TlsCert {
    fn from_row(row: &Row) -> rusqlite::Result<TlsCert> {
        Ok(TlsCert {
            id: row.get(0)?,
            domain: row.get(1)?,
            cert_pem: row.get(2)?,
            key_pem: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    }
}