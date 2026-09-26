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
        "tg-signer {ver} - Telegram 每日签到工具 (Rust 单二进制，多账号)

用法（命令末尾可加账号编号，默认账号 1）:
  tg-signer login [N]                   登录账号 N（交互式，生成 session）
  tg-signer accounts                    列出所有账号
  tg-signer add @bot /cmd [/cmd2...] [N]  添加签到任务（@用户名 格式）
  tg-signer add @bot button=按钮文本 [N]   添加按钮签到任务（如 button=✍️每日签到）
  tg-signer list [N]                    列出账号 N 的签到任务
  tg-signer rm <n|@bot> [N]             删除账号 N 的任务
  tg-signer test [N]                    立即执行一次账号 N 的签到（调试）
  tg-signer run-once [N]                同 test，供 cron/systemd 触发
  tg-signer run [N]                     常驻运行，每日定时签到（北京时间）
  tg-signer setup-service [N]           安装 systemd 服务+timer（开机自启）
  tg-signer uninstall-service [N]       卸载该账号的 systemd 服务+timer
  tg-signer status [N]                  查看账号 N 的任务/服务/日志

环境变量:
  TG_API_ID / TG_API_HASH   Telegram API 凭证（默认内置公开凭证）
  TG_PROXY                  SOCKS5 代理，如 socks5://127.0.0.1:1080
  TG_SIGN_TIME              每日执行时间 HH:MM（默认 07:00，北京时间）
  TG_CONFIG_DIR             配置目录（默认 ~/.config/tg-signer）

示例:
  tg-signer login                # 登录账号1（老用法不变）
  tg-signer login 2              # 登录账号2
  tg-signer add @AEONSGKBot /qd          # 加到账号1
  tg-signer add @DJXZTbot button=✍️每日签到 2   # 加到账号2
  tg-signer test 2
  sudo tg-signer setup-service 2         # 账号2 独立定时器",
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
    // 末尾的纯数字 = 账号编号（如 `add @bot /qd 1`、`list 2`）
    let (args, account) = config::split_account(args)?;
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");

    match cmd {
        "login" => login::cmd_login(account).await,
        "add" => {
            if args.len() < 3 {
                bail!("用法: tg-signer add @bot /cmd1 [/cmd2...] [账号]  或  tg-signer add @bot button=按钮文本 [账号]");
            }
            sign::cmd_add(account, &args[1], &args[2..]).await
        }
        "list" | "ls" => sign::cmd_list(account),
        "rm" | "remove" | "del" => {
            if args.len() < 2 {
                bail!("用法: tg-signer rm <编号|@bot> [账号]");
            }
            sign::cmd_rm(account, &args[1])
        }
        "test" | "run-once" => sign::cmd_test(account).await,
        "run" => scheduler::cmd_run(account).await,
        "setup-service" => scheduler::cmd_setup_service(account),
        "uninstall-service" => scheduler::cmd_uninstall_service(account),
        "accounts" => sign::cmd_accounts(),
        "status" => cmd_status(account),
        other => bail!("未知命令: {other}，运行 tg-signer help 查看用法"),
    }
}

fn cmd_status(account: u32) -> Result<()> {
    config::migrate_legacy();
    println!("== 账号 {account} | 目录: {} ==", config::account_dir(account).display());
    sign::cmd_list(account)?;

    let suffix = if account == config::DEFAULT_ACCOUNT {
        "tg-signer".to_string()
    } else {
        format!("tg-signer-{account}")
    };
    println!("\n== systemd timer ({suffix}) ==");
    match Command::new("systemctl")
        .args(["--no-pager", "list-timers", &format!("{suffix}.timer")])
        .status()
    {
        Ok(s) if s.success() => {}
        _ => println!("(未安装，运行 sudo tg-signer setup-service {account} 安装)"),
    }

    println!("\n== 最近日志 ==");
    let log = config::log_file(account);
    if log.exists() {
        let out = Command::new("tail").args(["-n", "20"]).arg(&log).output()?;
        print!("{}", String::from_utf8_lossy(&out.stdout));
    } else {
        println!("(暂无日志)");
    }
    Ok(())
}