# wp · 多用户网盘

基于 Rust / axum 的多用户网盘服务,支持分片断点上传、在线转码播放、目录浏览权限控制和分享链接。前端为内嵌的纯静态页面(`web/`,经 rust-embed 打包进二进制)。

## 功能特性

- **分片断点上传**:8 MiB 默认分片、可配置;支持查询分片状态、续传、中断与完全合并
- **在线媒体转码与播放**:依赖系统 `ffmpeg`(可通过 `WP_FFMPEG` 环境变量指定路径),提供 Range 流式播放、转码任务状态查询、视频封面抽帧缩略图
- **多用户与权限**:会话登录(SHA-256 令牌)、验证码、修改密码;可按目录配置浏览/上传/下载/删除/建目录等权限
- **分享链接**:文件级分享,随机 token,带访问统计;第三方匿名访问页 `/s/:token`
- **目录浏览**:`/api/browse/:share_id` 浏览挂载目录(支持宿主目录、用户目录、上传目录等),zip 打包下载
- **HTTPS**:管理界面上传/删除 TLS 证书(rustls 热加载),默认监听 443
- **SQLite 存储**:WAL 模式,单文件数据库,自动迁移建表

## 快速开始

需要 Rust 工具链(2018+ edition 2021)。媒体转码功能可选安装 `ffmpeg`。

```bash
cargo build --release
./target/release/wp
```

默认监听 `0.0.0.0:9000`,数据目录 `./wpdata`,初始管理员 `admin` / `admin123`(请尽快修改)。

### 命令行选项

| 选项 | 说明 | 默认值 |
| --- | --- | --- |
| `--listen <addr>` | HTTP 监听地址 | `0.0.0.0:9000` |
| `--tls-listen <addr>` | HTTPS 监听地址(配置证书后启用) | `0.0.0.0:443` |
| `--data <dir>` | 数据目录 | `./wpdata` |
| `--admin-user <name>` | 初始管理员用户名 | `admin` |
| `--admin-pass <pw>` | 初始管理员密码 | `admin123` |
| `--chunk-size <bytes>` | 上传分片大小 | `8388608` |
| `-h, --help` | 帮助 | |

### 环境变量

- `WP_FFMPEG`:ffmpeg 可执行文件路径(默认在 PATH 中查找)
- `RUST_LOG`:日志级别过滤(默认 `wp=info,tower_http=info`)

## 目录结构

```
src/
  main.rs       入口 / lib.rs 服务启动与模块声明
  config.rs     命令行解析与配置
  router.rs     全部 API 路由
  db.rs         SQLite 访问层与建表迁移
  state.rs      运行期状态、目录初始化、启动清理
  auth/ authz/  认证会话、权限模型与校验
  routes/       各模块 HTTP 处理(认证、文件、上传、媒体、分享、TLS、管理)
  media.rs      转码任务与封面抽帧
  stream.rs     Range 文件流
  zip.rs        zip 打包下载
  tls.rs        rustls 证书热加载
  mime.rs       文件类型识别
  testing.rs    测试服务器(spawn_test_server)
web/            前端静态资源(HTML/CSS/JS),随二进制嵌入
tests/          Rust 集成测试(cargo test)
e2e/            Playwright 端到端测试(e2e/run.sh)
wpdata/         运行时数据(默认数据目录)
```

## 测试

```bash
# Rust 集成测试(内部起临时服务器)
cargo test

# 端到端测试(需 node + playwright)
bash e2e/run.sh   # 自动编译、起服务器、生成媒体 fixture 并运行
```

## 主要 API

- 认证:`POST /api/auth/login`、`/api/auth/logout`、`POST /api/auth/password`、`GET /api/captcha`
- 用户/规则管理:`GET|POST /api/users`、`PATCH|DELETE /api/users/:id`、`GET|POST /api/shares`、`/api/shares/:id/rules`
- 文件:`GET /api/browse/:share_id`、`/api/file/meta/:share_id`、`POST /api/mkdir|rename|delete`、`GET|POST /api/zip`
- 上传:`POST /api/upload/init`、`PUT /api/upload/part/:token/:part`、`POST /api/upload/complete/:token`、`GET /api/upload/status/:token` 等
- 媒体:`GET /api/media/:share_id`(Range 流)、`/api/media/status/:share_id`、`/api/media/thumb/:share_id`
- 分享:`GET|POST /api/fileshares`、匿名页 `/s/:token`
- TLS:`GET|PUT|DELETE /api/tls`

完整路由见 `src/router.rs`。