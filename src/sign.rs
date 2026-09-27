//! 任务管理与执行：add / list / rm / test(run-once)，按账号。
//! v0.4.0：结果校验（bot 回复关键词）+ 失败重试 + 失败通知（Saved Messages）。
use crate::click;
use crate::config::{self, Task};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use grammers_client::Client;
use grammers_session::types::PeerRef;
use std::time::Duration;

// ================= 校验与重试 =================

/// 重试次数（含首次），TG_SIGN_RETRIES 可覆盖，clamp 1..5，默认 3
fn max_retries() -> usize {
    std::env::var("TG_SIGN_RETRIES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|n| n.clamp(1, 5))
        .unwrap_or(3)
}

/// 单条动作执行一次（发送/点击 + 等待 + 校验）；校验未命中/超时返回 Err
async fn attempt_once(
    client: &Client,
    peer: PeerRef,
    task: &Task,
    action: &str,
    patterns: Option<&[String]>,
) -> Result<()> {
    // 1) 执行动作，记录发送前时间戳
    let sent_at = Utc::now().timestamp();
    if let Some(btn) = action.strip_prefix("button:") {
        click::click_button(client, peer, btn, task.trigger()).await?;
    } else {
        client.send_message(peer, action).await?;
    }

    // 2) 无校验要求 => 发送成功即成功
    let Some(pats) = patterns else {
        return Ok(());
    };

    // 3) 等待并读取"发送之后"的新回复（最多 6 秒）
    let deadline = Utc::now() + chrono::Duration::seconds(6);
    loop {
        let mut it = client.iter_messages(peer).limit(6);
        while let Ok(Some(msg)) = it.next().await {
            if msg.outgoing() {
                continue; // 跳过自己发的
            }
            if msg.date().timestamp() < sent_at - 5 {
                continue; // 旧消息（减 5 秒容差，避免本机与服务器时钟偏差误判）
            }
            let text = msg.text().trim().to_string();
            if text.is_empty() {
                continue;
            }
            if pats.iter().any(|p| text.contains(p.as_str())) {
                return Ok(()); // 命中关键词
            }
            let snip: String = text.chars().take(80).collect();
            bail!("回复未命中关键词：「{snip}」(expect={pats:?})");
        }
        if Utc::now() >= deadline {
            bail!("6 秒内未收到 bot 回复 (expect={pats:?})");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// 带重试的单动作执行：最多 max_retries() 次
/// 返回 (成功?, 最后一次的说明用于日志)
async fn attempt_with_retry(
    account: u32,
    client: &Client,
    peer: PeerRef,
    task: &Task,
    action: &str,
    patterns: Option<&[String]>,
) -> (bool, String) {
    let total = max_retries();
    let label = if action.starts_with("button:") {
        action.replacen("button:", "点击「", 1) + "」"
    } else {
        format!("发送 {action}")
    };
    let mut last = String::new();
    for i in 1..=total {
        match attempt_once(client, peer, task, action, patterns).await {
            Ok(()) => {
                let m = if i == 1 {
                    format!("✓ {} {label} 成功", task.name)
                } else {
                    format!("✓ {} {label} 成功（第 {i} 次）", task.name)
                };
                return (true, m);
            }
            Err(e) => {
                last = format!("{} {label}: {e:#}", task.name);
            }
        }
        if i < total {
            config::log(
                account,
                &format!("↻ {} {label} 第 {i} 次失败，3 秒后重试（{}/{total}）", task.name, i + 1),
            );
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
    (false, format!("✗ {last}（已重试 {total} 次）"))
}

// ================= 通知 =================

/// 把失败汇总发给自己（Saved Messages），失败不影响主流程
async fn send_failure_notify(
    account: u32,
    client: &Client,
    fails: &[(String, String)],
    total: usize,
) {
    if fails.is_empty() || !config::notify_enabled() {
        return;
    }
    let mut body = format!(
        "⚠️ tg-signer 账号 {account} 签到有失败（{} / {}）\n时间: {}\n",
        fails.len(),
        total,
        Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .format("%Y-%m-%d %H:%M")
    );
    for (name, reason) in fails {
        body.push_str(&format!("\n• {name}: {reason}"));
    }
    let self_peer: PeerRef = grammers_tl_types::types::InputPeerSelf {}.into();
    match client.send_message(self_peer, body.as_str()).await {
        Ok(_) => config::log(account, "🔔 已发送失败通知到 Saved Messages"),
        Err(e) => config::log(account, &format!("通知发送失败: {e}")),
    }
}

// ================= 任务管理 =================

/// 解析 @用户名 -> (chat_id, PeerRef)
async fn resolve(
    client: &Client,
    username: &str,
) -> Result<(i64, PeerRef)> {
    let name = username.trim_start_matches('@');
    let peer = client
        .resolve_username(name)
        .await
        .with_context(|| format!("解析 @{name} 失败（用户名不存在或网络问题）"))?
        .with_context(|| format!("用户名 @{name} 不存在"))?;
    let chat_id = peer.id().bot_api_dialog_id_unchecked();
    let pref = peer
        .to_ref()
        .await
        .map_err(|e| anyhow::anyhow!("获取 @{name} 引用失败: {e}"))?
        .with_context(|| format!("@{name} 引用获取失败"))?;
    Ok((chat_id, pref))
}

pub async fn cmd_add(account: u32, bot: &str, actions: &[String]) -> Result<()> {
    let name = if bot.starts_with('@') {
        bot.to_string()
    } else {
        format!("@{bot}")
    };

    let mut commands = Vec::new();
    let mut button = None;
    let mut expect = None;
    for a in actions {
        if let Some(b) = a.strip_prefix("button=") {
            if b.is_empty() {
                bail!("button= 后面需要跟按钮文本");
            }
            button = Some(b.to_string());
        } else if let Some(e) = a.strip_prefix("expect=") {
            expect = Some(e.to_string());
        } else if a.starts_with('/') {
            commands.push(a.clone());
        } else {
            bail!("无法识别的动作: {a}（命令需以 / 开头，按钮用 button=文本，校验用 expect=关键词）");
        }
    }
    if commands.is_empty() && button.is_none() {
        bail!("至少需要一个动作: /命令 或 button=按钮文本");
    }

    let tg = crate::tg::Tg::connect(account).await?;
    let client = tg.client().clone();
    if !client.is_authorized().await? {
        bail!("账号 {account} 尚未登录，请先运行: tg-signer login {account}");
    }
    let (chat_id, _pref) = resolve(&client, &name).await?;
    tg.close().await;

    let mut cfg = config::load(account)?;
    let task = Task {
        name: name.clone(),
        chat_id,
        commands,
        button,
        expect,
    };
    if let Some(existing) = cfg.tasks.iter_mut().find(|t| t.name.eq_ignore_ascii_case(&name)) {
        *existing = task.clone();
        println!("✅ 已更新账号 {account} 任务 {name} (chat_id={chat_id}): {}", task.action_desc());
    } else {
        cfg.tasks.push(task.clone());
        println!("✅ 已添加账号 {account} 任务 {name} (chat_id={chat_id}): {}", task.action_desc());
    }
    config::save(account, &cfg)?;
    println!("配置: {}", config::tasks_file(account).display());
    Ok(())
}

pub fn cmd_list(account: u32) -> Result<()> {
    let cfg = config::load(account)?;
    if cfg.tasks.is_empty() {
        println!("(账号 {account} 暂无任务) 用法: tg-signer add @bot /cmd {account}");
        return Ok(());
    }
    println!("== 账号 {account} 任务 ==");
    println!("{:<4} {:<24} {:<14} {}", "#", "BOT", "CHAT_ID", "动作");
    for (i, t) in cfg.tasks.iter().enumerate() {
        println!("{:<4} {:<24} {:<14} {}", i + 1, t.name, t.chat_id, t.action_desc());
    }
    Ok(())
}

pub fn cmd_rm(account: u32, key: &str) -> Result<()> {
    let mut cfg = config::load(account)?;
    if cfg.tasks.is_empty() {
        bail!("账号 {account} 没有可删除的任务");
    }
    let idx = if let Ok(n) = key.parse::<usize>() {
        if n == 0 || n > cfg.tasks.len() {
            bail!("编号无效 (1..{})", cfg.tasks.len());
        }
        n - 1
    } else {
        let name = key.trim_start_matches('@');
        cfg
            .tasks
            .iter()
            .position(|t| t.name.trim_start_matches('@').eq_ignore_ascii_case(name))
            .ok_or_else(|| anyhow::anyhow!("未找到任务: {key}"))?
    };
    let removed = cfg.tasks.remove(idx);
    config::save(account, &cfg)?;
    println!("✅ 已删除账号 {account} 任务 {} ({})", removed.name, removed.action_desc());
    Ok(())
}

// ================= 执行 =================

/// 执行全部签到（test / run-once / 定时器共用）：校验+重试+失败通知
pub async fn run_all(account: u32) -> Result<()> {
    let cfg = config::load(account)?;
    if cfg.tasks.is_empty() {
        bail!("账号 {account} 没有任务，先运行: tg-signer add @bot /cmd {account}");
    }
    let tg = crate::tg::Tg::connect(account).await?;
    let client = tg.client().clone();
    if !client.is_authorized().await? {
        bail!("账号 {account} 尚未登录，请先运行: tg-signer login {account}");
    }

    config::log(account, &format!(
        "========== 签到开始（重试 {} 次，校验默认「{}」）==========",
        max_retries(),
        config::DEFAULT_EXPECT
    ));
    let mut ok = 0usize;
    let mut fails: Vec<(String, String)> = Vec::new();

    for task in &cfg.tasks {
        let pats = task.expect_patterns();
        let pref = match resolve(&client, &task.name).await {
            Ok((_, pref)) => pref,
            Err(e) => {
                let reason = format!("解析失败: {e:#}");
                config::log(account, &format!("✗ {} {reason}", task.name));
                fails.push((task.name.clone(), reason));
                continue;
            }
        };

        let mut task_ok = true;
        let mut task_reason = String::new();

        // 文本命令
        for cmd in &task.commands {
            let (success, msg) =
                attempt_with_retry(account, &client, pref, task, cmd, pats.as_deref()).await;
            config::log(account, &msg);
            if !success {
                task_ok = false;
                task_reason = msg;
            }
        }

        // 按钮
        if let Some(btn) = &task.button {
            let action = format!("button:{btn}");
            let (success, msg) =
                attempt_with_retry(account, &client, pref, task, &action, pats.as_deref()).await;
            config::log(account, &msg);
            if !success {
                task_ok = false;
                task_reason = msg;
            }
        }

        if task_ok {
            ok += 1;
        } else {
            fails.push((task.name.clone(), task_reason));
        }
        // 任务间隔
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    config::log(
        account,
        &format!(
            "========== 签到结束: 成功 {} / 失败 {} ==========",
            ok,
            fails.len()
        ),
    );

    // 失败通知（发给自己 Saved Messages）
    if !fails.is_empty() {
        send_failure_notify(account, &client, &fails, cfg.tasks.len()).await;
    }

    tg.close().await;
    if ok == 0 && !fails.is_empty() {
        bail!(
            "账号 {account} 全部任务失败，详见日志 {}",
            config::log_file(account).display()
        );
    }
    Ok(())
}

pub async fn cmd_test(account: u32) -> Result<()> {
    run_all(account).await
}

/// 列出所有账号及其任务数
pub fn cmd_accounts() -> Result<()> {
    let accounts = config::existing_accounts();
    if accounts.is_empty() {
        println!("(暂无账号) 先运行: tg-signer login");
        return Ok(());
    }
    println!("{:<8} {:<10} {}", "账号", "任务数", "session");
    for a in accounts {
        let tasks = config::load(a).map(|c| c.tasks.len()).unwrap_or(0);
        let sess = config::session_file(a);
        let has = if sess.exists() { "✓" } else { "✗" };
        println!("{:<8} {:<10} {} {}", a, tasks, has, sess.display());
    }
    println!("\n用法: 在命令末尾加账号编号，如 tg-signer add @bot /qd 2");
    Ok(())
}