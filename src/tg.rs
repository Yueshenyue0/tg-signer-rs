//! Telegram 客户端启动与退出。
use anyhow::{Context, Result};
use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use std::sync::Arc;

pub struct Tg {
    pub client: Client,
    handle: grammers_mtsender::SenderPoolFatHandle,
    runner: tokio::task::JoinHandle<()>,
}

impl Tg {
    /// 建立连接并返回客户端（session 持久化在配置目录）
    pub async fn connect() -> Result<Self> {
        let api_id = crate::config::api_id()?;
        let session_path = crate::config::session_file();
        if let Some(dir) = session_path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("创建目录失败: {}", dir.display()))?;
        }
        let session = Arc::new(
            SqliteSession::open(&session_path)
                .await
                .with_context(|| format!("打开 session 失败: {}", session_path.display()))?,
        );

        let pool = match std::env::var("TG_PROXY") {
            Ok(proxy) if !proxy.trim().is_empty() => {
                let mut params = grammers_mtsender::ConnectionParams::default();
                params.proxy_url = Some(proxy.trim().to_string());
                SenderPool::with_configuration(session, api_id, params)
            }
            _ => SenderPool::new(session, api_id),
        };
        let SenderPool {
            runner,
            handle,
            ..
        } = pool;

        let runner_task = tokio::spawn(runner.run());
        let client = Client::new(handle.clone());
        Ok(Self {
            client,
            handle,
            runner: runner_task,
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// 优雅关闭：通知 runner 退出并等待收尾（session 会随 Arc 落盘）
    pub async fn close(self) {
        self.handle.quit();
        let _ = self.runner.await;
    }
}