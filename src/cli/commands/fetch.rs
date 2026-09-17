use std::sync::Arc;
use tokio::sync::Mutex;
use deezchatz_sdk_rust::DeezChatzClient;
use crate::credentials::StoredSession;
use crate::storage::SqliteStorage;

pub async fn handle_fetch(
    print: bool,
    session: Option<&StoredSession>,
    storage: &SqliteStorage,
    client: Arc<Mutex<DeezChatzClient>>,
) -> color_eyre::Result<()> {
    let s = match session {
        Some(s) => s,
        None => color_eyre::eyre::bail!("Not logged in. Please run `register` first."),
    };

    let mut c = client.lock().await;

    println!("Connecting to broker...");
    let mut receiver = c.connect(&s.user_id, &s.device_id).await?;
    println!("Connected. Fetching offline messages...");

    let mut new_messages = Vec::new();
    let start_time = std::time::Instant::now();
    let max_duration = std::time::Duration::from_secs(10);
    let idle_duration = std::time::Duration::from_secs(3);

    loop {
        if start_time.elapsed() >= max_duration {
            break;
        }

        match tokio::time::timeout(idle_duration, receiver.recv()).await {
            Ok(Some(event)) => {
                if let deezchatz_sdk_rust::Event::MessageReceived { sender, plaintext } = event {
                    let text = match deezchatz_sdk_rust::decode_payload(&plaintext) {
                        Ok(deezchatz_sdk_rust::DecodedPayload::Text { text }) => text,
                        Ok(deezchatz_sdk_rust::DecodedPayload::Voice { .. }) => "🎤 Voice message".to_string(),
                        Ok(deezchatz_sdk_rust::DecodedPayload::Image { caption, .. }) => {
                            if caption.trim().is_empty() {
                                "📷 Photo".to_string()
                            } else {
                                format!("📷 Photo: {}", caption)
                            }
                        }
                        Err(_) => String::from_utf8_lossy(&plaintext).to_string(),
                    };

                    let msg_id = uuid::Uuid::new_v4().to_string();
                    let phone = storage.get_phone_by_user_id(&sender);
                    let _ = storage.save_message(&msg_id, &sender, &sender, &text, &s.user_id);
                    let _ = storage.upsert_chat(&sender, phone.as_deref(), phone.as_deref(), &text, true);

                    new_messages.push(serde_json::json!({
                        "sender": sender,
                        "sender_phone": phone,
                        "content": text,
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    }));
                }
            }
            Ok(None) => break,
            Err(_) => break, // Idle timeout
        }
    }

    println!("Fetch complete. Received {} new messages.", new_messages.len());
    if print && !new_messages.is_empty() {
        println!("{}", serde_json::to_string_pretty(&new_messages)?);
    }

    Ok(())
}
