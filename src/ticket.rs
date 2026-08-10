use sha2::{Digest, Sha256};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// 下载链接默认有效期(秒):7 天。
pub const TICKET_TTL: i64 = 7 * 86400;

/// HMAC-SHA256(密钥进程级随机,复制出的下载链接带签名,重启后旧链接失效)。
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = key.to_vec();
    if k.len() > BLOCK {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(BLOCK, 0);
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(msg);
    let hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(hash);
    outer.finalize().into()
}

fn to_hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for v in b {
        s.push_str(&format!("{v:02x}"));
    }
    s
}

/// 签发下载链接签名:payload = "{kind}|{share_id}|{rel}|{exp}"。
/// 单文件 kind=file,批量 zip kind=zip(rel 为空串时用 path|names 表达)。
pub fn sign(st: &AppState, kind: &str, share_id: i64, rel: &str, exp: i64) -> String {
    let msg = format!("{kind}|{share_id}|{rel}|{exp}");
    to_hex(&hmac_sha256(&st.dl_key, msg.as_bytes()))
}

/// 校验签名与有效期。`expect_kind` 限制票据类型(file/zip)。
pub fn verify(st: &AppState, kind: &str, share_id: i64, rel: &str, exp: i64, sig: &str) -> ApiResult<()> {
    let now = crate::now();
    if exp < now || exp > now + TICKET_TTL * 4 {
        return Err(ApiError::forbidden("下载链接已过期,请重新复制"));
    }
    let expect = sign(st, kind, share_id, rel, exp);
    let ok = sig.len() == expect.len() && sig.eq_ignore_ascii_case(&expect);
    if !ok {
        return Err(ApiError::forbidden("下载链接无效"));
    }
    Ok(())
}

/// 给单个路径签发完整下载 URL(以真实文件名结尾,便于 curl/wget 直接保存)。
pub fn file_url(st: &AppState, share_id: i64, rel: &str) -> String {
    let exp = crate::now() + TICKET_TTL;
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let sig = sign(st, "file", share_id, rel, exp);
    format!(
        "/api/dl/{share_id}/{name}?path={}&exp={exp}&sig={sig}",
        crate::stream::urlencode(rel)
    )
}

/// 给批量 zip 下载签发完整 URL。
pub fn zip_url(st: &AppState, share_id: i64, path: &str, names: &[String]) -> String {
    let exp = crate::now() + TICKET_TTL;
    let csv = names.join(",");
    let rel = format!("{path}|{csv}");
    let sig = sign(st, "zip", share_id, &rel, exp);
    let ns: Vec<String> = names.iter().map(|n| crate::stream::urlencode(n)).collect();
    format!(
        "/api/zip?share_id={share_id}&path={}&names={}&exp={exp}&sig={sig}",
        crate::stream::urlencode(path),
        ns.join(",")
    )
}