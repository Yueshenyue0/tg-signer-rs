//! 配置目录、账号、任务配置文件、日志。
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 内置公开 API 凭证（tg-signer 项目公开值，用户也可用 TG_API_ID/TG_API_HASH 覆盖）
pub const DEFAULT_API_ID: i32 = 611335;
pub const DEFAULT_API_HASH: &str = "d524b414d21f4d37f08684c1df41ac9c";

/// 默认账号编号（不带参数时使用）
pub const DEFAULT_ACCOUNT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// 显示名，如 @AEONSGKBot
    pub name: String,
    /// 解析到的 chat_id
    pub chat_id: i64,
    /// 文本命令列表（如 ["/qd", "/checkin"]）
    #[serde(default)]
    pub commands: Vec<String>,
    /// 按钮文本（点击签到），如 "✍️每日签到"
    #[serde(default)]
    pub button: Option<String>,
}

impl Task {
    pub fn action_desc(&self) -> String {
        let mut parts = Vec::new();
        if !self.commands.is_empty() {
            parts.push(self.commands.join(" "));
        }
        if let Some(b) = &self.button {
            parts.push(format!("点击「{b}」"));
        }
        parts.join(" + ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub tasks: Vec<Task>,
}

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("TG_CONFIG_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("tg-signer")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

/// 账号目录（多账号布局：accounts/<n>/）
pub fn account_dir(account: u32) -> PathBuf {
    config_dir().join("accounts").join(account.to_string())
}

/// 迁移旧版单账号文件到 accounts/1/（向后兼容，幂等）
pub fn migrate_legacy() {
    let base = config_dir();
    let legacy_session = base.join("account.session");
    let legacy_tasks = base.join("tasks.json");
    let legacy_log = base.join("sign.log");
    if !(legacy_session.exists() || legacy_tasks.exists() || legacy_log.exists()) {
        return;
    }
    let dest = account_dir(DEFAULT_ACCOUNT);
    if std::fs::create_dir_all(&dest).is_err() {
        return;
    }
    // 仅当目标不存在时迁移，避免覆盖新布局
    for (src, name) in [
        (legacy_session, "account.session"),
        (legacy_tasks, "tasks.json"),
        (legacy_log, "sign.log"),
    ] {
        let target = dest.join(name);
        if src.exists() && !target.exists() {
            let _ = std::fs::rename(&src, &target);
        } else if src.exists() {
            let _ = std::fs::remove_file(&src);
        }
    }
}

pub fn session_file(account: u32) -> PathBuf {
    account_dir(account).join("account.session")
}

pub fn tasks_file(account: u32) -> PathBuf {
    account_dir(account).join("tasks.json")
}

pub fn log_file(account: u32) -> PathBuf {
    account_dir(account).join("sign.log")
}

/// 已存在的账号编号列表（含 legacy 迁移后的 1）
pub fn existing_accounts() -> Vec<u32> {
    migrate_legacy();
    let mut out = Vec::new();
    let dir = config_dir().join("accounts");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    if let Ok(n) = name.parse::<u32>() {
                        out.push(n);
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out
}

pub fn load(account: u32) -> Result<Config> {
    let p = tasks_file(account);
    if !p.exists() {
        return Ok(Config::default());
    }
    let data = std::fs::read_to_string(&p)
        .with_context(|| format!("读取配置失败: {}", p.display()))?;
    serde_json::from_str(&data).with_context(|| format!("解析配置失败: {}", p.display()))
}

pub fn save(account: u32, cfg: &Config) -> Result<()> {
    let dir = account_dir(account);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建账号目录失败: {}", dir.display()))?;
    let p = tasks_file(account);
    let data = serde_json::to_string_pretty(cfg)?;
    std::fs::write(&p, data).with_context(|| format!("写入配置失败: {}", p.display()))?;
    Ok(())
}

/// 追加日志行（带北京时间时间戳），同时打印到 stdout
pub fn log(account: u32, msg: &str) {
    let line = format!(
        "[{}][账号{}] {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        account,
        msg
    );
    println!("{line}");
    use std::io::Write;
    let dir = account_dir(account);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file(account))
    {
        let _ = writeln!(f, "{line}");
    }
}

/// 每日执行时间 HH:MM（默认 07:00），来自 TG_SIGN_TIME
pub fn sign_time() -> String {
    let t = std::env::var("TG_SIGN_TIME").unwrap_or_default();
    let t = t.trim();
    let ok = t.len() == 5
        && t.as_bytes()[2] == b':'
        && t[..2].parse::<u32>().is_ok()
        && t[3..].parse::<u32>().is_ok()
        && t[..2].parse::<u32>().unwrap() < 24
        && t[3..].parse::<u32>().unwrap() < 60;
    if ok {
        t.to_string()
    } else {
        "07:00".to_string()
    }
}

pub fn api_id() -> Result<i32> {
    match std::env::var("TG_API_ID") {
        Ok(v) if !v.is_empty() => v
            .parse()
            .with_context(|| format!("TG_API_ID 不是合法数字: {v}")),
        _ => Ok(DEFAULT_API_ID),
    }
}

pub fn api_hash() -> Result<String> {
    match std::env::var("TG_API_HASH") {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Ok(DEFAULT_API_HASH.to_string()),
    }
}

/// 校验时间格式，供 setup-service 前检查
pub fn check_sign_time() -> Result<String> {
    let t = sign_time();
    let hh: u32 = t[..2].parse().unwrap_or(99);
    let mm: u32 = t[3..].parse().unwrap_or(99);
    if hh >= 24 || mm >= 60 {
        bail!("TG_SIGN_TIME 格式应为 HH:MM，当前: {t}");
    }
    Ok(t)
}

/// 解析命令行末尾可选的账号编号：Some(剩余args, account)
/// 形如 `add @bot /qd 1` -> (["add","@bot","/qd"], 1)；无数字 -> 默认账号
pub fn split_account(args: &[String]) -> Result<(Vec<String>, u32)> {
    migrate_legacy();
    if let Some(last) = args.last() {
        if let Ok(n) = last.parse::<u32>() {
            if n == 0 {
                bail!("账号编号从 1 开始");
            }
            if args.len() == 1 {
                // 单独一个数字参数也算账号（如 `list 2`）
                return Ok((args[..args.len() - 1].to_vec(), n));
            }
            return Ok((args[..args.len() - 1].to_vec(), n));
        }
    }
    Ok((args.to_vec(), DEFAULT_ACCOUNT))
}

#[allow(dead_code)]
pub fn path_str(p: &Path) -> String {
    p.display().to_string()
}