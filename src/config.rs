//! 配置目录、任务配置文件、日志。
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 内置公开 API 凭证（tg-signer 项目公开值，用户也可用 TG_API_ID/TG_API_HASH 覆盖）
pub const DEFAULT_API_ID: i32 = 611335;
pub const DEFAULT_API_HASH: &str = "d524b414d21f4d37f08684c1df41ac9c";

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
    dirs_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("tg-signer")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

pub fn config_file() -> PathBuf {
    config_dir().join("tasks.json")
}

pub fn session_file() -> PathBuf {
    config_dir().join("account.session")
}

pub fn log_file() -> PathBuf {
    config_dir().join("sign.log")
}

pub fn load() -> Result<Config> {
    let p = config_file();
    if !p.exists() {
        return Ok(Config::default());
    }
    let data = std::fs::read_to_string(&p)
        .with_context(|| format!("读取配置失败: {}", p.display()))?;
    serde_json::from_str(&data).with_context(|| format!("解析配置失败: {}", p.display()))
}

pub fn save(cfg: &Config) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建配置目录失败: {}", dir.display()))?;
    let p = config_file();
    let data = serde_json::to_string_pretty(cfg)?;
    std::fs::write(&p, data).with_context(|| format!("写入配置失败: {}", p.display()))?;
    Ok(())
}

/// 追加日志行（带北京时间时间戳），同时打印到 stdout
pub fn log(msg: &str) {
    let line = format!(
        "[{}] {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        msg
    );
    println!("{line}");
    if let Some(dir) = config_dir().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file())
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

/// 供外部展示路径用
#[allow(dead_code)]
pub fn path_str(p: &Path) -> String {
    p.display().to_string()
}