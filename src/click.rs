//! 点击消息上的 inline 按钮（按按钮文本匹配）。
use anyhow::{bail, Result};
use grammers_client::Client;
use grammers_session::types::PeerRef;
use grammers_tl_types as tl;

/// 在 peer 最近的消息里找到文本等于 `button_text` 的回调按钮并点击。
/// 会先发一条 trigger_text（如 /start）唤出菜单（为空则只翻历史消息）。
pub async fn click_button(
    client: &Client,
    peer: PeerRef,
    button_text: &str,
    trigger: Option<&str>,
) -> Result<()> {
    if let Some(t) = trigger {
        client.send_message(peer, t).await?;
        // 等 bot 返回带按钮的菜单
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    let mut it = client.iter_messages(peer).limit(8);
    let mut found: Option<(i32, Vec<u8>)> = None;
    while let Some(msg) = it.next().await? {
        let Some(markup) = msg.reply_markup() else {
            continue;
        };
        let tl::enums::ReplyMarkup::ReplyInlineMarkup(markup) = markup else {
            continue;
        };
        for row in &markup.rows {
            // KeyboardButtonRow 枚举只有 Row 一个变体
            let tl::enums::KeyboardButtonRow::Row(row) = row;
            for btn in &row.buttons {
                if let tl::enums::KeyboardButton::Callback(cb) = btn {
                    if cb.text == button_text {
                        found = Some((msg.id(), cb.data.clone()));
                        break;
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }

    let Some((msg_id, data)) = found else {
        bail!("未找到按钮「{button_text}」（最近 8 条消息内）");
    };

    client
        .invoke(&tl::functions::messages::GetBotCallbackAnswer {
            game: false,
            peer: peer.into(),
            msg_id,
            data: Some(data),
            password: None,
        })
        .await?;
    Ok(())
}