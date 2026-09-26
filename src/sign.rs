//! 任务管理与执行：add / list / rm / test(run-once)，按账号。
use crate::click;
use crate::config::{self, Task};
use anyhow::{bail, Context, Result};
use std::time::Duration;

/// 解析 @用户名 -> (chat_id, PeerRef)
async fn resolve(
    client: &grammers_client::Client,
    username: &str,
) -> Result<(i64, grammers_session::types::PeerRef)> {
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
    for a in actions {
        if let Some(b) = a.strip_prefix("button=") {
            if b.is_empty() {
                bail!("button= 后面需要跟按钮文本");
            }
            button = Some(b.to_string());
        } else if a.starts_with('/') {
            commands.push(a.clone());
        } else {
            bail!("无法识别的动作: {a}（命令需以 / 开头，按钮用 button=文本）");
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
        println!("(账号 {account} 暂无任务) 用法: tg-signer add @bot /cmd {account}  或  tg-signer add @bot button=按钮文本 {account}");
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
        let found = cfg
            .tasks
            .iter()
            .position(|t| t.name.trim_start_matches('@').eq_ignore_ascii_case(name));
        match found {
            Some(i) => i,
            None => bail!("未找到任务: {key}"),
        }
    };
    let removed = cfg.tasks.remove(idx);
    config::save(account, &cfg)?;
    println!("✅ 已删除账号 {account} 任务 {} ({})", removed.name, removed.action_desc());
    Ok(())
}

/// 执行全部签到（test / run-once / 定时器共用）
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

    config::log(account, "========== 签到开始 ==========");
    let mut ok = 0usize;
    let mut fail = 0usize;

    for task in &cfg.tasks {
        let resolved = resolve(&client, &task.name).await;
        let pref = match resolved {
            Ok((_, pref)) => pref,
            Err(e) => {
                config::log(account, &format!("✗ {} 解析失败: {:#}", task.name, e));
                fail += 1;
                continue;
            }
        };

        let mut task_ok = true;
        for cmd in &task.commands {
            match client.send_message(pref, cmd.as_str()).await {
                Ok(m) => config::log(account, &format!(
                    "✓ {} 发送 {} (msg_id={})",
                    task.name, cmd, m.id()
                )),
                Err(e) => {
                    config::log(account, &format!("✗ {} 发送 {} 失败: {e}", task.name, cmd));
                    task_ok = false;
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }

        if let Some(btn) = &task.button {
            let trigger = if task.commands.is_empty() {
                Some("/start")
            } else {
                None
            };
            match click::click_button(&client, pref, btn, trigger).await {
                Ok(()) => config::log(account, &format!("✓ {} 点击「{btn}」成功", task.name)),
                Err(e) => {
                    config::log(account, &format!("✗ {} 点击「{btn}」失败: {e:#}", task.name));
                    task_ok = false;
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }

        if task_ok {
            ok += 1;
        } else {
            fail += 1;
        }
    }

    config::log(account, &format!("========== 签到结束: 成功 {ok} / 失败 {fail} =========="));
    tg.close().await;
    if ok == 0 && fail > 0 {
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