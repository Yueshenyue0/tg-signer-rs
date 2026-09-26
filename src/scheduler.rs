//! 常驻调度 + systemd 服务安装（按账号编号生成独立单元）。
use crate::config;
use anyhow::{bail, Result};
use chrono::{DateTime, FixedOffset, Local, NaiveTime, TimeZone};
use std::process::Command;

/// 固定 +08:00（北京时间，无夏令时）
fn beijing() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("+08:00")
}

/// 下一个目标时刻（北京时间）：今天 hm 未到则取今天，否则取明天
fn next_run(now: DateTime<FixedOffset>, hm: NaiveTime) -> DateTime<FixedOffset> {
    let today_target = now.date_naive().and_time(hm);
    let now_naive = now.naive_local();
    let target = if now_naive < today_target {
        today_target
    } else {
        today_target + chrono::Duration::days(1)
    };
    now.offset()
        .from_local_datetime(&target)
        .single()
        .unwrap_or(now + chrono::Duration::hours(24))
}

/// 常驻进程：每日固定时间执行指定账号的签到
pub async fn cmd_run(account: u32) -> Result<()> {
    let time_str = config::sign_time();
    let hm = NaiveTime::parse_from_str(&time_str, "%H:%M")
        .map_err(|_| anyhow::anyhow!("TG_SIGN_TIME 格式应为 HH:MM，当前: {time_str}"))?;

    config::log(
        account,
        &format!("调度器启动，每 {time_str}（北京时间）执行签到；时区固定 +08:00"),
    );

    loop {
        let now = Local::now().with_timezone(&beijing());
        let target = next_run(now, hm);
        match target.signed_duration_since(now).to_std() {
            Ok(d) => {
                config::log(
                    account,
                    &format!(
                        "下次执行: {}（等待 {} 分钟）",
                        target.format("%Y-%m-%d %H:%M"),
                        d.as_secs() / 60
                    ),
                );
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        config::log(account, "收到 Ctrl+C，调度器退出");
                        return Ok(());
                    }
                    _ = tokio::time::sleep(d) => {}
                }
            }
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }
        }

        config::log(
            account,
            &format!("== 到达计划时间 {}，开始签到 ==", target.format("%Y-%m-%d %H:%M")),
        );
        if let Err(e) = crate::sign::run_all(account).await {
            config::log(account, &format!("签到执行出错: {e:#}"));
        }
    }
}

// ---------- systemd 服务（每账号独立单元） ----------

fn unit_suffix(account: u32) -> String {
    if account == config::DEFAULT_ACCOUNT {
        "tg-signer".to_string()
    } else {
        format!("tg-signer-{account}")
    }
}

const SERVICE: &str = r#"[Unit]
Description=tg-signer account %ACC% daily sign-in (systemd timer trigger)
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=%EXE% run-once %ACC%
Environment=TZ=Asia/Shanghai

[Install]
WantedBy=multi-user.target
"#;

const TIMER: &str = r#"[Unit]
Description=tg-signer account %ACC% daily sign-in at %TIME% (Asia/Shanghai)

[Timer]
OnCalendar=*-*-* %TIME%:00
Persistent=true
RandomizedDelaySec=30

[Install]
WantedBy=timers.target
"#;

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

/// 安装 systemd service+timer（开机自启 + 每日定时）
pub fn cmd_setup_service(account: u32) -> Result<()> {
    let exe = std::fs::canonicalize("/proc/self/exe")?
        .to_string_lossy()
        .to_string();
    let time = config::check_sign_time()?;
    let suffix = unit_suffix(account);
    let service_path = format!("/etc/systemd/system/{suffix}.service");
    let timer_path = format!("/etc/systemd/system/{suffix}.timer");

    if !is_root() {
        bail!("setup-service 需要 root 权限（写 /etc/systemd/system），请用 sudo 运行");
    }

    let service = SERVICE
        .replace("%EXE%", &exe)
        .replace("%ACC%", &account.to_string());
    let timer = TIMER
        .replace("%TIME%", &time)
        .replace("%ACC%", &account.to_string());

    std::fs::write(&service_path, service)?;
    std::fs::write(&timer_path, timer)?;
    println!("已写入 {service_path}");
    println!("已写入 {timer_path}（账号 {account}，每日 {time} 北京时间）");

    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", &format!("{suffix}.timer")])?;
    let _ = systemctl(&["--no-pager", "list-timers", &format!("{suffix}.timer")]);

    println!("\n✅ 账号 {account} 安装完成：开机自启 + 每日 {time} 自动签到。");
    println!("   查看状态: tg-signer status {account}");
    println!("   卸载:     sudo tg-signer uninstall-service {account}");
    Ok(())
}

/// 卸载 systemd service+timer
pub fn cmd_uninstall_service(account: u32) -> Result<()> {
    let suffix = unit_suffix(account);
    let _ = systemctl(&["disable", "--now", &format!("{suffix}.timer")]);
    let _ = std::fs::remove_file(format!("/etc/systemd/system/{suffix}.service"));
    let _ = std::fs::remove_file(format!("/etc/systemd/system/{suffix}.timer"));
    systemctl(&["daemon-reload"])?;
    println!("✅ 已卸载账号 {account} 的 systemd 服务与定时器。");
    Ok(())
}