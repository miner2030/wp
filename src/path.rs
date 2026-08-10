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

/// 规范化用于展示/入库的绝对路径:Windows 下 canonicalize 会返回
/// `\\?\` 前缀的 verbatim 路径,这里转成常规形式;其他平台原样返回。
pub fn display_path(p: &Path) -> String {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
        s.to_string()
    }
    #[cfg(not(windows))]
    {
        p.to_string_lossy().to_string()
    }
}

/// 判断是否为盘符根目录(如 `C:\`),用于"上一级"返回盘符列表。
#[cfg(windows)]
pub fn is_drive_root(p: &Path) -> bool {
    use std::path::Component;
    let mut comps = p.components();
    matches!(
        (comps.next(), comps.next(), comps.next()),
        (Some(Component::Prefix(_)), Some(Component::RootDir), None)
    )
}

#[cfg(not(windows))]
pub fn is_drive_root(_p: &Path) -> bool {
    false
}

/// 列出 Windows 上可用的盘符(如 `C:`、`D:`);其他平台返回空列表。
#[cfg(windows)]
pub fn list_drives() -> Vec<String> {
    let mut out = Vec::new();
    for b in b'A'..=b'Z' {
        let letter = (b as char).to_string();
        if PathBuf::from(format!(r"{letter}:\")).is_dir() {
            out.push(format!("{letter}:"));
        }
    }
    out
}

#[cfg(not(windows))]
pub fn list_drives() -> Vec<String> {
    Vec::new()
}

/// 把盘符输入规范化为根路径:`C:` / `C:/` -> `C:\`;其他输入原样返回。
/// 盘符根不经过 canonicalize(其返回的 verbatim 路径对 read_dir 等并无必要,
/// 且部分环境下对盘符根调用 canonicalize 会报错),故单独规范。
#[cfg(windows)]
pub fn drive_of(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() == 2 && bytes[1] == b':' {
        return format!("{raw}\\");
    }
    if bytes.len() == 3 && bytes[1] == b':' && (bytes[2] == b'/' || bytes[2] == b'\\') {
        return format!("{}\\", &raw[..2]);
    }
    raw.to_string()
}

#[cfg(not(windows))]
pub fn drive_of(raw: &str) -> String {
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn drive_of_windows() {
        assert_eq!(drive_of("C:"), r"C:\");
        assert_eq!(drive_of(r"C:\"), r"C:\");
        assert_eq!(drive_of("C:/"), r"C:\");
        assert_eq!(drive_of(r"C:\Users"), r"C:\Users");
        assert_eq!(drive_of(r"D:\x"), r"D:\x");
        assert_eq!(drive_of(""), "");
        assert_eq!(drive_of("/"), "/");
        assert_eq!(drive_of(r"\\server\share"), r"\\server\share");
        for d in list_drives() {
            assert!(d.ends_with(':'), "盘符条目应为 X: 形式,实际 {d}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn drive_of_identity() {
        assert_eq!(drive_of("C:"), "C:");
        assert_eq!(drive_of(r"C:\"), r"C:\");
        assert_eq!(drive_of("C:/"), "C:/");
    }

    #[test]
    fn display_path_normal() {
        assert_eq!(display_path(Path::new("/tmp/x")), "/tmp/x");
    }
}