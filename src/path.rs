use std::path::{Path, PathBuf};

/// 规范化相对路径:去重 `/`、`.`、首尾斜杠;出现 `..` 视为非法。
pub fn norm_rel(path: &str) -> Result<String, String> {
    let mut parts = Vec::new();
    for seg in path.split('/') {
        let seg = seg.trim();
        match seg {
            "" | "." => {}
            ".." => return Err("非法路径".into()),
            _ => parts.push(seg.to_string()),
        }
    }
    Ok(parts.join("/"))
}

/// 校验单个文件名(不含分隔符与非法字符)。
pub fn sanitize_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("非法文件名".into());
    }
    if name.contains(['/', '\\', '\0']) {
        return Err("文件名不能包含路径分隔符".into());
    }
    if name.contains([':', '*', '?', '"', '<', '>', '|']) {
        return Err("文件名包含非法字符".into());
    }
    Ok(name.to_string())
}

/// 在(已规范化的)根目录下拼接相对路径,保证结果不越出根目录。
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("根目录不可用: {e}"))?;
    let mut out = canonical_root.clone();
    let mut check = canonical_root.clone();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Err("路径越界".into()),
            _ => {
                out.push(seg);
                check.push(seg);
                if let Ok(c) = check.canonicalize() {
                    if !c.starts_with(&canonical_root) {
                        return Err("路径越界".into());
                    }
                    check = c;
                }
            }
        }
    }
    Ok(out)
}

pub fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}