use std::sync::Arc;
use tokio::sync::Mutex;
use deezchatz_sdk_rust::DeezChatzClient;
use crate::credentials::StoredSession;
use crate::phone;
use crate::storage::SqliteStorage;

pub async fn handle_send_text(
    recipient: String,
    text: String,
    session: Option<&StoredSession>,
    storage: &SqliteStorage,
    client: Arc<Mutex<DeezChatzClient>>,
) -> color_eyre::Result<()> {
    let s = match session {
        Some(s) => s,
        None => color_eyre::eyre::bail!("Not logged in. Please run `register` first."),
    };

    // Normalize recipient
    let country_code = phone::extract_country_code(&s.phone_number).unwrap_or_else(|| "91".to_string());
    let normalized_recipient = if uuid::Uuid::parse_str(&recipient).is_ok() {
        recipient.clone()
    } else {
        phone::normalize_identifier(&recipient, Some(&country_code))
    };

    let mut c = client.lock().await;
    let _receiver = c.connect(&s.user_id, &s.device_id).await?;

    println!("Sending message...");
    let sent = c.send_text_message(&normalized_recipient, &text).await?;

    let canonical_chat_id = sent.recipient_user_id.clone();
    let _ = storage.link_phone_and_user_id(&normalized_recipient, &canonical_chat_id, None);
    let _ = storage.save_message(
        &sent.message_id,
        &canonical_chat_id,
        &s.user_id,
        &text,
        &s.user_id,
    );

    let phone_opt = if normalized_recipient.starts_with('+') {
        Some(normalized_recipient.as_str())
    } else {
        None
    };
    let _ = storage.upsert_chat(&canonical_chat_id, phone_opt, phone_opt, &text, false);

    println!("Message sent successfully. ID: {}", sent.message_id);



    Ok(())
}
