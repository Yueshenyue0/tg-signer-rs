//! 交互式登录（手机号 -> 验证码 -> 可选 2FA），支持指定账号编号。
use anyhow::{bail, Context, Result};
use grammers_client::SignInError;
use std::io::{self, Write};

fn prompt(msg: &str) -> Result<String> {
    print!("{msg}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub async fn cmd_login(account: u32) -> Result<()> {
    let api_hash = crate::config::api_hash()?;
    let tg = crate::tg::Tg::connect(account).await?;
    let client = tg.client().clone();

    if client.is_authorized().await? {
        let me = client.get_me().await?;
        println!(
            "✅ 账号 {account} 已登录: {} (id={})",
            me.first_name().unwrap_or("?"),
            me.id()
        );
        println!("session: {}", crate::config::session_file(account).display());
        tg.close().await;
        return Ok(());
    }

    println!(">> 登录账号 {account}");
    let phone = prompt("输入手机号（国际格式，如 +15808467917）: ")?;
    if phone.is_empty() {
        bail!("手机号不能为空");
    }
    let confirm = prompt(&format!("确认手机号 {phone} ? (y/N): "))?;
    if !matches!(confirm.to_lowercase().as_str(), "y" | "yes") {
        bail!("已取消");
    }

    let token = client
        .request_login_code(&phone, &api_hash)
        .await
        .context("请求验证码失败（检查手机号/网络/代理 TG_PROXY）")?;
    println!("验证码已发送，请查收 Telegram");

    let code = prompt("输入验证码: ")?;
    if code.is_empty() {
        bail!("验证码不能为空");
    }

    match client.sign_in(&token, &code).await {
        Ok(user) => {
            println!(
                "✅ 账号 {account} 登录成功: {} (id={})",
                user.first_name().unwrap_or("?"),
                user.id()
            );
        }
        Err(SignInError::PasswordRequired(pw_token)) => {
            let hint = pw_token.hint().unwrap_or("无提示");
            println!("该账号开启两步验证 (提示: {hint})");
            let password = prompt("输入两步验证密码: ")?;
            let user = client
                .check_password(pw_token, password.as_bytes())
                .await
                .context("两步验证密码错误")?;
            println!(
                "✅ 账号 {account} 登录成功: {} (id={})",
                user.first_name().unwrap_or("?"),
                user.id()
            );
        }
        Err(SignInError::InvalidCode) => bail!("验证码错误，请重试: tg-signer login {account}"),
        Err(SignInError::SignUpRequired) => bail!("该手机号未注册，请先用官方客户端注册"),
        Err(SignInError::InvalidPassword(_)) => bail!("两步验证密码错误"),
        Err(SignInError::Other(e)) => return Err(e.into()),
    }

    println!("session: {}", crate::config::session_file(account).display());
    tg.close().await;
    Ok(())
}