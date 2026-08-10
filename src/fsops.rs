use std::path::{Path, PathBuf};

use crate::db::{AccessRule, Share};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// 一次请求解析出的目标:共享 + 物理根目录 + 相对路径(已规范化)。
pub struct Located {
    pub share: Share,
    pub root: PathBuf,
    pub rel: String,
    pub full: PathBuf,
}

/// 解析共享并定位路径:共享不存在 -> 404,路径越界 -> 400。
pub async fn locate(st: &AppState, share_id: i64, rel: &str) -> ApiResult<Located> {
    let share = st
        .db
        .share(share_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("共享不存在"))?;
    let root = PathBuf::from(&share.host_path);
    if !root.is_dir() {
        return Err(ApiError::internal("共享根目录不可用"));
    }
    let root = root.canonicalize().map_err(|e| ApiError::internal(format!("共享根目录不可用: {e}")))?;
    let rel_n = crate::path::norm_rel(rel).map_err(ApiError::bad_request)?;
    let full = crate::path::safe_join(&root, &rel_n).map_err(ApiError::bad_request)?;
    Ok(Located { share, root, rel: rel_n, full })
}

/// 定位 + 规则 + 该路径访问级别:各读写路由共用的前置步骤。
pub async fn resolve(st: &AppState, share_id: i64, rel: &str) -> ApiResult<(Located, Vec<AccessRule>, crate::authz::Access)> {
    let located = locate(st, share_id, rel).await?;
    let rules = st.db.rules(located.share.id).await?;
    let access = crate::authz::access_of(&rules, &located.rel);
    Ok((located, rules, access))
}

/// 递归收集目录下所有文件(跳过隐藏项),`rel` 作归档内相对路径。
pub fn collect_dir(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for de in rd.flatten() {
        let name = de.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let child_rel = format!("{}/{}", rel, name);
        let child = de.path();
        if child.is_dir() {
            collect_dir(&child, &child_rel, out);
        } else if child.is_file() {
            out.push((child_rel, child));
        }
    }
}

/// 递归删除目录并统计删除的条目数(含目录本身)。
pub fn remove_dir_all_count(dir: &Path) -> usize {
    let mut n = 1usize;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for de in rd.flatten() {
            let p = de.path();
            if p.is_dir() {
                n += remove_dir_all_count(&p);
            } else {
                n += 1;
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    n
}