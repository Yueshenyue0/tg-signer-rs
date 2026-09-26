# tg-signer (Rust)

Telegram 每日签到自动化 —— **单二进制、零运行环境依赖**。登录账号 → 按 `@用户名` 添加任务 → 定时/开机自动执行，北京时间（固定 +08:00）。

A single-binary Telegram daily sign-in bot runner. No runtime needed on target machines.

## 功能

- `login` 交互式登录（手机号 → 验证码 → 可选两步验证），生成 session
- `add @bot /cmd` 按 `@用户名` 配置签到命令，支持一条任务多个命令
- `add @bot button=按钮文本` 点击 inline 按钮签到（如 `✍️每日签到`）
- `list` / `rm` 任务管理
- `test` 立即执行一次全部签到
- `run` 常驻进程，每日固定时间执行（默认 07:00 北京时间）
- `setup-service` 一键安装 systemd 服务 + timer（**开机自启**）
- `status` 查看任务、timer 状态与最近日志
- 支持 SOCKS5 代理（`TG_PROXY`），默认北京时间调度，不依赖系统时区

## 用法

```bash
# 1. 登录（生成 session）
./tg-signer login

# 2. 配置任务（@用户名 格式）
./tg-signer add @AEONSGKBot /qd
./tg-signer add @hh_liemo_bot /checkin
./tg-signer add @Kaernet2_bot /sign
./tg-signer add @DJXZTbot button=✍️每日签到
./tg-signer list

# 3. 测试一次
./tg-signer test

# 4. 开机自启 + 每日 07:00 自动签到（systemd）
sudo ./tg-signer setup-service
./tg-signer status
```

常驻进程方式（不用 systemd）：

```bash
./tg-signer run   # 每日 07:00（北京时间）自动签到，Ctrl+C 退出
```

## 配置

| 环境变量 | 说明 | 默认 |
| --- | --- | --- |
| `TG_PROXY` | SOCKS5 代理，如 `socks5://127.0.0.1:1080` | 无 |
| `TG_SIGN_TIME` | 每日执行时间 `HH:MM` | `07:00` |
| `TG_CONFIG_DIR` | 配置目录 | `~/.config/tg-signer` |
| `TG_API_ID` / `TG_API_HASH` | Telegram API 凭证 | 内置公开凭证 |

配置目录内：`account.session`（登录凭证，**勿泄露**）、`tasks.json`（任务）、`sign.log`（日志）。

## 构建

本地：`cargo build --release`

CI：推 `v*` 标签自动构建 Linux(x86_64/aarch64)、Windows、macOS 产物并发布 Release。

## License

MIT