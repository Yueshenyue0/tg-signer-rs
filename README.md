# tg-signer (Rust)

Telegram 每日签到自动化 —— **单二进制、零运行环境依赖、多账号**。登录账号 → 按 `@用户名` 添加任务 → 定时/开机自动执行，北京时间（固定 +08:00）。

## 特性

- **多账号**：账号用数字编号 1、2、3...，命令末尾加编号即可（默认账号 1，老用法完全兼容）
- `login` 交互式登录（手机号 → 验证码 → 可选两步验证）
- `add @bot /cmd` 按 `@用户名` 配置签到命令，一条任务可多个命令
- `add @bot button=按钮文本` 点击 inline 按钮签到（如 `✍️每日签到`）
- `test` 立即执行一次；`run` 常驻定时；`setup-service` systemd 开机自启
- 每账号独立的 session / 任务 / 日志 / systemd timer
- 支持 SOCKS5 代理（`TG_PROXY`），固定北京时间调度，不依赖系统时区

## 用法

```bash
# 登录（账号编号放命令末尾，不写 = 账号1）
tg-signer login            # 账号1
tg-signer login 2          # 账号2

# 配置任务（编号在最后面）
tg-signer add @AEONSGKBot /qd                  # 账号1
tg-signer add @hh_liemo_bot /checkin           # 账号1
tg-signer add @Kaernet2_bot /sign 2            # 账号2
tg-signer add @DJXZTbot button=✍️每日签到        # 账号1

# 管理
tg-signer accounts         # 列出所有账号
tg-signer list             # 账号1 任务
tg-signer list 2           # 账号2 任务
tg-signer test 2           # 用账号2 测试一次

# 开机自启 + 每日 07:00 自动签到（每账号独立 timer）
sudo tg-signer setup-service        # 账号1 -> tg-signer.timer
sudo tg-signer setup-service 2      # 账号2 -> tg-signer-2.timer
tg-signer status 2
```

常驻进程方式（不用 systemd）：`tg-signer run [N]`

## 配置

| 环境变量 | 说明 | 默认 |
| --- | --- | --- |
| `TG_PROXY` | SOCKS5 代理，如 `socks5://127.0.0.1:1080` | 无 |
| `TG_SIGN_TIME` | 每日执行时间 `HH:MM` | `07:00` |
| `TG_CONFIG_DIR` | 配置目录 | `~/.config/tg-signer` |
| `TG_API_ID` / `TG_API_HASH` | Telegram API 凭证 | 内置公开凭证 |

存储布局（旧版单账号文件自动迁移到 `accounts/1/`）：
```
~/.config/tg-signer/accounts/<N>/{account.session, tasks.json, sign.log}
```

## 构建

本地：`cargo build --release`

CI：推 `v*` 标签自动构建全平台无依赖产物（Linux x86_64/aarch64 musl 静态、Windows 静态 CRT、macOS ARM/x64）并发布 Release。

## License

MIT