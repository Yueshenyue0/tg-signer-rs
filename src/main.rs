mod click;
mod config;
mod login;
mod scheduler;
mod sign;
mod tg;

use anyhow::{bail, Result};
use std::env;
use std::process::Command;

fn print_help() {
    println!(
        "tg-signer {ver} - Telegram 每日签到工具 (Rust 单二进制)

用法:
  tg-signer login                      登录 Telegram 账号（交互式，生成 session）
  tg-signer add @bot /cmd [/cmd2...]   添加签到任务（按 @用户名 解析）
  tg-signer add @bot button=按钮文本    添加按钮签到任务（如: add @bot button=✍️每日签到）
  tg-signer list                       列出所有签到任务
  tg-signer rm <n|@bot>                删除任务（按编号或 @username）
  tg-signer test                       立即执行一次全部签到（调试用）
  tg-signer run-once                   同 test，供 cron/systemd 手动触发
  tg-signer run                        常驻运行，按计划时间自动签到（北京时间）
  tg-signer setup-service              安装 systemd 服务+timer（开机自启，每日定时执行）
  tg-signer uninstall-service          卸载 systemd 服务+timer
  tg-signer status                     查看服务/任务状态

环境变量:
  TG_API_ID / TG_API_HASH   Telegram API 凭证（默认使用内置公开凭证）
  TG_PROXY                  SOCKS5 代理，如 socks5://127.0.0.1:1080
  TG_SIGN_TIME              每日执行时间 HH:MM（默认 07:00，北京时间）
  TG_CONFIG_DIR             配置目录（默认 ~/.config/tg-signer）

示例:
  tg-signer login
  tg-signer add @AEONSGKBot /qd
  tg-signer add @hh_liemo_bot /checkin
  tg-signer add @Kaernet2_bot /sign
  tg-signer add @DJXZTbot button=✍️每日签到
  tg-signer test
  sudo tg-signer setup-service",
        ver = env!("CARGO_PKG_VERSION")
    );
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print_help();
        return;
    }

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    if let Err(e) = rt.block_on(dispatch(&args)) {
        eprintln!("错误: {e:#}");
        std::process::exit(1);
    }
}

async fn dispatch(args: &[String]) -> Result<()> {
    match args[0].as_str() {
        "login" => login::cmd_login().await,
        "add" => {
            if args.len() < 3 {
                bail!("用法: tg-signer add @bot /cmd1 [/cmd2...]  或  tg-signer add @bot button=按钮文本");
            }
            sign::cmd_add(&args[1], &args[2..]).await
        }
        "list" | "ls" => sign::cmd_list(),
        "rm" | "remove" | "del" => {
            if args.len() < 2 {
                bail!("用法: tg-signer rm <编号|@bot>");
            }
            sign::cmd_rm(&args[1])
        }
        "test" | "run-once" => sign::cmd_test().await,
        "run" => scheduler::cmd_run().await,
        "setup-service" => cmd_setup_service(),
        "uninstall-service" => cmd_uninstall_service(),
        "status" => cmd_status(),
        other => bail!("未知命令: {other}，运行 tg-signer help 查看用法"),
    }
}

const SERVICE: &str = r#"[Unit]
Description=tg-signer daily sign-in (systemd timer trigger)
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=%s run-once
Environment=TZ=Asia/Shanghai

[Install]
WantedBy=multi-user.target
"#;

const TIMER: &str = r#"[Unit]
Description=tg-signer daily sign-in at %TIME% (Asia/Shanghai)

[Timer]
OnCalendar=*-*-* %TIME%:00
Persistent=true
RandomizedDelaySec=30

[Install]
WantedBy=timers.target
"#;

const SERVICE_PATH: &str = "/etc/systemd/system/tg-signer.service";
const TIMER_PATH: &str = "/etc/systemd/system/tg-signer.timer";

fn self_exe() -> Result<String> {
    Ok(std::fs::canonicalize("/proc/self/exe")?
        .to_string_lossy()
        .to_string())
}

fn systemctl(args: &[&str]) -> Result<()> {
    let st = Command::new("systemctl").args(args).status()?;
    if !st.success() {
        bail!("systemctl {} 失败 (exit {:?})", args.join(" "), st.code());
    }
    Ok(())
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata("/proc/self")
            .map(|m| m.uid() == 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn cmd_setup_service() -> Result<()> {
    let exe = self_exe()?;
    let time = config::check_sign_time()?;
    let service = SERVICE.replace("%s", &exe);
    let timer = TIMER.replace("%TIME%", &time);

    if !is_root() {
        bail!("setup-service 需要 root 权限（写 /etc/systemd/system），请用 sudo 运行");
    }

    std::fs::write(SERVICE_PATH, service)?;
    std::fs::write(TIMER_PATH, timer)?;
    println!("已写入 {SERVICE_PATH}");
    println!("已写入 {TIMER_PATH}（每日 {time} 北京时间）");

    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", "tg-signer.timer"])?;
    let _ = systemctl(&["--no-pager", "list-timers", "tg-signer.timer"]);

    println!("\n✅ 安装完成：开机自启 + 每日 {time} 自动签到。");
    println!("   查看状态: tg-signer status");
    println!("   卸载:     sudo tg-signer uninstall-service");
    Ok(())
}

fn cmd_uninstall_service() -> Result<()> {
    let _ = systemctl(&["disable", "--now", "tg-signer.timer"]);
    let _ = std::fs::remove_file(SERVICE_PATH);
    let _ = std::fs::remove_file(TIMER_PATH);
    systemctl(&["daemon-reload"])?;
    println!("✅ 已卸载 tg-signer systemd 服务与定时器。");
    Ok(())
}

fn cmd_status() -> Result<()> {
    println!("== 配置目录: {} ==", config::config_dir().display());
    sign::cmd_list()?;
    println!("\n== systemd timer ==");
    match Command::new("systemctl")
        .args(["--no-pager", "list-timers", "tg-signer.timer"])
        .status()
    {
        Ok(s) if s.success() => {}
        _ => println!("(未安装，运行 sudo tg-signer setup-service 安装)"),
    }
    println!("\n== 最近日志 ==");
    let log = config::log_file();
    if log.exists() {
        let out = Command::new("tail").args(["-n", "20"]).arg(&log).output()?;
        print!("{}", String::from_utf8_lossy(&out.stdout));
    } else {
        println!("(暂无日志)");
    }
    Ok(())
}