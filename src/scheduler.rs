//! 常驻调度：每日固定时间（北京时间）执行签到。
use crate::config;
use anyhow::Result;
use chrono::{DateTime, FixedOffset, Local, NaiveTime, TimeZone};

/// 固定 +08:00（北京时间，无夏令时）
fn beijing() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("+08:00")
}

/// 下一个目标时刻（北京时间）：今天 07:00 未到则取今天，否则取明天
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

pub async fn cmd_run() -> Result<()> {
    let time_str = config::sign_time();
    let hm = NaiveTime::parse_from_str(&time_str, "%H:%M")
        .map_err(|_| anyhow::anyhow!("TG_SIGN_TIME 格式应为 HH:MM，当前: {time_str}"))?;

    config::log(&format!(
        "调度器启动，每 {}（北京时间）执行签到；时区固定 +08:00",
        time_str
    ));

    loop {
        let now = Local::now().with_timezone(&beijing());
        let target = next_run(now, hm);
        let wait = target.signed_duration_since(now).to_std();
        match wait {
            Ok(d) => {
                config::log(&format!(
                    "下次执行: {}（等待 {} 分钟）",
                    target.format("%Y-%m-%d %H:%M"),
                    d.as_secs() / 60
                ));
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        config::log("收到 Ctrl+C，调度器退出");
                        return Ok(());
                    }
                    _ = tokio::time::sleep(d) => {}
                }
            }
            Err(_) => {
                // 理论上不会发生（target 恒在未来），兜底等 1 分钟
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }
        }

        config::log(&format!(
            "== 到达计划时间 {}，开始签到 ==",
            target.format("%Y-%m-%d %H:%M")
        ));
        if let Err(e) = crate::sign::run_all().await {
            config::log(&format!("签到执行出错: {e:#}"));
        }
    }
}