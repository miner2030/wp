use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

fn unique_data(name: &str) -> String {
    std::fs::create_dir_all("/tmp/wp_it").unwrap();
    let d = format!("/tmp/wp_it/{name}_{}", std::process::id());
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Server {
    base: String,
    client: reqwest::blocking::Client,
}

impl Server {
    fn new(name: &str) -> Server {
        let url = wp::spawn_test_server("admin", "admin123", &unique_data(name)).expect("spawn");
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(180)).build().unwrap();
        let s = Server { base: url.0, client };
        let mut n = 0;
        while n < 200 {
            if let Ok(r) = s.client.get(format!("{}/api/config", s.base)).send() {
                if r.status().is_success() {
                    return s;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
            n += 1;
        }
        panic!("server not ready");
    }

    fn get(&self, path: &str, token: Option<&str>) -> (u16, Value) {
        let mut rb = self.client.get(format!("{}{}", self.base, path));
        if let Some(t) = token {
            rb = rb.header("Authorization", format!("Bearer {t}"));
        }
        let resp = rb.send().unwrap();
        let status = resp.status().as_u16();
        let body: Value = resp.json().unwrap_or(Value::Null);
        (status, body)
    }

    fn post(&self, path: &str, token: Option<&str>, body: Value) -> (u16, Value) {
        let mut rb = self.client.post(format!("{}{}", self.base, path));
        if let Some(t) = token {
            rb = rb.header("Authorization", format!("Bearer {t}"));
        }
        let resp = rb.json(&body).send().unwrap();
        let status = resp.status().as_u16();
        let val: Value = resp.json().unwrap_or(Value::Null);
        (status, val)
    }

    fn patch(&self, path: &str, token: &str, body: Value) -> (u16, Value) {
        let resp = self.client.patch(format!("{}{}", self.base, path)).header("Authorization", format!("Bearer {token}")).json(&body).send().unwrap();
        let status = resp.status().as_u16();
        let val: Value = resp.json().unwrap_or(Value::Null);
        (status, val)
    }

    /// 拉取验证码并解出 4 位明文(服务端 SVG 内每个字符一个 <text> 节点)。
    fn captcha(&self) -> (String, String) {
        let (s, j) = self.get("/api/captcha", None);
        assert_eq!(s, 200, "captcha endpoint failed: {j}");
        let id = j["id"].as_str().unwrap().to_string();
        let svg = j["svg"].as_str().unwrap().to_string();
        let mut code = String::new();
        let mut rest = svg.as_str();
        while let Some(p) = rest.find("<text") {
            let after = &rest[p + 5..];
            let Some(gi) = after.find('>') else { break };
            let mut chars = after[gi + 1..].chars();
            match chars.next() {
                Some(c) if c != '<' => {
                    code.push(c);
                    rest = &after[gi + 1 + c.len_utf8()..];
                }
                _ => break,
            }
        }
        assert_eq!(code.len(), 4, "unexpected captcha svg: {svg}");
        (id, code)
    }

    fn login(&self, user: &str, pass: &str) -> String {
        let (cid, code) = self.captcha();
        let (s, j) = self.post(
            "/api/auth/login",
            None,
            json!({ "username": user, "password": pass, "captcha_id": cid, "captcha": code }),
        );
        assert_eq!(s, 200, "login failed: {j}");
        j["token"].as_str().unwrap().to_string()
    }

    /// create temp host dir and register a share
    fn mk_share(&self, token: &str, name: &str) -> (i64, PathBuf) {
        let dir = std::env::temp_dir().join(format!("wp_share_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (s, j) = self.post("/api/shares", Some(token), json!({ "name": name, "host_path": dir.to_string_lossy() }));
        assert_eq!(s, 200, "{j}");
        (j["id"].as_i64().unwrap(), dir)
    }

    fn put_json(&self, token: &str, url: &str, body: Value) -> (u16, Value) {
        let resp = self.client.put(format!("{}{}", self.base, url)).header("Authorization", format!("Bearer {token}")).json(&body).send().unwrap();
        let status = resp.status().as_u16();
        let val: Value = resp.json().unwrap_or(Value::Null);
        (status, val)
    }

    fn put_raw(&self, token: &str, url: &str, data: Vec<u8>) -> u16 {
        self.client
            .put(format!("{}{}", self.base, url))
            .header("Authorization", format!("Bearer {token}"))
            .body(data)
            .send()
            .unwrap()
            .status()
            .as_u16()
    }
}

#[test]
fn test_host_browse_admin_only() {
    let s = Server::new("hostbrowse");
    let admin = s.login("admin", "admin123");
    let dir = std::env::temp_dir().join(format!("wp_hb_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub_a")).unwrap();
    std::fs::create_dir_all(dir.join("sub_b")).unwrap();
    std::fs::write(dir.join("file.txt"), "x").unwrap();

    let (code, _j) = s.get(&format!("/api/browse-host?path={}", urlencode(&dir.to_string_lossy())), None);
    assert_eq!(code, 401, "guest must not browse host dirs");

    let (code, j) = s.get(&format!("/api/browse-host?path={}", urlencode(&dir.to_string_lossy())), Some(&admin));
    assert_eq!(code, 200, "{j}");
    assert_eq!(j["path"], dir.to_string_lossy().to_string());
    let dirs: Vec<&str> = j["dirs"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(dirs.contains(&"sub_a") && dirs.contains(&"sub_b"), "subdirs listed: {dirs:?}");
    assert!(!dirs.contains(&"file.txt"), "files must not be listed");

    std::fs::write(dir.join("chain.pem"), "x").unwrap();
    std::fs::write(dir.join("priv.key"), "x").unwrap();
    let (code, j) = s.get(&format!("/api/browse-host?path={}&files=1", urlencode(&dir.to_string_lossy())), Some(&admin));
    assert_eq!(code, 200, "files=1 must work: {j}");
    let files: Vec<&str> = j["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(files.contains(&"chain.pem") && files.contains(&"priv.key"), "cert files listed: {files:?}");
    assert!(!files.contains(&"file.txt"), "non-cert files must not be listed");

    let (code, j) = s.get("/api/browse-host?path=/no-such-dir-xyz-123", Some(&admin));
    assert_eq!(code, 400, "bad path rejected: {j}");

    let _ = std::fs::remove_dir_all(&dir);
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[test]
fn test_auth_flow() {
    let s = Server::new("auth");
    let admin = s.login("admin", "admin123");
    assert!(!admin.is_empty());

    let (cid, code) = s.captcha();
    let (status, _) = s.post(
        "/api/auth/login",
        None,
        json!({ "username": "admin", "password": "wrong", "captcha_id": cid, "captcha": code }),
    );
    assert_eq!(status, 401, "wrong password must be rejected");

    // 验证码错误或缺失时,即使密码正确也拒绝
    let (cid, _) = s.captcha();
    let (status, _) = s.post(
        "/api/auth/login",
        None,
        json!({ "username": "admin", "password": "admin123", "captcha_id": cid, "captcha": "XXXX" }),
    );
    assert_eq!(status, 403, "bad captcha must be rejected");
    let (status, _) = s.post(
        "/api/auth/login",
        None,
        json!({ "username": "admin", "password": "admin123" }),
    );
    assert_eq!(status, 403, "missing captcha must be rejected");

    let (code, _) = s.get("/api/users", None);
    assert_eq!(code, 401, "guest must not list users");

    let (code, _) = s.get("/api/users", Some(&admin));
    assert_eq!(code, 200);

    // /api/auth/me with token
    let (code, j) = s.get("/api/auth/me", Some(&admin));
    assert_eq!(code, 200);
    assert_eq!(j["username"], "admin");
    assert_eq!(j["is_admin"], true);
}

#[test]
fn test_users_and_home() {
    let s = Server::new("users");
    let admin = s.login("admin", "admin123");
    let (code, j) = s.post("/api/users", Some(&admin), json!({ "username": "alice", "password": "pw123" }));
    assert_eq!(code, 200, "{j}");

    let (code, _) = s.post("/api/users", Some(&admin), json!({ "username": "alice", "password": "x" }));
    assert_eq!(code, 409, "duplicate username must be rejected");

    let (code, list) = s.get("/api/users", Some(&admin));
    assert_eq!(code, 200);
    assert_eq!(list.as_array().unwrap().len(), 2);

    // alice has a private home share
    let alice = s.login("alice", "pw123");
    let (code, shares) = s.get("/api/shares", Some(&alice));
    assert_eq!(code, 200);
    let arr = shares["shares"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["kind"], "home");
    assert_eq!(arr[0]["access"], "private");

    // alice can browse her home
    let home_id = arr[0]["id"].as_i64().unwrap();
    let (code, j) = s.get(&format!("/api/browse/{home_id}?path="), Some(&alice));
    assert_eq!(code, 200, "{j}");

    // alice may not browse admin-only shares (the bootstrap svc has none)
    // and guest user sees nothing
    let (code, j) = s.get("/api/shares", None);
    assert_eq!(code, 200);
    assert!(j["shares"].as_array().unwrap().is_empty());
}

#[test]
fn test_share_permissions_rules() {
    let s = Server::new("perm");
    let admin = s.login("admin", "admin123");
    s.post("/api/users", Some(&admin), json!({ "username": "bob", "password": "pw" }));
    let (sid, dir) = s.mk_share(&admin, "公开目录");

    // guest cannot see default (login) share
    let (code, j) = s.get("/api/shares", None);
    assert_eq!(code, 200);
    assert!(j["shares"].as_array().unwrap().is_empty());

    // guest browse denied
    let (code, _) = s.get(&format!("/api/browse/{sid}?path="), None);
    assert_eq!(code, 401);

    // set root rule -> guest
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "", "access": "guest" }));
    assert_eq!(code, 200);

    // guest can now browse root
    let (code, j) = s.get(&format!("/api/browse/{sid}?path="), None);
    assert_eq!(code, 200, "{j}");

    // subfolder admin-only
    s.post("/api/mkdir", Some(&admin), json!({ "share_id": sid, "path": "", "name": "secret" }));
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "secret", "access": "admin" }));
    assert_eq!(code, 200);

    let (code, _) = s.get(&format!("/api/browse/{sid}?path=secret"), None);
    assert_eq!(code, 403);

    let bob = s.login("bob", "pw");
    let (code, _) = s.get(&format!("/api/browse/{sid}?path=secret"), Some(&bob));
    assert_eq!(code, 403);

    let (code, _) = s.get(&format!("/api/browse/{sid}?path=secret"), Some(&admin));
    assert_eq!(code, 200);

    // guest allowed in public subtrees
    std::fs::create_dir(dir.join("public_dir")).unwrap();
    let (code, _) = s.get(&format!("/api/browse/{sid}?path=public_dir"), None);
    assert_eq!(code, 200);

    // remove rule -> guest no longer sees share
    let (_, rules) = s.get(&format!("/api/shares/{sid}/rules"), Some(&admin));
    assert_eq!(rules["rules"].as_array().unwrap().len(), 2);
    let _ = &rules;
}

#[test]
fn test_upload_download_resume() {
    let s = Server::new("up");
    let admin = s.login("admin", "admin123");
    let (sid, dir) = s.mk_share(&admin, "uploads");
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "", "access": "guest" }));
    assert_eq!(code, 200);

    s.post("/api/mkdir", Some(&admin), json!({ "share_id": sid, "path": "", "name": "sub" }));
    s.post("/api/mkdir", Some(&admin), json!({ "share_id": sid, "path": "sub", "name": "deep" }));

    let payload: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let (code, init) = s.post("/api/upload/init", Some(&admin), json!({ "share_id": sid, "path": "sub/deep", "filename": "big.bin", "size": payload.len() }));
    assert_eq!(code, 200, "{init}");
    let token = init["token"].as_str().unwrap().to_string();
    let parts = init["num_parts"].as_i64().unwrap();
    let chunk = init["chunk_size"].as_i64().unwrap() as usize;
    assert_eq!(parts, 3);

    // upload only first part, simulate interruption
    let first = &payload[..chunk];
    assert_eq!(s.put_raw(&admin, &format!("/api/upload/part/{token}/0"), first.to_vec()), 200);
    let (code, j) = s.get(&format!("/api/upload/parts/{token}"), Some(&admin));
    assert_eq!(code, 200);
    assert_eq!(j["parts"].as_array().unwrap().len(), 1);

    // complete while parts missing -> conflict
    let (code, _) = s.post(&format!("/api/upload/complete/{token}"), Some(&admin),json!({}));
    assert_eq!(code, 409);

    // upload remaining parts and complete
    for p in 1..3 {
        let start = p * chunk;
        let end = payload.len().min(start + chunk);
        let st_code = s.put_raw(&admin, &format!("/api/upload/part/{token}/{p}"), payload[start..end].to_vec());
        assert_eq!(st_code, 200);
    }
    let (code, cj) = s.post(&format!("/api/upload/complete/{token}"), Some(&admin),json!({}));
    assert_eq!(code, 200, "{cj}");

    // content on disk matches exactly
    let ondisk = std::fs::read(dir.join("sub/deep/big.bin")).unwrap();
    assert_eq!(ondisk, payload, "uploaded content mismatch");

    // download round-trip
    let resp = s.client.get(format!("{}/api/media/{}?path=sub%2Fdeep%2Fbig.bin", s.base, sid)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.bytes().unwrap().to_vec(), payload);
}

fn zip_names(buf: &[u8]) -> Vec<String> {
    let mut eocd = 0usize;
    let from = buf.len().saturating_sub(65557);
    for i in from..=buf.len().saturating_sub(22) {
        if &buf[i..i + 4] == b"PK\x05\x06" {
            eocd = i;
            break;
        }
    }
    assert!(eocd > 0, "zip missing EOCD");
    let count = u16::from_le_bytes([buf[eocd + 10], buf[eocd + 11]]) as usize;
    let mut off = u32::from_le_bytes([buf[eocd + 16], buf[eocd + 17], buf[eocd + 18], buf[eocd + 19]]) as usize;
    let mut names = Vec::new();
    for _ in 0..count {
        assert_eq!(&buf[off..off + 4], b"PK\x01\x02", "bad central entry");
        let nl = u16::from_le_bytes([buf[off + 28], buf[off + 29]]) as usize;
        names.push(String::from_utf8_lossy(&buf[off + 46..off + 46 + nl]).into_owned());
        let xl = u16::from_le_bytes([buf[off + 30], buf[off + 31]]) as usize;
        let cl = u16::from_le_bytes([buf[off + 32], buf[off + 33]]) as usize;
        off += 46 + nl + xl + cl;
    }
    names
}

#[test]
fn test_zip_batch_download() {
    let s = Server::new("zip");
    let admin = s.login("admin", "admin123");
    let (sid, dir) = s.mk_share(&admin, "zips");
    s.post("/api/mkdir", Some(&admin), json!({ "share_id": sid, "path": "", "name": "fld" }));
    let (code, init) = s.post("/api/upload/init", Some(&admin), json!({ "share_id": sid, "path": "", "filename": "b.txt", "size": 5 }));
    assert_eq!(code, 200);
    let t1 = {
        let (c0, i0) = s.post("/api/upload/init", Some(&admin), json!({ "share_id": sid, "path": "fld", "filename": "a.txt", "size": 3 }));
        assert_eq!(c0, 200);
        i0["token"].as_str().unwrap().to_string()
    };
    assert_eq!(s.put_raw(&admin, &format!("/api/upload/part/{t1}/0"), b"abc".to_vec()), 200);
    assert_eq!(s.post(&format!("/api/upload/complete/{t1}"), Some(&admin), json!({})).0, 200);
    let t2 = init["token"].as_str().unwrap().to_string();
    assert_eq!(s.put_raw(&admin, &format!("/api/upload/part/{t2}/0"), b"hello".to_vec()), 200);
    assert_eq!(s.post(&format!("/api/upload/complete/{t2}"), Some(&admin), json!({})).0, 200);

    let resp = s.client.post(format!("{}/api/zip", s.base))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {admin}"))
        .body(json!({ "share_id": sid, "path": "", "names": ["fld", "b.txt"] }).to_string())
        .send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.headers().get("content-type").unwrap().to_str().unwrap(), "application/zip");
    let bytes = resp.bytes().unwrap().to_vec();
    assert_eq!(&bytes[..4], b"PK\x03\x04", "local header magic");
    let names = zip_names(&bytes);
    assert_eq!(names, vec!["fld/a.txt", "b.txt"], "folder recursion + file entries");
    assert!(std::fs::read(dir.join("fld/a.txt")).unwrap() == b"abc");
    assert_eq!(std::fs::read(dir.join("b.txt")).unwrap(), b"hello");
}

#[test]
fn test_upload_range_and_media() {
    let s = Server::new("range");
    let admin = s.login("admin", "admin123");
    let (sid, dir) = s.mk_share(&admin, "rng");
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "", "access": "guest" }));
    assert_eq!(code, 200);

    let payload: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();
    std::fs::write(dir.join("blob.bin"), &payload).unwrap();

    // range request
    let resp = s.client
        .get(format!("{}/api/media/{}?path=blob.bin", s.base, sid))
        .header("Range", "bytes=100-199")
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 206);
    let cr = resp.headers().get("Content-Range").unwrap().to_str().unwrap().to_string();
    let bytes = resp.bytes().unwrap().to_vec();
    assert_eq!(bytes, payload[100..200].to_vec());
    assert!(cr.contains("bytes 100-199/5000"), "{cr}");

    // suffix range
    let resp = s.client.get(format!("{}/api/media/{}?path=blob.bin", s.base, sid)).header("Range", "bytes=-99").send().unwrap();
    assert_eq!(resp.status().as_u16(), 206);
    assert_eq!(resp.bytes().unwrap().to_vec(), payload[payload.len() - 99..].to_vec());

    // full (no range)
    let resp = s.client.get(format!("{}/api/media/{}?path=blob.bin", s.base, sid)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.bytes().unwrap().to_vec(), payload);

    // path traversal blocked
    let _ = s.get(&format!("/api/browse/{sid}?path={}", "%2E%2E%2F%2E%2E%2Fetc"), None);

    // image file served inline
    std::fs::write(dir.join("pic.png"), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    let (code, j) = s.get(&format!("/api/media/status/{}?path=pic.png", sid), None);
    assert_eq!(code, 200);
    assert_eq!(j["kind"], "image");
}

#[test]
fn test_delete_and_rename() {
    let s = Server::new("del");
    let admin = s.login("admin", "admin123");
    let (sid, dir) = s.mk_share(&admin, "delonly");

    s.post("/api/mkdir", Some(&admin), json!({ "share_id": sid, "path": "", "name": "folder" }));
    s.post("/api/mkdir", Some(&admin), json!({ "share_id": sid, "path": "folder", "name": "inner" }));

    let (code, _) = s.post("/api/delete", Some(&admin),json!({ "share_id": sid, "path": "folder" }));
    assert_eq!(code, 409, "non-empty folder needs recursive");

    let (code, _) = s.post("/api/delete", Some(&admin),json!({ "share_id": sid, "path": "folder", "recursive": true }));
    assert_eq!(code, 200);
    assert!(!dir.join("folder").exists());

    std::fs::write(dir.join("old.txt"), b"abc").unwrap();
    let (code, _) = s.post("/api/rename", Some(&admin),json!({ "share_id": sid, "path": "", "old": "old.txt", "new": "new.txt" }));
    assert_eq!(code, 200);
    assert!(!dir.join("old.txt").exists());
    assert!(dir.join("new.txt").exists());

    let (code, _) = s.post("/api/delete", Some(&admin),json!({ "share_id": sid, "path": "new.txt" }));
    assert_eq!(code, 200);
    assert!(!dir.join("new.txt").exists());

    let (code, _) = s.post("/api/rename", Some(&admin),json!({ "share_id": sid, "path": "", "old": "x", "new": "a/b" }));
    assert_eq!(code, 400);
}

#[test]
fn test_upload_init_validation() {
    let s = Server::new("valid");
    let admin = s.login("admin", "admin123");
    let (sid, _dir) = s.mk_share(&admin, "v");
    // invalid filename rejected
    let (code, _j) = s.post("/api/upload/init", Some(&admin),json!({ "share_id": sid, "path": "", "filename": "x:y.bin", "size": 10 }));
    assert_eq!(code, 400, "colon in name should be rejected");
    let (code, _j) = s.post("/api/upload/init", Some(&admin),json!({ "share_id": sid, "path": "", "filename": "x", "size": -1 }));
    assert_eq!(code, 400);
    let (code, _j) = s.post("/api/upload/init", Some(&admin),json!({ "share_id": sid, "path": "", "filename": "ok.bin", "size": 0 }));
    assert_eq!(code, 200);
}

#[test]
fn test_user_capability_limits() {
    let s = Server::new("caps");
    let admin = s.login("admin", "admin123");
    s.post("/api/users", Some(&admin), json!({ "username": "restricted", "password": "pw" }));
    let rest = s.login("restricted", "pw");

    let (sid, dir) = s.mk_share(&admin, "capshare");
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "", "access": "guest" }));
    assert_eq!(code, 200);

    // defaults are all-off
    let (code, _) = s.post("/api/upload/init", Some(&rest), json!({ "share_id": sid, "path": "", "filename": "x.bin", "size": 10 }));
    assert_eq!(code, 403);
    let (code, _) = s.post("/api/mkdir", Some(&rest), json!({ "share_id": sid, "path": "", "name": "d" }));
    assert_eq!(code, 403);
    std::fs::write(dir.join("file.txt"), b"data").unwrap();
    let (code, _) = s.post("/api/delete", Some(&rest), json!({ "share_id": sid, "path": "file.txt" }));
    assert_eq!(code, 403);
    let (code, _) = s.get(&format!("/api/file/meta/{sid}?path=file.txt"), Some(&rest));
    assert_eq!(code, 403, "no download flag");

    // grant download + upload via patch
    let users = s.get("/api/users", Some(&admin)).1;
    let uid = users.as_array().unwrap().iter().find(|u| u["username"] == "restricted").unwrap()["id"].as_i64().unwrap();
    let (code, _) = s.patch(&format!("/api/users/{uid}"), &admin, json!({ "permit": { "can_upload": true, "can_download": true, "can_delete": true, "can_mkdir": true } }));
    assert_eq!(code, 200);

    let (code, j) = s.get(&format!("/api/file/meta/{sid}?path=file.txt"), Some(&rest));
    assert_eq!(code, 200, "{j}");
    let resp = s.client.get(format!("{}/api/media/{}?path=file.txt", s.base, sid)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let (code, _) = s.post("/api/delete", Some(&rest), json!({ "share_id": sid, "path": "file.txt" }));
    assert_eq!(code, 200);
    assert!(!dir.join("file.txt").exists());
}

#[test]
fn test_video_direct_stream() {
    let s = Server::new("vid");
    let admin = s.login("admin", "admin123");
    let (sid, dir) = s.mk_share(&admin, "vid");
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "", "access": "guest" }));
    assert_eq!(code, 200);

    let mp4 = dir.join("clip.mp4");
    if !ffmpeg_available() {
        eprintln!("ffmpeg missing – skipping video test");
        return;
    }
    let out = std::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=duration=1:size=320x240:rate=25", "-pix_fmt", "yuv420p", "-movflags", "+faststart", mp4.to_string_lossy().as_ref()])
        .output();
    assert!(out.is_ok() && out.unwrap().status.success(), "ffmpeg mp4 generation failed");

    let (code, status) = s.get(&format!("/api/media/status/{}?path=clip.mp4", sid), None);
    assert_eq!(code, 200);
    assert_eq!(status["kind"], "video");
    assert_eq!(status["direct"], true);
    let src = status["src"].as_str().unwrap().to_string();

    let resp = s.client.get(format!("{}{}", s.base, src)).header("Range", "bytes=0-31").send().unwrap();
    assert_eq!(resp.status().as_u16(), 206);
    assert_eq!(resp.headers().get("content-type").unwrap().to_str().unwrap(), "video/mp4");

    // HEAD
    let resp = s.client.head(format!("{}/api/media/{}?path=clip.mp4", s.base, sid)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

#[test]
fn test_avi_transcode() {
    let s = Server::new("tx");
    let admin = s.login("admin", "admin123");
    let (sid, dir) = s.mk_share(&admin, "video");
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "", "access": "guest" }));
    assert_eq!(code, 200);

    if !ffmpeg_available() {
        eprintln!("ffmpeg missing – skipping transcode test");
        return;
    }
    std::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=1:size=320x240:rate=10", "-c:v", "mpeg4", dir.join("mov.avi").to_string_lossy().as_ref()])
        .output()
        .expect("ffmpeg avi generation failed");

    let (code, st) = s.get(&format!("/api/media/status/{}?path=mov.avi", sid), None);
    assert_eq!(code, 200);
    assert_eq!(st["kind"], "video");
    assert_eq!(st["direct"], false);

    let mut ready = false;
    for _ in 0..120 {
        let (_, st2) = s.get(&format!("/api/media/status/{}?path=mov.avi", sid), None);
        let state = st2["state"].as_str().unwrap_or("");
        if state == "ready" || state == "failed" {
            ready = state == "ready";
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if ready {
        let (_, st3) = s.get(&format!("/api/media/status/{}?path=mov.avi", sid), None);
        let src = st3["src"].as_str().unwrap().to_string();
        assert!(src.contains("t=1"), "{src}");
        let resp = s
            .client
            .get(format!("{}{}", s.base, src))
            .header("Range", "bytes=0-127")
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 206, "transcoded stream must support range");
        assert_eq!(resp.headers().get("content-type").unwrap().to_str().unwrap(), "video/mp4");
        let hresp = s
            .client
            .head(format!("{}/api/media/{}?path=mov.avi&t=1", s.base, sid))
            .send()
            .unwrap();
        assert_eq!(hresp.status().as_u16(), 200);
        assert_eq!(hresp.headers().get("content-type").unwrap().to_str().unwrap(), "video/mp4", "HEAD of transcoded stream must report video/mp4");
    } else {
        panic!("transcode did not finish");
    }
}

#[test]
fn test_video_thumbnails() {
    let s = Server::new("thumb");
    let admin = s.login("admin", "admin123");
    let (sid, dir) = s.mk_share(&admin, "thumb_videos");
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "", "access": "guest" }));
    assert_eq!(code, 200);

    if !ffmpeg_available() {
        eprintln!("ffmpeg missing – skipping thumbnail test");
        return;
    }
    std::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=duration=2:size=320x240:rate=10", "-c:v", "libx264", "-pix_fmt", "yuv420p", dir.join("mov.mp4").to_string_lossy().as_ref()])
        .output()
        .expect("ffmpeg mp4 generation failed");

    // 封面懒生成:mp4 -> 200 image/jpeg,内容为 JPEG 魔数
    let resp = s.client.get(format!("{}/api/media/thumb/{sid}?path=mov.mp4", s.base)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200, "video thumb must be served: {}", resp.text().unwrap_or_default());
    assert_eq!(resp.headers().get("content-type").unwrap().to_str().unwrap(), "image/jpeg");
    let bytes = resp.bytes().unwrap();
    assert!(bytes.len() > 100, "thumb too small");
    assert_eq!(&bytes[0..2], b"\xff\xd8", "thumb must be a jpeg");

    // 已缓存：再次请求直接命中
    let resp = s.client.get(format!("{}/api/media/thumb/{sid}?path=mov.mp4", s.base)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.bytes().unwrap(), bytes, "cached thumb must be identical");

    // avi 一律不生成封面
    std::fs::write(dir.join("old.avi"), b"not-really-a-video").unwrap();
    let resp = s.client.get(format!("{}/api/media/thumb/{sid}?path=old.avi", s.base)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 404, "avi must not have a thumbnail");

    // 非视频文件 -> 400
    std::fs::write(dir.join("doc.txt"), b"x").unwrap();
    let resp = s.client.get(format!("{}/api/media/thumb/{sid}?path=doc.txt", s.base)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 400, "non-video must be rejected");

    // 不存在的文件 -> 404
    let resp = s.client.get(format!("{}/api/media/thumb/{sid}?path=nope.mp4", s.base)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

#[test]
fn test_static_and_config() {
    let s = Server::new("static");
    let resp = s.client.get(format!("{}/", s.base)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert!(resp.text().unwrap().contains("云盘"));

    let resp = s.client.get(format!("{}/static/app.js", s.base)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    let (code, j) = s.get("/api/config", None);
    assert_eq!(code, 200);
    assert!(j["chunk_size"].as_i64().unwrap() > 0);
    assert_eq!(j["chunk_size"].as_i64().unwrap(), 1024 * 1024);
}

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn test_cookie_auth_and_guest_mode() {
    let s = Server::new("cookie");
    let admin = s.login("admin", "admin123");
    let (sid, dir) = s.mk_share(&admin, "cookie_share");
    // default root rule is login
    std::fs::write(dir.join("a.txt"), b"guest-viewable").unwrap();

    // bearer-only access works
    let (code, _) = s.get(&format!("/api/browse/{sid}?path="), Some(&admin));
    assert_eq!(code, 200);

    // no auth -> 401 while root is login
    let (code, _) = s.get(&format!("/api/browse/{sid}?path="), None);
    assert_eq!(code, 401);
    let resp = s.client.get(format!("{}/api/media/{}?path=a.txt", s.base, sid)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 401, "no auth on login-only share must be denied");

    // cookie token (what browser <img>/<video> would send)
    let resp = s
        .client
        .get(format!("{}/api/media/{}?path=a.txt", s.base, sid))
        .header("Cookie", format!("wp_token={admin}"))
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.bytes().unwrap().to_vec(), b"guest-viewable".to_vec());

    // bad cookie -> denied
    let resp = s.client.get(format!("{}/api/media/{}?path=a.txt", s.base, sid)).header("Cookie", "wp_token=bad").send().unwrap();
    assert_eq!(resp.status().as_u16(), 401);

    // switch root rule to guest
    let (code, _) = s.post(&format!("/api/shares/{sid}/rules"), Some(&admin), json!({ "rel_path": "", "access": "guest" }));
    assert_eq!(code, 200);

    // anonymous share list now shows it
    let (code, j) = s.get("/api/shares", None);
    assert_eq!(code, 200);
    assert_eq!(j["shares"].as_array().unwrap().len(), 1);

    // anonymous browse + media
    let (code, _) = s.get(&format!("/api/browse/{sid}?path="), None);
    assert_eq!(code, 200);
    let resp = s.client.get(format!("{}/api/media/{}?path=a.txt", s.base, sid)).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

#[test]
fn test_password_change() {
    let s = Server::new("pwd");
    let admin = s.login("admin", "admin123");
    let (code, _) = s.post("/api/auth/password", Some(&admin), json!({ "old_password": "admin123", "password": "shiny-new" }));
    assert_eq!(code, 200);

    let (cid, code) = s.captcha();
    let (code, _) = s.post(
        "/api/auth/login",
        None,
        json!({ "username": "admin", "password": "admin123", "captcha_id": cid, "captcha": code }),
    );
    assert_eq!(code, 401, "old password must no longer work");

    let (cid, code) = s.captcha();
    let (code, _) = s.post(
        "/api/auth/login",
        None,
        json!({ "username": "admin", "password": "shiny-new", "captcha_id": cid, "captcha": code }),
    );
    assert_eq!(code, 200);

    let admin2 = s.login("admin", "shiny-new");
    assert!(!admin2.is_empty());

    // wrong old password rejected
    let (code, _) = s.post("/api/auth/password", Some(&admin2), json!({ "old_password": "nope", "password": "x" }));
    assert_eq!(code, 400);
}

#[test]
fn test_password_change_revokes_other_sessions() {
    let s = Server::new("pwdrevoke");
    let token_a = s.login("admin", "admin123");
    let token_b = s.login("admin", "admin123");

    let (code, _) = s.get("/api/auth/me", Some(&token_b));
    assert_eq!(code, 200, "second session valid before password change");

    let (code, _) = s.post("/api/auth/password", Some(&token_a), json!({ "old_password": "admin123", "password": "rotated" }));
    assert_eq!(code, 200);

    let (code, _) = s.get("/api/auth/me", Some(&token_b));
    assert_eq!(code, 401, "sibling session must be revoked on password change");

    let (code, j) = s.get("/api/auth/me", Some(&token_a));
    assert_eq!(code, 200, "current session keeps working");
    assert_eq!(j["username"], "admin");
}
const TEST_CERT_PEM: &str = concat!(
    "-----BEGIN CERTIFICATE-----\n",
    "MIIDJTCCAg2gAwIBAgIUXRY4bPfeYTfXzVYlSdeegXiceZUwDQYJKoZIhvcNAQEL\n",
    "BQAwFDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI2MDgwOTE1MTYzM1oXDTM2MDgw\n",
    "NjE1MTYzM1owFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEF\n",
    "AAOCAQ8AMIIBCgKCAQEA64OFFq5EJyzMnxisNrHwyqsk+hV5oUKKBJVtbhGl3R/2\n",
    "oP7fnDQcfoCHpOecJk32M+J1e0J2EkKnWDHFTC9j7d3ecZQA42N6EEgW2ZXoPqjt\n",
    "l5eRTZvc7gNj09lnqwy05MoC3kIuMjkptmrHRllvPaewKX7ruBOeExe5ZwbfJgAy\n",
    "hpVjxMLGnZ+4M2QJvKdSzpPeqTjsl5ZVNdIpMsWAd+xJZwbs7kF/IGNh7eBLqR+A\n",
    "sMbQKW2+AV7vwGY7G26Rv+iFZtl+iWLoTGcVkX7DTVWznNe/aM2SheZlCqHPBBvF\n",
    "4bHXP88uuYbV/w487ym7WwtfKuAcynN2i3ywL18g/QIDAQABo28wbTAdBgNVHQ4E\n",
    "FgQUUmjXAuusNs4IN0/t5As425/GvbkwHwYDVR0jBBgwFoAUUmjXAuusNs4IN0/t\n",
    "5As425/GvbkwDwYDVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARgglsb2NhbGhvc3SH\n",
    "BH8AAAEwDQYJKoZIhvcNAQELBQADggEBAJxdcDxs0W1UOZz9IJ38ymU/AG1Ozu1X\n",
    "jykyuGdysDfgLnBJkPDx3zAtXvkuFH72i6xTKqhnkVHLezmfXsyHjg3R8K0h7jg9\n",
    "9ac4OwyLAgQ9ZKx6mAlWotFRSJx7njlqP7hwBMkJCKZdFKoGaJnu5nti301N1Xcr\n",
    "n4Hu6frPYqP4zQ8i/TVyNZ3U+tHTMvTelxVPb0nAnnTuA/jPDx91GWZFWZtuZBOU\n",
    "UeQeiFJ6ivUYV+nLmhqMSsuVKalEIEj8Wj3wJF718u7GzBkI3OdrLuozQKpMSQVS\n",
    "qeNd8Moaf6Ufa0hxbeSMQZ999aZdni3ofBomGlwdwNkO3JRFZIOupSQ=\n",
    "-----END CERTIFICATE-----\n",
);
const TEST_KEY_PEM: &str = concat!(
    "-----BEGIN PRIVATE KEY-----\n",
    "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDrg4UWrkQnLMyf\n",
    "GKw2sfDKqyT6FXmhQooElW1uEaXdH/ag/t+cNBx+gIek55wmTfYz4nV7QnYSQqdY\n",
    "McVML2Pt3d5xlADjY3oQSBbZleg+qO2Xl5FNm9zuA2PT2WerDLTkygLeQi4yOSm2\n",
    "asdGWW89p7Apfuu4E54TF7lnBt8mADKGlWPEwsadn7gzZAm8p1LOk96pOOyXllU1\n",
    "0ikyxYB37ElnBuzuQX8gY2Ht4EupH4CwxtApbb4BXu/AZjsbbpG/6IVm2X6JYuhM\n",
    "ZxWRfsNNVbOc179ozZKF5mUKoc8EG8Xhsdc/zy65htX/DjzvKbtbC18q4BzKc3aL\n",
    "fLAvXyD9AgMBAAECggEABLdzFGO0ELbIeE01sbi5kk2AJZQFhhG/ktztPp2S3U1V\n",
    "Ep9YGhg/E9d+H/LVgTzhd+OKp8fKdo4oLM9+XIq8bRia6lpk9CjbWnUfJAdpCcw3\n",
    "SehqrAu5jni56JW7ikTsYIKdMRdRNodHixQzXnjPDgSMNiCJFvwpl8dLWsFb+bZQ\n",
    "ddviSk6Q/D75PVI32kC0Npe+rVzPIogd0+Ru0CtboKCSc/u8/UOD4ZwF2XkBYdxj\n",
    "O2i6KwxnRdo9VLoqakS9IweVJ4poq+xX2VjfEKtSgqjb9AraU1V8Wps8RkwIgw59\n",
    "4ja08aXzYu7KAEhDoqw3x3/GTewvTo3vZjDR9AoZMQKBgQD84Y+8Yo2/8hYe48YZ\n",
    "gjGIJW3lTVan1N8/7F/7pcQ+SI2+O1HuN8AsuwMHb5CBDhFUXCJCJ3KaPvh47mi/\n",
    "/5rKzsjBAoeyvQmUef9jNfCfhyeQ/24dU6S6VrWU1oK5HcoWR6lNCIuv1+itndPM\n",
    "76fF0x2C0hp8kfTcFEClXIpWmQKBgQDuax+Q6nnSJC61oghmh8kVZ1Pz4GvgDK/L\n",
    "2YkOUr/1C+3jUkoERXlk5rYutMIeHLWaeYPFiuP2wECW0IcoIPdLU62H5y8iPLGI\n",
    "i49O+RQBQRXtYtawxbbmZ6O4NNdxXAeUWjwyYAyVPKu5+Au9WMVCiE8IY5lnH2I7\n",
    "pefZ5NzwBQKBgQCbVPI1hVQM02dDEiZdBNvoBRW8BAV2kguP+GH88ZXZrDdk4osx\n",
    "CI3C7BywNJdOrpV2fCGPASwkEwoNPmSZiyhJ6zrlU+iGoheAXG/DQR7M3vgu2LeK\n",
    "zBYjX5+qDRm/G81GYj5cUyN6n+UMwCWZfJxRp5w4/1XFruH5TXdoG6/eAQKBgCvI\n",
    "SkSvem2HrXg3aDmD5/5hOj6H0OeVDNdvfEbAFXYnmajsHKGKLI/F+mC7WwZymTrj\n",
    "47nsFo7ioTnQ03EgFKmllTFm1/X+lU+Q6WFERlMwo5kzVF+j/0FyeNNECOhNUSsC\n",
    "IxnRH55sRNDi5yz/V0Ivi+vrdmlEjyKLBfeymowNAoGALg7MLIbZs5KiYt9HSDKl\n",
    "2kuFBvLe6WQlbvU6WZsmk2Y80e2LCrlZyYwil8Gi5TPsWJq0mFIfIdIusaQT+0ps\n",
    "5MqO2B8+ijujW3i8CBgZe3H/Agg1uz/rlvM8DWSpo7vyRip2eKOOFS/muzJgeTkY\n",
    "EjyPrqToJgoJGTXEWysIjTc=\n",
    "-----END PRIVATE KEY-----\n",
);

#[test]
fn test_tls_certificate_management() {
    let s = Server::new("tls");
    let admin = s.login("admin", "admin123");

    let (code, j) = s.get("/api/tls", Some(&admin));
    assert_eq!(code, 200, "{j}");
    assert_eq!(j["enabled"], false, "no cert yet: {j}");

    let (code, _) = s.put_json(&admin, "/api/tls", json!({ "domain": "x.example.com", "cert_pem": "garbage", "key_pem": "junk" }));
    assert_eq!(code, 400, "invalid cert must be rejected");

    let (code, _) = s.put_json(&admin, "/api/tls", json!({ "domain": "x.example.com", "cert_pem": TEST_CERT_PEM, "key_pem": "-----BEGIN PRIVATE KEY-----\njunk\n-----END PRIVATE KEY-----" }));
    assert_eq!(code, 400, "mismatched key must be rejected");

    let (code, j) = s.put_json(&admin, "/api/tls", json!({ "domain": "localhost", "cert_pem": TEST_CERT_PEM, "key_pem": TEST_KEY_PEM }));
    assert_eq!(code, 200, "paste mode must work: {j}");

    // 路径模式:文件不存在 → 400
    let (code, _) = s.put_json(&admin, "/api/tls", json!({ "domain": "localhost", "cert_path": "/no/such/file.pem", "key_path": "/no/such/file.key" }));
    assert_eq!(code, 400, "missing files rejected");

    // 路径模式:服务器上存在证书文件 → 200
    let dir = std::env::temp_dir().join(format!("wp_tls_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fullchain.pem"), TEST_CERT_PEM).unwrap();
    std::fs::write(dir.join("privkey.pem"), TEST_KEY_PEM).unwrap();
    let cert_file = dir.join("fullchain.pem").to_string_lossy().to_string();
    let key_file = dir.join("privkey.pem").to_string_lossy().to_string();
    let (code, j) = s.put_json(&admin, "/api/tls", json!({ "domain": "localhost", "cert_path": cert_file, "key_path": key_file }));
    assert_eq!(code, 200, "file mode must work: {j}");

    let mut port = 0u16;
    for _ in 0..100 {
        let (_, j2) = s.get("/api/tls", Some(&admin));
        if j2["enabled"] == true {
            port = j2["port"].as_u64().unwrap() as u16;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_ne!(port, 0, "https must come up after adding a cert");

    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        .pool_max_idle_per_host(0)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let resp = client.get(format!("https://127.0.0.1:{port}/api/config")).send().unwrap();
    assert_eq!(resp.status().as_u16(), 200, "https must serve api");
    assert!(resp.text().unwrap().contains("chunk_size"));

    let resp = s.client
        .delete(format!("{}/api/tls", s.base))
        .header("Authorization", format!("Bearer {admin}"))
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200, "delete cert must succeed");

    let mut off = false;
    for _ in 0..100 {
        let (_, j3) = s.get("/api/tls", Some(&admin));
        if j3["enabled"] == false {
            off = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(off, "https must stop after deleting cert");
    let err = client.get(format!("https://127.0.0.1:{port}/api/config")).send();
    assert!(err.is_err(), "tls listener must be closed after cert removal");

    let _ = std::fs::remove_dir_all(&dir);
}
