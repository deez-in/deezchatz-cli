use super::{ChatMessage, ChatSummary, SqliteStorage};
use deezchatz_sdk_rust::error::SdkError;
use rusqlite::{params, Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

impl SqliteStorage {
    /// Save or update a chat row
    pub fn upsert_chat(
        &self,
        user_id: &str,
        phone: Option<&str>,
        display_name: Option<&str>,
        last_message: &str,
        increment_unread: bool,
    ) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "INSERT INTO chats (user_id, phone, display_name, last_message, last_message_at, unread_count, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?5)
             ON CONFLICT(user_id) DO UPDATE SET
                phone = COALESCE(?2, chats.phone),
                display_name = COALESCE(?3, chats.display_name),
                last_message = ?4,
                last_message_at = ?5,
                unread_count = CASE WHEN ?7 THEN chats.unread_count + 1 ELSE chats.unread_count END,
                updated_at = ?5",
            params![
                user_id,
                phone,
                display_name,
                last_message,
                now,
                if increment_unread { 1 } else { 0 },
                increment_unread
            ],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Reset unread count for a chat in primary database
    pub fn mark_chat_read(&self, user_id: &str) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        conn.execute(
            "UPDATE chats SET unread_count = 0 WHERE user_id = ?1",
            params![user_id],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Normalizes existing chat IDs in the primary DB (converting un-prefixed phone numbers to E.164
    /// and auto-merging split phone/UUID chat threads matching deezchatz-mobile).
    pub fn normalize_existing_chats(&self, default_calling_code: &str) -> Result<(), SdkError> {
        let chat_ids: Vec<String> = {
            let primary = self.primary_conn.lock().unwrap();
            let mut stmt = primary
                .prepare("SELECT chat_id FROM chat_databases")
                .map_err(|e| SdkError::Storage(e.to_string()))?;

            stmt.query_map([], |row| row.get(0))
                .map_err(|e| SdkError::Storage(e.to_string()))?
                .filter_map(Result::ok)
                .collect()
        };

        for old_id in &chat_ids {
            if !old_id.starts_with('+') && !old_id.contains('@') && uuid::Uuid::parse_str(old_id).is_err() {
                let new_id = crate::phone::normalize_identifier(old_id, Some(default_calling_code));
                if &new_id != old_id && !new_id.is_empty() {
                    tracing::info!("Migrating unnormalized chat {} -> {}", old_id, new_id);

                    let primary = self.primary_conn.lock().unwrap();
                    let exists: bool = primary
                        .query_row(
                            "SELECT COUNT(*) FROM chat_databases WHERE chat_id = ?1",
                            params![new_id],
                            |row| row.get::<_, i64>(0),
                        )
                        .map(|c| c > 0)
                        .unwrap_or(false);

                    if !exists {
                        let _ = primary.execute(
                            "UPDATE chat_databases SET chat_id = ?1 WHERE chat_id = ?2",
                            params![new_id, old_id],
                        );
                        let _ = primary.execute(
                            "UPDATE chats SET user_id = ?1, display_name = ?1 WHERE user_id = ?2",
                            params![new_id, old_id],
                        );
                    } else {
                        let _ = primary.execute(
                            "DELETE FROM chat_databases WHERE chat_id = ?1",
                            params![old_id],
                        );
                        let _ = primary.execute(
                            "DELETE FROM chats WHERE user_id = ?1",
                            params![old_id],
                        );
                    }
                }
            }
        }

        // Auto-merge split phone-number conversations into their canonical user_id (UUID)
        let phone_candidates: Vec<String> = {
            let primary = self.primary_conn.lock().unwrap();
            let mut stmt = primary
                .prepare("SELECT DISTINCT chat_id FROM chat_databases WHERE chat_id LIKE '+%'")
                .map_err(|e| SdkError::Storage(e.to_string()))?;
            stmt.query_map([], |r| r.get(0))
                .map_err(|e| SdkError::Storage(e.to_string()))?
                .filter_map(Result::ok)
                .collect()
        };

        for phone in phone_candidates {
            let target_user_id = self.get_user_id_by_phone(&phone).or_else(|| {
                // Read ratchet_session from phone DB to inspect remote_user_id
                let creds = {
                    let primary = self.primary_conn.lock().unwrap();
                    primary
                        .query_row(
                            "SELECT db_id, encryption_key FROM chat_databases WHERE chat_id = ?1",
                            params![phone],
                            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                        )
                        .optional()
                        .ok()
                        .flatten()
                };
                if let Some((db_id, enc_key)) = creds {
                    let path = self.base_dir.join("chats").join(format!("{}.db", db_id));
                    if let Ok(p_conn) = Connection::open(&path) {
                        let pragma = format!("PRAGMA key = '{}';", enc_key.replace('\'', "''"));
                        if p_conn.execute_batch(&pragma).is_ok() {
                            let session_bytes: Option<Vec<u8>> = p_conn
                                .query_row(
                                    "SELECT value FROM sessions WHERE key = 'ratchet_session'",
                                    [],
                                    |r| r.get(0),
                                )
                                .optional()
                                .ok()
                                .flatten();

                            if let Some(bytes) = session_bytes {
                                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                                    return val.get("remote_user_id")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                }
                            }
                        }
                    }
                }
                None
            });

            if let Some(uid) = target_user_id {
                let _ = self.link_phone_and_user_id(&phone, &uid, None);
            }
        }

        // Also check any chats table entries that are still keyed by phone number
        let raw_phone_chats: Vec<String> = {
            let primary = self.primary_conn.lock().unwrap();
            let mut stmt = primary
                .prepare("SELECT user_id FROM chats WHERE user_id LIKE '+%'")
                .map_err(|e| SdkError::Storage(e.to_string()))?;
            stmt.query_map([], |r| r.get(0))
                .map_err(|e| SdkError::Storage(e.to_string()))?
                .filter_map(Result::ok)
                .collect()
        };

        for phone in raw_phone_chats {
            if let Some(uid) = self.get_user_id_by_phone(&phone) {
                let _ = self.link_phone_and_user_id(&phone, &uid, None);
            }
        }

        Ok(())
    }

    /// Fetch all chats ordered by latest update from primary database
    pub fn get_chats(&self) -> Result<Vec<ChatSummary>, SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT user_id, phone, display_name, last_message, last_message_at, unread_count
                 FROM chats ORDER BY updated_at DESC",
            )
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(ChatSummary {
                    user_id: row.get(0)?,
                    phone: row.get(1)?,
                    display_name: row.get(2)?,
                    last_message: row.get(3)?,
                    last_message_at: row.get(4)?,
                    unread_count: row.get(5)?,
                })
            })
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let mut chats = Vec::new();
        for r in rows {
            chats.push(r.map_err(|e| SdkError::Storage(e.to_string()))?);
        }
        Ok(chats)
    }

    /// Save a chat message to its dedicated per-chat encrypted database
    pub fn save_message(
        &self,
        id: &str,
        chat_id: &str,
        sender_id: &str,
        content: &str,
        my_user_id: &str,
    ) -> Result<(), SdkError> {
        let chat_conn = self.get_or_create_chat_conn(chat_id)?;
        let conn = chat_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let is_outgoing = sender_id == my_user_id;

        conn.execute(
            "INSERT OR REPLACE INTO messages (id, content, sender_id, created_at, received_at, status, type, caption)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5, 'message', NULL)",
            params![id, content, sender_id, now, if is_outgoing { "sent" } else { "received" }],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Get messages for a given chat from its dedicated per-chat database
    pub fn get_messages(&self, chat_id: &str, my_user_id: &str) -> Result<Vec<ChatMessage>, SdkError> {
        let chat_conn = self.get_or_create_chat_conn(chat_id)?;
        let conn = chat_conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, content, sender_id, created_at, status
                 FROM messages ORDER BY created_at ASC",
            )
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let sender: String = row.get(2)?;
                let is_outgoing = sender == my_user_id;
                Ok(ChatMessage {
                    id: row.get(0)?,
                    chat_id: chat_id.to_string(),
                    content: row.get(1)?,
                    sender_id: sender,
                    created_at: row.get(3)?,
                    is_outgoing,
                    status: row.get(4)?,
                })
            })
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let mut msgs = Vec::new();
        for r in rows {
            msgs.push(r.map_err(|e| SdkError::Storage(e.to_string()))?);
        }
        Ok(msgs)
    }
}
