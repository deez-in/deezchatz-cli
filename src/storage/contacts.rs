use super::SqliteStorage;
use deezchatz_sdk_rust::error::SdkError;
use rusqlite::{params, Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

impl SqliteStorage {
    /// Save or update a contact mapping (phone <-> user_id)
    pub fn save_contact(&self, phone: &str, user_id: &str, name: Option<&str>) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "INSERT INTO contacts (phone, user_id, name, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(phone) DO UPDATE SET
                 user_id = excluded.user_id,
                 name = COALESCE(excluded.name, contacts.name)",
            params![phone, user_id, name, now],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Resolve a user_id (UUID) to a contact phone number if known
    pub fn get_phone_by_user_id(&self, user_id: &str) -> Option<String> {
        let conn = self.primary_conn.lock().unwrap();
        conn.query_row(
            "SELECT phone FROM contacts WHERE user_id = ?1",
            params![user_id],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
    }

    /// Resolve a phone number to a user_id (UUID) if known
    #[allow(dead_code)]
    pub fn get_user_id_by_phone(&self, phone: &str) -> Option<String> {
        let conn = self.primary_conn.lock().unwrap();
        conn.query_row(
            "SELECT user_id FROM contacts WHERE phone = ?1",
            params![phone],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
    }

    /// Three-way identity linking matching deezchatz-mobile:
    /// Binds Phone Number <-> User UUID <-> Contact Name in `contacts` table,
    /// merges any existing split per-chat SQLite databases, and unifies the conversation thread in `chats`.
    pub fn link_phone_and_user_id(
        &self,
        phone: &str,
        user_id: &str,
        display_name: Option<&str>,
    ) -> Result<(), SdkError> {
        if phone == user_id || phone.is_empty() || user_id.is_empty() {
            return Ok(());
        }

        // 1. Save or update mapping in contacts table
        self.save_contact(phone, user_id, display_name)?;

        // 2. Check chat_databases for phone and user_id entries
        let (phone_db_info, user_db_info) = {
            let primary = self.primary_conn.lock().unwrap();
            let get_db = |id: &str| -> Option<(String, String)> {
                primary
                    .query_row(
                        "SELECT db_id, encryption_key FROM chat_databases WHERE chat_id = ?1",
                        params![id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .ok()
                    .flatten()
            };
            (get_db(phone), get_db(user_id))
        };

        // Case A: Both phone and user_id databases exist (the split conversation case)
        if let (Some((phone_db_id, _)), Some((user_db_id, _))) = (&phone_db_info, &user_db_info) {
            if phone_db_id != user_db_id {
                let phone_db_id = phone_db_id.clone();
                tracing::info!(
                    "Merging split chat databases: phone {} (db {}) -> user {} (db {})",
                    phone, phone_db_id, user_id, user_db_id
                );

                // Evict any cached connection for phone
                {
                    let mut conns = self.chat_conns.lock().unwrap();
                    conns.remove(phone);
                }

                // Open raw connection to phone DB to read all messages
                let phone_db_path = self.base_dir.join("chats").join(format!("{}.db", phone_db_id));
                let user_conn = self.get_or_create_chat_conn(user_id)?;

                let (msgs, sessions) = {
                    let phone_key = {
                        let primary = self.primary_conn.lock().unwrap();
                        primary
                            .query_row(
                                "SELECT encryption_key FROM chat_databases WHERE chat_id = ?1",
                                params![phone],
                                |r| r.get::<_, String>(0),
                            )
                            .map_err(|e| SdkError::Storage(e.to_string()))?
                    };

                    let p_conn = Connection::open(&phone_db_path)
                        .map_err(|e| SdkError::Storage(e.to_string()))?;
                    let pragma = format!("PRAGMA key = '{}';", phone_key.replace('\'', "''"));
                    p_conn.execute_batch(&pragma)
                        .map_err(|e| SdkError::Storage(e.to_string()))?;

                    let mut stmt = p_conn
                        .prepare("SELECT id, content, sender_id, created_at, received_at, status, type, caption FROM messages")
                        .map_err(|e| SdkError::Storage(e.to_string()))?;

                    let msgs = stmt
                        .query_map([], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(3)?,
                                row.get::<_, Option<i64>>(4)?,
                                row.get::<_, Option<String>>(5)?,
                                row.get::<_, Option<String>>(6)?,
                                row.get::<_, Option<String>>(7)?,
                            ))
                        })
                        .map_err(|e| SdkError::Storage(e.to_string()))?
                        .filter_map(Result::ok)
                        .collect::<Vec<_>>();

                    let mut session_stmt = p_conn
                        .prepare("SELECT key, value, updated_at FROM sessions")
                        .map_err(|e| SdkError::Storage(e.to_string()))?;

                    let sessions = session_stmt
                        .query_map([], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Vec<u8>>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        })
                        .map_err(|e| SdkError::Storage(e.to_string()))?
                        .filter_map(Result::ok)
                        .collect::<Vec<_>>();

                    (msgs, sessions)
                };

                // Copy into user_conn
                {
                    let mut u_conn = user_conn.lock().unwrap();
                    let tx = u_conn.transaction().map_err(|e| SdkError::Storage(e.to_string()))?;
                    for (id, content, sender_id, created_at, received_at, status, mtype, caption) in msgs {
                        let _ = tx.execute(
                            "INSERT OR IGNORE INTO messages (id, content, sender_id, created_at, received_at, status, type, caption)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                            params![id, content, sender_id, created_at, received_at, status, mtype, caption],
                        );
                    }
                    for (key, val, updated_at) in sessions {
                        let _ = tx.execute(
                            "INSERT OR IGNORE INTO sessions (key, value, updated_at) VALUES (?1, ?2, ?3)",
                            params![key, val, updated_at],
                        );
                    }
                    tx.commit().map_err(|e| SdkError::Storage(e.to_string()))?;
                }

                // Delete the redundant phone database file and registry entry
                let _ = self.delete_chat(phone);
            }
        } else if let (Some(_), None) = (&phone_db_info, &user_db_info) {
            // Case B: Only phone database exists -> Re-key chat_databases from phone to user_id
            let primary = self.primary_conn.lock().unwrap();
            let _ = primary.execute(
                "UPDATE chat_databases SET chat_id = ?1 WHERE chat_id = ?2",
                params![user_id, phone],
            );
            let mut conns = self.chat_conns.lock().unwrap();
            if let Some(conn) = conns.remove(phone) {
                conns.insert(user_id.to_string(), conn);
            }
        }

        // 3. Update primary DB `chats` table: merge rows
        {
            let primary = self.primary_conn.lock().unwrap();

            let phone_chat: Option<(Option<String>, Option<String>, String, i64, i64)> = primary
                .query_row(
                    "SELECT phone, display_name, last_message, last_message_at, unread_count FROM chats WHERE user_id = ?1",
                    params![phone],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()
                .unwrap_or(None);

            let user_chat: Option<(Option<String>, Option<String>, String, i64, i64)> = primary
                .query_row(
                    "SELECT phone, display_name, last_message, last_message_at, unread_count FROM chats WHERE user_id = ?1",
                    params![user_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()
                .unwrap_or(None);

            let _ = primary.execute("DELETE FROM chats WHERE user_id = ?1", params![phone]);

            let (merged_last_msg, merged_last_at, merged_unread, resolved_name) = match (&phone_chat, &user_chat) {
                (Some(p), Some(u)) => {
                    let (last_msg, last_at) = if p.3 >= u.3 {
                        (p.2.clone(), p.3)
                    } else {
                        (u.2.clone(), u.3)
                    };
                    let unread = p.4 + u.4;
                    let name = display_name
                        .map(|s| s.to_string())
                        .or_else(|| p.1.clone())
                        .or_else(|| u.1.clone())
                        .unwrap_or_else(|| phone.to_string());
                    (last_msg, last_at, unread, name)
                }
                (Some(p), None) => {
                    let name = display_name
                        .map(|s| s.to_string())
                        .or_else(|| p.1.clone())
                        .unwrap_or_else(|| phone.to_string());
                    (p.2.clone(), p.3, p.4, name)
                }
                (None, Some(u)) => {
                    let name = display_name
                        .map(|s| s.to_string())
                        .or_else(|| u.1.clone())
                        .unwrap_or_else(|| phone.to_string());
                    (u.2.clone(), u.3, u.4, name)
                }
                (None, None) => (
                    "Conversation started".to_string(),
                    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64,
                    0,
                    display_name.unwrap_or(phone).to_string(),
                ),
            };

            let _ = primary.execute(
                "INSERT INTO chats (user_id, phone, display_name, last_message, last_message_at, unread_count, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?5)
                 ON CONFLICT(user_id) DO UPDATE SET
                    phone = excluded.phone,
                    display_name = excluded.display_name,
                    last_message = excluded.last_message,
                    last_message_at = excluded.last_message_at,
                    unread_count = excluded.unread_count,
                    updated_at = excluded.updated_at",
                params![
                    user_id,
                    phone,
                    resolved_name,
                    merged_last_msg,
                    merged_last_at,
                    merged_unread
                ],
            );
        }

        Ok(())
    }
}
