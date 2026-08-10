use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::db::TranscodeRow;
use crate::state::AppState;

/// 封面抽帧目标宽:小图足够网格缩略,保持等比。
const THUMB_WIDTH: u32 = 320;
/// 抽帧失败后的重试窗口(秒),期间不再重复启动 ffmpeg。
const THUMB_FAIL_TTL: i64 = 300;

/// 保证 (share, rel) 有转码记录且任务已启动;已在转码或转码完成则直接返回。
pub async fn ensure(st: &AppState, share_id: i64, rel: &str, input: &Path) -> Result<(), String> {
    let hash = hash_key(share_id, rel);
    let out_path = st.media_dir.join(&hash).join("f.mp4");
    if let Some(tc) = st.db.transcode(share_id, rel).await.map_err(|e| e.to_string())? {
        if tc.status == "running" {
            return Ok(());
        }
        if tc.status == "done" && Path::new(&tc.out_path).exists() {
            return Ok(());
        }
    }
    let tc = TranscodeRow {
        share_id,
        rel_path: rel.to_string(),
        hash,
        status: "running".into(),
        out_path: out_path.to_string_lossy().to_string(),
        updated_at: crate::now(),
    };
    st.db.transcode_start(&tc).await.map_err(|e| e.to_string())?;
    spawn_transcode(st.clone(), share_id, rel.to_string(), input.to_path_buf(), tc.hash.clone());
    Ok(())
}

/// 封面(jpg)产物路径;文件不存在即未生成。
pub fn thumb_path(st: &AppState, share_id: i64, rel: &str) -> PathBuf {
    st.media_dir.join(hash_key(share_id, rel)).join("thumb.jpg")
}

/// 懒生成视频封面:已存在直接返回;否则用 ffmpeg 抽第 1 秒一帧存为 jpg。
/// 并发请求由 `thumb_pending` 去重,同 key 只会有一路真正抽帧;
/// 失败在 `thumb_failed` 记录,窗口内直接返回失败避免反复启动 ffmpeg。
pub async fn ensure_thumb(st: &AppState, share_id: i64, rel: &str, input: &Path) -> Result<PathBuf, String> {
    let out = thumb_path(st, share_id, rel);
    if out.exists() {
        return Ok(out);
    }
    let key = (share_id, rel.to_string());
    {
        let mut failed = st.thumb_failed.lock().unwrap();
        if let Some(&t) = failed.get(&key) {
            if crate::now() - t < THUMB_FAIL_TTL {
                return Err("封面生成失败,请稍后重试".into());
            }
            failed.remove(&key);
        }
    }
    {
        let mut pending = st.thumb_pending.lock().unwrap();
        if pending.contains(&key) {
            return Err("封面正在生成,请稍后重试".into());
        }
        pending.insert(key.clone());
    }
    let res = gen_thumb(st, input, &out).await;
    st.thumb_pending.lock().unwrap().remove(&key);
    if let Err(e) = &res {
        tracing::warn!("封面抽帧失败 ({share_id},{rel}): {e}");
        st.thumb_failed.lock().unwrap().insert(key, crate::now());
    }
    res?;
    Ok(out)
}

async fn gen_thumb(st: &AppState, input: &Path, out: &Path) -> Result<(), String> {
    let Some(ffmpeg) = &st.ffmpeg else {
        return Err("未找到 ffmpeg,无法生成封面".into());
    };
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut last_err = String::new();
    for seek in ["1", "0"] {
        let status = tokio::process::Command::new(ffmpeg)
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-ss", seek, "-i", &input.to_string_lossy(),
                "-frames:v", "1", "-q:v", "4",
                "-vf", &format!("scale='min({THUMB_WIDTH},iw)':-2"),
                "-pix_fmt", "yuvj420p",
                &out.to_string_lossy(),
            ])
            .output()
            .await
            .map_err(|e| format!("ffmpeg 启动失败: {e}"))?;
        if status.status.success() && out.exists() {
            return Ok(());
        }
        let _ = std::fs::remove_file(out);
        last_err = String::from_utf8_lossy(&status.stderr).to_string();
    }
    Err(format!("封面抽帧失败: {last_err}"))
}

/// 删除源文件时联动清理:删掉转码记录、落盘产物与封面图。
pub async fn purge(st: &AppState, share_id: i64, rel: &str) {
    if let Ok(Some(out)) = st.db.transcode_take_out(share_id, rel).await {
        let _ = std::fs::remove_file(&out);
    }
    let _ = std::fs::remove_file(thumb_path(st, share_id, rel));
}

fn hash_key(share_id: i64, rel_path: &str) -> String {
    let mut h = Sha256::new();
    h.update(share_id.to_string().as_bytes());
    h.update(b"|");
    h.update(rel_path.as_bytes());
    let out = h.finalize();
    out.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn spawn_transcode(state: AppState, share_id: i64, rel_path: String, input: PathBuf, hash: String) {
    tokio::spawn(async move {
        let out_dir = state.media_dir.join(&hash);
        let out_file = out_dir.join("f.mp4");
        let mut ok = false;
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            tracing::warn!("转码目录创建失败: {e}");
        } else if let Some(ffmpeg) = state.ffmpeg {
            let status = tokio::process::Command::new(&ffmpeg)
                .args([
                    "-hide_banner", "-loglevel", "error", "-y", "-i", &input.to_string_lossy(),
                    "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p",
                    "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart",
                    "-max_muxing_queue_size", "2048",
                    &out_file.to_string_lossy(),
                ])
                .output()
                .await;
            ok = matches!(&status, Ok(out) if out.status.success()) && out_file.exists();
            if !ok {
                tracing::warn!(
                    "ffmpeg 转码失败: {}",
                    status.map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_else(|e| e.to_string())
                );
            }
        } else {
            tracing::warn!("未找到 ffmpeg,无法转码");
        }
        let _ = state.db.transcode_finish(share_id, &rel_path, ok).await;
    });
}