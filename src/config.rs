use std::path::PathBuf;

pub const DEFAULT_CHUNK: i64 = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: String,
    pub tls_listen: String,
    pub data_dir: PathBuf,
    pub admin_user: String,
    pub admin_pass: String,
    /// 0 表示使用默认分片大小。
    pub chunk_size: i64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: "0.0.0.0:9000".into(),
            tls_listen: "0.0.0.0:443".into(),
            data_dir: PathBuf::from("./wpdata"),
            admin_user: "admin".into(),
            admin_pass: "admin123".into(),
            chunk_size: DEFAULT_CHUNK,
        }
    }
}

fn usage() {
    eprintln!(
        "wp - 多用户网盘\n\
         用法: wp [选项]\n\
         选项:\n\
           --listen <addr>      监听地址 (默认 0.0.0.0:9000)\n\
           --tls-listen <addr>  HTTPS 监听地址,配置证书后启用 (默认 0.0.0.0:443)\n\
           --data <dir>         数据目录 (默认 ./wpdata)\n\
           --admin-user <name>  初始管理员用户名 (默认 admin)\n\
           --admin-pass <pw>    初始管理员密码 (默认 admin123)\n\
           --chunk-size <bytes> 上传分片大小 (默认 8388608)\n\
           -h, --help           帮助"
    );
}

pub fn parse_args() -> Option<Config> {
    let mut args = std::env::args().skip(1);
    let mut cfg = Config::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => cfg.listen = args.next()?,
            "--tls-listen" => cfg.tls_listen = args.next()?,
            "--data" => cfg.data_dir = PathBuf::from(args.next()?),
            "--admin-user" => cfg.admin_user = args.next()?,
            "--admin-pass" => cfg.admin_pass = args.next()?,
            "--chunk-size" => {
                if let Ok(n) = args.next()?.parse::<i64>() {
                    cfg.chunk_size = n;
                }
            }
            "-h" | "--help" => {
                usage();
                return None;
            }
            _ => {}
        }
    }
    Some(cfg)
}