#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Other,
}

pub fn kind_of(ext: &str) -> MediaKind {
    const IMAGE: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "avif", "tif", "tiff", "ico", "jfif", "heic"];
    const VIDEO: &[&str] = &[
        "mp4", "webm", "ogv", "mov", "m4v", "avi", "mkv", "flv", "wmv", "mpg", "mpeg", "ts", "3gp", "3g2", "ogm", "m2ts", "mts", "vob", "asf", "rmvb", "f4v",
    ];
    const AUDIO: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "wma", "amr"];
    if IMAGE.contains(&ext) {
        MediaKind::Image
    } else if VIDEO.contains(&ext) {
        MediaKind::Video
    } else if AUDIO.contains(&ext) {
        MediaKind::Audio
    } else {
        MediaKind::Other
    }
}

pub fn kind_label(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Other => "other",
    }
}

/// 浏览器原生可直接播放的视频容器。
pub fn native_video(ext: &str) -> bool {
    matches!(ext, "mp4" | "m4v" | "webm" | "ogv" | "mov")
}

/// 可作为纯文本在线查看/编辑的类型。
pub fn editable_text(ext: &str) -> bool {
    matches!(
        ext,
        "txt" | "log" | "md" | "markdown" | "yaml" | "yml" | "conf" | "ini" | "toml" | "csv" | "tsv"
            | "json" | "xml" | "html" | "htm" | "css" | "js" | "mjs" | "ts" | "sh" | "bash" | "py"
            | "sql" | "properties" | "env" | "gitignore" | "gitconfig" | "dockerfile" | "makefile"
    )
}

/// 表格类(可用 SheetJS 在线读写)。
pub fn office_sheet(ext: &str) -> bool {
    matches!(ext, "xlsx" | "xlsm" | "xls")
}

/// Word 新格式(可在线渲染预览)。
pub fn office_doc(ext: &str) -> bool {
    matches!(ext, "docx" | "docm")
}

/// 支持在线预览的扩展名(媒体 + 文档)。
pub fn previewable(ext: &str) -> bool {
    !matches!(kind_of(ext), MediaKind::Other)
        || editable_text(ext)
        || office_sheet(ext)
        || office_doc(ext)
        || matches!(ext, "pdf" | "doc" | "xlsm" | "odt" | "ods" | "odp" | "rtf")
}

pub fn mime_of(ext: &str) -> String {
    match ext {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "flv" => "video/x-flv",
        "wmv" => "video/x-ms-wmv",
        "mpg" | "mpeg" => "video/mpeg",
        "ts" | "m2ts" => "video/mp2t",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "opus" => "audio/opus",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" | "yaml" | "yml" | "conf" | "ini" | "toml" | "csv" => "text/plain; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "json" => "application/json",
        "xml" => "application/xml",
        "apk" => "application/vnd.android.package-archive",
        "exe" => "application/x-msdownload",
        "iso" => "application/x-iso9660-image",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// 解析 HTTP Range 头,返回 (start, end) 闭区间;多段范围不支持。
pub fn parse_range(range: &str, file_size: u64) -> Option<(u64, u64)> {
    let r = range.trim();
    let spec = r.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    if start.is_empty() {
        let n: u64 = end.parse().ok()?;
        if n == 0 {
            return None;
        }
        return Some((file_size.saturating_sub(n), file_size.saturating_sub(1)));
    }
    let s: u64 = start.parse().ok()?;
    if s >= file_size {
        return None;
    }
    let e: u64 = if end.is_empty() { file_size.saturating_sub(1) } else { end.parse().ok()? };
    Some((s, e.min(file_size.saturating_sub(1))))
}