use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rand::Rng;

use crate::auth::{gen_token, hash_password, UserFlags};
use crate::config::Config;
use crate::db::Db;

pub const SESSION_TTL: i64 = 48 * 3600;
pub const CAPTCHA_TTL: i64 = 600;
pub const UPLOAD_TTL: i64 = 48 * 3600;
pub const DB_PATH: &str = "data/app.sqlite3";

/// 应用运行期状态:数据库句柄 + 数据目录 + 运行时配置 + 验证码表。
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub uploads_dir: PathBuf,
    pub users_dir: PathBuf,
pub media_dir: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    pub chunk_size: i64,
    pub tls_listen: String,
    pub tls: Arc<crate::tls::TlsRuntime>,
    pub(crate) thumb_pending: Arc<Mutex<HashSet<(i64, String)>>>,
    /// 抽帧失败记录 (share, rel) -> 最近失败时刻,5 分钟内不重复尝试。
    pub(crate) thumb_failed: Arc<Mutex<HashMap<(i64, String), i64>>>,
    captchas: Arc<Mutex<HashMap<String, (String, i64)>>>,
}

fn which_ffmpeg() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("WP_FFMPEG") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let cand = std::path::Path::new(dir).join("ffmpeg");
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    None
}

impl AppState {
    pub async fn boot(cfg: &Config) -> Result<Self, String> {
        let base = cfg.data_dir.clone();
        let uploads_dir = base.join("uploads");
        let users_dir = base.join("users");
        let media_dir = base.join("media");
        for dir in [&base, &uploads_dir, &users_dir, &media_dir] {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }

        let db = Db::open(&base.join(DB_PATH)).await?;
        let st = AppState {
            db,
            uploads_dir,
            users_dir,
            media_dir,
            ffmpeg: which_ffmpeg(),
            chunk_size: if cfg.chunk_size > 0 { cfg.chunk_size } else { crate::config::DEFAULT_CHUNK },
            captchas: Arc::new(Mutex::new(HashMap::new())),
            thumb_pending: Arc::new(Mutex::new(HashSet::new())),
            thumb_failed: Arc::new(Mutex::new(HashMap::new())),
            tls_listen: cfg.tls_listen.clone(),
            tls: Arc::new(crate::tls::TlsRuntime::default()),
        };
        match &st.ffmpeg {
            Some(p) => tracing::info!("检测到 ffmpeg: {}", p.display()),
            None => tracing::warn!("未找到 ffmpeg:视频转码与封面抽帧不可用。请安装 ffmpeg 并加入系统 PATH,或设置环境变量 WP_FFMPEG 指向其路径"),
        }
        st.prune_uploads().await;
        st.bootstrap_admin(&cfg.admin_user, &cfg.admin_pass).await;
        tokio::spawn(crate::tls::run(st.clone()));
        Ok(st)
    }

    /// 统一清理由上传产生的陈旧数据:先删 DB 过期行,再删磁盘上无主的分片目录。
    async fn prune_uploads(&self) {
        let cutoff = crate::now() - UPLOAD_TTL;
        let _ = self.db.prune_uploads(cutoff).await;
        let live: HashSet<String> = match self.db.all_upload_tokens().await {
            Ok(t) => t.into_iter().collect(),
            Err(_) => return,
        };
        if let Ok(rd) = std::fs::read_dir(&self.uploads_dir) {
            for de in rd.flatten() {
                let name = de.file_name().to_string_lossy().to_string();
                let keep = live.contains(&name)
                    || (name.ends_with(".merge") && live.contains(name.trim_end_matches(".merge")));
                if !keep {
                    let _ = std::fs::remove_dir_all(de.path());
                }
            }
        }
    }

    /// 签发验证码,返回 (id, code)。code 仅出现在服务端渲染的 SVG 中。
    pub fn issue_captcha(&self) -> (String, String) {
        let mut rng = rand::thread_rng();
        let alpha: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
        let code: String = (0..4).map(|_| alpha[rng.gen_range(0..alpha.len())] as char).collect();
        let id = gen_token();
        let now = crate::now();
        let mut m = self.captchas.lock().unwrap();
        m.retain(|_, (_, t)| now - *t < CAPTCHA_TTL);
        m.insert(id.clone(), (code.clone(), now));
        (id, code)
    }

    /// 校验验证码:不区分大小写、一次性、10 分钟有效。
    pub fn verify_captcha(&self, id: &str, code: &str) -> bool {
        let mut m = self.captchas.lock().unwrap();
        match m.remove(id) {
            Some((want, t)) => crate::now() - t < CAPTCHA_TTL && want.eq_ignore_ascii_case(code.trim()),
            None => false,
        }
    }

    /// 无用户时创建初始管理员。
    async fn bootstrap_admin(&self, user: &str, pass: &str) {
        match self.db.list_users().await {
            Ok(users) if !users.is_empty() => return,
            Err(_) => return,
            _ => {}
        }
        match hash_password(pass) {
            Ok(hash) => {
                let flags = UserFlags { can_upload: true, can_download: true, can_delete: true, can_mkdir: true };
                let _ = self.db.create_user(user, &hash, true, flags).await;
                tracing::warn!("已创建初始管理员账号: {user} / {pass}(请尽快修改密码)");
            }
            Err(e) => tracing::error!("创建管理员失败: {e}"),
        }
    }
}