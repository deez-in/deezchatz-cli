pub mod migrations;

use async_trait::async_trait;
use deezchatz_sdk_rust::error::SdkError;
use deezchatz_sdk_rust::store::{
    InboxEntry, InboxStore, KeyStore, MessageStatus, OutboxEntry, OutboxStore, SessionStore,
};
use rand::Rng;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ChatSummary {
    pub user_id: String,
    pub phone: Option<String>,
    pub display_name: Option<String>,
    pub last_message: String,
    pub last_message_at: u64,
    pub unread_count: u32,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ChatMessage {
    pub id: String,
    pub chat_id: String,
    pub content: String,
    pub sender_id: String,
    pub created_at: u64,
    pub is_outgoing: bool,
    pub status: String,
}

/// Multi-database storage engine mimicking the deezchatz-mobile architecture:
/// - `__primary__.db`: Encrypted using the master OS keyring key. Stores global identity keys,
///   delivery queues, thread summaries, contacts, and the registry of per-chat database credentials.
/// - `chats/<db_id>.db`: Each chat thread is an isolated, independently encrypted SQLite database
///   holding that contact's messages and Double Ratchet session state.
#[derive(Clone)]
pub struct SqliteStorage {
    base_dir: PathBuf,
    primary_conn: Arc<Mutex<Connection>>,
    chat_conns: Arc<Mutex<HashMap<String, Arc<Mutex<Connection>>>>>,
}

impl SqliteStorage {
    /// Opens the primary encrypted SQLite database and initializes the per-chat databases directory.
    pub fn open<P: AsRef<Path>>(base_dir: P, master_encryption_key: &str) -> Result<Self, SdkError> {
        let base_dir = base_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&base_dir)
            .map_err(|e| SdkError::Storage(format!("Failed to create base data dir: {}", e)))?;

        let chats_dir = base_dir.join("chats");
        std::fs::create_dir_all(&chats_dir)
            .map_err(|e| SdkError::Storage(format!("Failed to create chats dir: {}", e)))?;

        let primary_path = base_dir.join("__primary__.db");
        let conn = Connection::open(&primary_path)
            .map_err(|e| SdkError::Storage(format!("Failed to open primary DB: {}", e)))?;

        // Apply SQLCipher encryption key to primary database
        println!("[DB DEBUG] Applying PRAGMA key (length: {} chars) to primary database...", master_encryption_key.len());
        eprintln!("[DB DEBUG] Applying PRAGMA key (length: {} chars) to primary database...", master_encryption_key.len());
        tracing::info!("[DB DEBUG] Applying PRAGMA key to primary database: {}", master_encryption_key);

        let pragma_key = format!("PRAGMA key = '{}';", master_encryption_key.replace('\'', "''"));
        conn.execute_batch(&pragma_key)
            .map_err(|e| SdkError::Storage(format!("Failed to apply primary encryption key: {}", e)))?;

        // Verify primary database encryption integrity
        let _count: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master;", [], |row| row.get(0))
            .map_err(|e| {
                let err_msg = format!("Key verification or corrupted primary DB: {} (Used key: {})", e, master_encryption_key);
                eprintln!("[DB DEBUG] Primary database verification failed: {}", err_msg);
                tracing::error!("[DB DEBUG] Primary database verification failed: {}", err_msg);
                SdkError::Storage(err_msg)
            })?;

        tracing::info!("[DB DEBUG] Primary database opened and verified successfully!");

        migrations::run_primary_migrations(&conn)
            .map_err(|e| SdkError::Storage(format!("Primary migration failed: {}", e)))?;

        Ok(Self {
            base_dir,
            primary_conn: Arc::new(Mutex::new(conn)),
            chat_conns: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Retrieves or lazily creates an encrypted per-chat database connection.
    /// Manages per-chat encryption keys and randomized UUID filenames inside the primary DB registry.
    pub fn get_or_create_chat_conn(&self, chat_id: &str) -> Result<Arc<Mutex<Connection>>, SdkError> {
        let canonical_id = self.get_user_id_by_phone(chat_id).unwrap_or_else(|| chat_id.to_string());
        let mut conns = self.chat_conns.lock().unwrap();
        if let Some(conn) = conns.get(&canonical_id) {
            return Ok(conn.clone());
        }

        // Query primary DB for (db_id, encryption_key)
        let (db_id, encryption_key) = {
            let primary = self.primary_conn.lock().unwrap();
            let mut stmt = primary
                .prepare("SELECT db_id, encryption_key FROM chat_databases WHERE chat_id = ?1")
                .map_err(|e| SdkError::Storage(e.to_string()))?;

            let row = stmt
                .query_row(params![canonical_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .optional()
                .map_err(|e| SdkError::Storage(e.to_string()))?;

            match row {
                Some(creds) => creds,
                None => {
                    // Generate new randomized UUID filename and 256-bit hex SQLCipher key
                    let db_id = uuid::Uuid::new_v4().to_string();
                    let mut rng = rand::thread_rng();
                    let mut random_bytes = [0u8; 32];
                    rng.fill(&mut random_bytes);
                    let encryption_key: String = random_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    primary
                        .execute(
                            "INSERT INTO chat_databases (chat_id, db_id, encryption_key, created_at)
                             VALUES (?1, ?2, ?3, ?4)",
                            params![canonical_id, db_id, encryption_key, now],
                        )
                        .map_err(|e| SdkError::Storage(format!("Failed to register chat DB: {}", e)))?;

                    (db_id, encryption_key)
                }
            }
        };

        // Open the dedicated per-chat encrypted database file
        let chat_path = self.base_dir.join("chats").join(format!("{}.db", db_id));
        let conn = Connection::open(&chat_path)
            .map_err(|e| SdkError::Storage(format!("Failed to open chat DB for {}: {}", canonical_id, e)))?;

        let pragma_key = format!("PRAGMA key = '{}';", encryption_key.replace('\'', "''"));
        conn.execute_batch(&pragma_key)
            .map_err(|e| SdkError::Storage(format!("Failed to apply encryption key for chat DB {}: {}", canonical_id, e)))?;

        let _count: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master;", [], |row| row.get(0))
            .map_err(|e| SdkError::Storage(format!("Key verification or corrupted chat DB for {}: {}", canonical_id, e)))?;

        migrations::run_chat_migrations(&conn)
            .map_err(|e| SdkError::Storage(format!("Chat DB migration failed for {}: {}", canonical_id, e)))?;

        let conn_arc = Arc::new(Mutex::new(conn));
        conns.insert(canonical_id, conn_arc.clone());
        Ok(conn_arc)
    }

    /// Anti-forensic "Nuclear" chat deletion matching the mobile design:
    /// Closes connection, unregisters the encryption key, and deletes the SQLite database files.
    #[allow(dead_code)]
    pub fn delete_chat(&self, chat_id: &str) -> Result<(), SdkError> {
        let db_id = {
            let primary = self.primary_conn.lock().unwrap();
            let mut stmt = primary
                .prepare("SELECT db_id FROM chat_databases WHERE chat_id = ?1")
                .map_err(|e| SdkError::Storage(e.to_string()))?;

            let db_id: Option<String> = stmt
                .query_row(params![chat_id], |r| r.get(0))
                .optional()
                .map_err(|e| SdkError::Storage(e.to_string()))?;

            if let Some(_) = &db_id {
                primary
                    .execute("DELETE FROM chat_databases WHERE chat_id = ?1", params![chat_id])
                    .map_err(|e| SdkError::Storage(e.to_string()))?;
                primary
                    .execute("DELETE FROM chats WHERE user_id = ?1", params![chat_id])
                    .map_err(|e| SdkError::Storage(e.to_string()))?;
            }
            db_id
        };

        // Remove from connection pool
        {
            let mut conns = self.chat_conns.lock().unwrap();
            conns.remove(chat_id);
        }

        // Delete underlying database file and journal/WAL files
        if let Some(id) = db_id {
            let chat_dir = self.base_dir.join("chats");
            let _ = std::fs::remove_file(chat_dir.join(format!("{}.db", id)));
            let _ = std::fs::remove_file(chat_dir.join(format!("{}.db-wal", id)));
            let _ = std::fs::remove_file(chat_dir.join(format!("{}.db-shm", id)));
        }

        Ok(())
    }

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

// ---------------------------------------------------------------------------
// SDK KeyStore Implementation
// ---------------------------------------------------------------------------
#[async_trait]
impl KeyStore for SqliteStorage {
    async fn save_identity_key(
        &self,
        private_key: &[u8; 32],
        public_key: &[u8; 33],
    ) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO identity_keys (id, private_key, public_key) VALUES (1, ?1, ?2)",
            params![private_key.to_vec(), public_key.to_vec()],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn get_identity_key(&self) -> Result<Option<([u8; 32], [u8; 33])>, SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT private_key, public_key FROM identity_keys WHERE id = 1")
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let row = stmt
            .query_row([], |r| {
                let priv_bytes: Vec<u8> = r.get(0)?;
                let pub_bytes: Vec<u8> = r.get(1)?;
                Ok((priv_bytes, pub_bytes))
            })
            .optional()
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        if let Some((priv_v, pub_v)) = row {
            if priv_v.len() == 32 && pub_v.len() == 33 {
                let mut priv_arr = [0u8; 32];
                let mut pub_arr = [0u8; 33];
                priv_arr.copy_from_slice(&priv_v);
                pub_arr.copy_from_slice(&pub_v);
                return Ok(Some((priv_arr, pub_arr)));
            }
        }
        Ok(None)
    }

    async fn save_signed_pre_key(
        &self,
        id: u32,
        private_key: &[u8; 32],
        public_key: &[u8; 33],
    ) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "INSERT OR REPLACE INTO signed_pre_keys (id, private_key, public_key, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, private_key.to_vec(), public_key.to_vec(), now],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn get_signed_pre_key(&self, id: u32) -> Result<Option<([u8; 32], [u8; 33])>, SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT private_key, public_key FROM signed_pre_keys WHERE id = ?1")
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let row = stmt
            .query_row(params![id], |r| {
                let priv_bytes: Vec<u8> = r.get(0)?;
                let pub_bytes: Vec<u8> = r.get(1)?;
                Ok((priv_bytes, pub_bytes))
            })
            .optional()
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        if let Some((priv_v, pub_v)) = row {
            if priv_v.len() == 32 && pub_v.len() == 33 {
                let mut priv_arr = [0u8; 32];
                let mut pub_arr = [0u8; 33];
                priv_arr.copy_from_slice(&priv_v);
                pub_arr.copy_from_slice(&pub_v);
                return Ok(Some((priv_arr, pub_arr)));
            }
        }
        Ok(None)
    }

    async fn save_one_time_pre_keys(
        &self,
        keys: Vec<(u32, [u8; 32], [u8; 33])>,
    ) -> Result<(), SdkError> {
        let mut conn = self.primary_conn.lock().unwrap();
        let tx = conn
            .transaction()
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        for (id, priv_key, pub_key) in keys {
            tx.execute(
                "INSERT OR REPLACE INTO one_time_pre_keys (id, private_key, public_key, consumed)
                 VALUES (?1, ?2, ?3, 0)",
                params![id, priv_key.to_vec(), pub_key.to_vec()],
            )
            .map_err(|e| SdkError::Storage(e.to_string()))?;
        }

        tx.commit().map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn consume_one_time_pre_key(&self, id: u32) -> Result<Option<([u8; 32], [u8; 33])>, SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT private_key, public_key FROM one_time_pre_keys WHERE id = ?1 AND consumed = 0")
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let res = stmt
            .query_row(params![id], |r| {
                let priv_bytes: Vec<u8> = r.get(0)?;
                let pub_bytes: Vec<u8> = r.get(1)?;
                Ok((priv_bytes, pub_bytes))
            })
            .optional()
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        if let Some((priv_v, pub_v)) = res {
            conn.execute(
                "UPDATE one_time_pre_keys SET consumed = 1 WHERE id = ?1",
                params![id],
            )
            .map_err(|e| SdkError::Storage(e.to_string()))?;

            if priv_v.len() == 32 && pub_v.len() == 33 {
                let mut priv_arr = [0u8; 32];
                let mut pub_arr = [0u8; 33];
                priv_arr.copy_from_slice(&priv_v);
                pub_arr.copy_from_slice(&pub_v);
                return Ok(Some((priv_arr, pub_arr)));
            }
        }

        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// SDK SessionStore Implementation (Routes to each dedicated per-chat DB)
// ---------------------------------------------------------------------------
#[async_trait]
impl SessionStore for SqliteStorage {
    async fn save_session(&self, recipient_id: &str, session_data: &[u8]) -> Result<(), SdkError> {
        let chat_conn = self.get_or_create_chat_conn(recipient_id)?;
        let conn = chat_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "INSERT OR REPLACE INTO sessions (key, value, updated_at) VALUES ('ratchet_session', ?1, ?2)",
            params![session_data.to_vec(), now],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn get_session(&self, recipient_id: &str) -> Result<Option<Vec<u8>>, SdkError> {
        let chat_conn = self.get_or_create_chat_conn(recipient_id)?;
        let conn = chat_conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT value FROM sessions WHERE key = 'ratchet_session'")
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let res = stmt
            .query_row([], |r| r.get(0))
            .optional()
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        Ok(res)
    }

    async fn delete_session(&self, recipient_id: &str) -> Result<(), SdkError> {
        let chat_conn = self.get_or_create_chat_conn(recipient_id)?;
        let conn = chat_conn.lock().unwrap();
        conn.execute(
            "DELETE FROM sessions WHERE key = 'ratchet_session'",
            [],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SDK InboxStore Implementation (Primary DB)
// ---------------------------------------------------------------------------
#[async_trait]
impl InboxStore for SqliteStorage {
    async fn save_to_inbox(&self, topic: &str, payload: &[u8]) -> Result<i64, SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "INSERT INTO inbox (topic, payload, received_at, status, retry_count) VALUES (?1, ?2, ?3, 'Pending', 0)",
            params![topic, payload.to_vec(), now],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;

        let id = conn.last_insert_rowid();
        Ok(id)
    }

    async fn mark_inbox_processed(&self, id: i64) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "UPDATE inbox SET status = 'Processed', processed_at = ?2 WHERE id = ?1",
            params![id, now],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn mark_inbox_failed(&self, id: i64, _error: &str) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        conn.execute(
            "UPDATE inbox SET status = 'Failed', retry_count = retry_count + 1 WHERE id = ?1",
            params![id],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn get_pending_inbox(&self) -> Result<Vec<InboxEntry>, SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, topic, payload, received_at, retry_count, processed_at FROM inbox WHERE status = 'Pending'")
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], |r| {
                Ok(InboxEntry {
                    id: r.get(0)?,
                    topic: r.get(1)?,
                    payload: r.get(2)?,
                    received_at: r.get(3)?,
                    status: MessageStatus::Pending,
                    retry_count: r.get(4)?,
                    processed_at: r.get(5)?,
                })
            })
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row.map_err(|e| SdkError::Storage(e.to_string()))?);
        }
        Ok(list)
    }
}

// ---------------------------------------------------------------------------
// SDK OutboxStore Implementation (Primary DB)
// ---------------------------------------------------------------------------
#[async_trait]
impl OutboxStore for SqliteStorage {
    async fn save_to_outbox(
        &self,
        recipient_id: &str,
        message_id: &str,
        topic: &str,
        payload: &[u8],
    ) -> Result<i64, SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "INSERT INTO outbox (recipient_id, message_id, topic, payload, created_at, status, retry_count)
             VALUES (?1, ?2, ?3, ?4, ?5, 'Pending', 0)",
            params![recipient_id, message_id, topic, payload.to_vec(), now],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;

        let id = conn.last_insert_rowid();
        Ok(id)
    }

    async fn mark_outbox_sent(&self, id: i64) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "UPDATE outbox SET status = 'Sent', sent_at = ?2 WHERE id = ?1",
            params![id, now],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn mark_outbox_failed(&self, id: i64, _error: &str) -> Result<(), SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        conn.execute(
            "UPDATE outbox SET status = 'Failed', retry_count = retry_count + 1 WHERE id = ?1",
            params![id],
        )
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn get_pending_outbox(&self) -> Result<Vec<OutboxEntry>, SdkError> {
        let conn = self.primary_conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, recipient_id, message_id, topic, payload, created_at, retry_count, sent_at FROM outbox WHERE status = 'Pending'")
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], |r| {
                Ok(OutboxEntry {
                    id: r.get(0)?,
                    recipient_id: r.get(1)?,
                    message_id: r.get(2)?,
                    topic: r.get(3)?,
                    payload: r.get(4)?,
                    created_at: r.get(5)?,
                    status: MessageStatus::Pending,
                    retry_count: r.get(6)?,
                    sent_at: r.get(7)?,
                })
            })
            .map_err(|e| SdkError::Storage(e.to_string()))?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row.map_err(|e| SdkError::Storage(e.to_string()))?);
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sqlite_storage_traits_and_per_chat_databases() {
        let temp_dir = std::env::temp_dir().join(format!("test_deezchatz_{}", uuid::Uuid::new_v4()));
        let master_key = "master_super_secret_test_key_12345";

        let storage = SqliteStorage::open(&temp_dir, master_key).expect("Failed to open primary DB");

        // 1. Primary KeyStore tests
        let priv_key = [7u8; 32];
        let pub_key = [9u8; 33];
        storage.save_identity_key(&priv_key, &pub_key).await.expect("save id key");

        let retrieved = storage.get_identity_key().await.expect("get id key").expect("key exists");
        assert_eq!(retrieved.0, priv_key);
        assert_eq!(retrieved.1, pub_key);

        // 2. Per-Chat SessionStore tests (Isolated DBs)
        storage.save_session("user_alice", b"alice_session_bytes").await.expect("save alice session");
        storage.save_session("user_bob", b"bob_session_bytes").await.expect("save bob session");

        let alice_sess = storage.get_session("user_alice").await.expect("get session").expect("session exists");
        assert_eq!(alice_sess, b"alice_session_bytes");

        let bob_sess = storage.get_session("user_bob").await.expect("get session").expect("session exists");
        assert_eq!(bob_sess, b"bob_session_bytes");

        // 3. Verify per-chat DB files were created inside chats/ directory
        let chats_dir = temp_dir.join("chats");
        let entries = std::fs::read_dir(&chats_dir).expect("read chats dir");
        let db_files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "db"))
            .collect();
        assert_eq!(db_files.len(), 2, "Expected 2 isolated per-chat DB files");

        // 4. Per-Chat Messages tests
        storage.save_message("msg_1", "user_alice", "user_alice", "Hello Alice!", "my_id").expect("save message");
        storage.save_message("msg_2", "user_bob", "user_bob", "Hello Bob!", "my_id").expect("save message");

        let alice_msgs = storage.get_messages("user_alice", "my_id").expect("get alice messages");
        assert_eq!(alice_msgs.len(), 1);
        assert_eq!(alice_msgs[0].content, "Hello Alice!");

        let bob_msgs = storage.get_messages("user_bob", "my_id").expect("get bob messages");
        assert_eq!(bob_msgs.len(), 1);
        assert_eq!(bob_msgs[0].content, "Hello Bob!");

        // 5. Inbox & Outbox (Primary DB)
        let inbox_id = storage.save_to_inbox("/test/topic", b"ciphertext_123").await.expect("save to inbox");
        let pending_inbox = storage.get_pending_inbox().await.expect("get inbox");
        assert_eq!(pending_inbox.len(), 1);
        assert_eq!(pending_inbox[0].id, inbox_id);

        storage.mark_inbox_processed(inbox_id).await.expect("mark processed");
        let pending_after = storage.get_pending_inbox().await.expect("get inbox");
        assert_eq!(pending_after.len(), 0);

        // 6. Nuclear Chat Deletion test
        storage.delete_chat("user_alice").expect("delete alice chat");
        let remaining_entries = std::fs::read_dir(&chats_dir).expect("read chats dir");
        let remaining_db_files: Vec<_> = remaining_entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "db"))
            .collect();
        assert_eq!(remaining_db_files.len(), 1, "Alice's DB file should have been deleted");

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_three_way_linking_and_chat_merge() {
        let temp_dir = std::env::temp_dir().join(format!("test_merge_{}", uuid::Uuid::new_v4()));
        let master_key = "master_super_secret_test_key_linking";

        let storage = SqliteStorage::open(&temp_dir, master_key).expect("Failed to open primary DB");

        let phone = "+918906620865";
        let user_id = "23589b33-70ca-4b36-9b46-288d653ac0f3";

        // 1. Simulate sending message to phone number
        storage.save_message("msg_sent_1", phone, "my_user_id", "Hello", "my_user_id").expect("save sent message");
        storage.upsert_chat(phone, Some(phone), Some(phone), "Hello", false).expect("upsert phone chat");
        // Save dummy session containing remote_user_id in phone's DB
        let session_json = serde_json::json!({
            "remote_user_id": user_id,
            "remote_device_id": "device_1"
        });
        storage.save_session(phone, &serde_json::to_vec(&session_json).unwrap()).await.expect("save session");

        // 2. Simulate receiving message from UUID
        storage.save_message("msg_recv_1", user_id, user_id, "Hi", "my_user_id").expect("save recv message");
        storage.upsert_chat(user_id, None, None, "Hi", true).expect("upsert uuid chat");

        // Assert 2 chats exist before merging
        let chats_before = storage.get_chats().expect("get chats");
        assert_eq!(chats_before.len(), 2, "Expected 2 split chats before merging");

        // 3. Trigger normalize_existing_chats (startup migration)
        storage.normalize_existing_chats("91").expect("normalize chats");

        // Assert only 1 chat exists after merging
        let chats_after = storage.get_chats().expect("get chats");
        assert_eq!(chats_after.len(), 1, "Expected exactly 1 merged chat");
        assert_eq!(chats_after[0].user_id, user_id, "Chat primary key must be user_id (UUID)");
        assert_eq!(chats_after[0].phone.as_deref(), Some(phone), "Chat phone must be populated");
        assert_eq!(chats_after[0].display_name.as_deref(), Some(phone), "Chat display_name must be populated");

        // Assert messages in user_id DB contain both messages
        let msgs = storage.get_messages(user_id, "my_user_id").expect("get messages by user_id");
        assert_eq!(msgs.len(), 2, "Both sent and received messages must be present");
        let contents: Vec<_> = msgs.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.contains(&"Hello"), "Sent message must be preserved");
        assert!(contents.contains(&"Hi"), "Received message must be preserved");

        // Assert querying via phone alias also resolves to same messages
        let msgs_by_phone = storage.get_messages(phone, "my_user_id").expect("get messages by phone alias");
        assert_eq!(msgs_by_phone.len(), 2, "Alias query must route to canonical DB");

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    #[ignore]
    async fn test_inspect_user_db() {
        let data_dir = std::path::PathBuf::from("/home/dezire/.local/share/deezchatz-cli");
        let master_key = "5ac4c3ced4f7aabb17c1ea1321ee44093c9012ce8282cd9ad472a1ebdfc18049";

        let storage = SqliteStorage::open(&data_dir, master_key).expect("open live db");

        if let Ok(sess) = crate::keyring_store::KeyringManager::get_session() {
            println!("=== KEYRING SESSION: {:?} ===", sess);
        }

        println!("=== CHATS ===");
        let chats = storage.get_chats().expect("get chats");
        for c in &chats {
            println!("Chat: user_id={}, phone={:?}, display_name={:?}, last_msg={}, unread={}", c.user_id, c.phone, c.display_name, c.last_message, c.unread_count);
        }

        println!("=== CONTACTS ===");
        let primary = storage.primary_conn.lock().unwrap();
        let mut c_stmt = primary.prepare("SELECT phone, user_id, name FROM contacts").unwrap();
        let rows = c_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))).unwrap();
        for r in rows.flatten() {
            println!("Contact: phone={}, user_id={}, name={:?}", r.0, r.1, r.2);
        }

        println!("=== CHAT DATABASES ===");
        let mut cd_stmt = primary.prepare("SELECT chat_id, db_id FROM chat_databases").unwrap();
        let cd_rows = cd_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap();
        for r in cd_rows.flatten() {
            println!("Chat DB: chat_id={}, db_id={}", r.0, r.1);
        }

        println!("=== OUTBOX ===");
        let mut o_stmt = primary.prepare("SELECT id, recipient_id, topic, status, retry_count, payload FROM outbox").unwrap();
        let o_rows = o_stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, Vec<u8>>(5)?))).unwrap();
        for r in o_rows.flatten() {
            let json_str = String::from_utf8_lossy(&r.5);
            println!("Outbox: id={}, recipient={}, status={}, payload={}", r.0, r.1, r.3, json_str);
        }

        println!("=== INBOX ===");
        let mut i_stmt = primary.prepare("SELECT id, topic, status, retry_count, payload FROM inbox").unwrap();
        let i_rows = i_stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, Vec<u8>>(4)?))).unwrap();
        for r in i_rows.flatten() {
            let json_str = String::from_utf8_lossy(&r.4);
            println!("Inbox: id={}, topic={}, status={}, payload={}", r.0, r.1, r.2, json_str);
        }

        drop(c_stmt);
        drop(cd_stmt);
        drop(o_stmt);
        drop(i_stmt);
        drop(primary);



        for c in &chats {
            println!("=== MESSAGES FOR {} ===", c.user_id);
            if let Ok(msgs) = storage.get_messages(&c.user_id, "my_id") {
                for m in msgs {
                    println!("Msg: id={}, sender={}, content={}, created_at={}, status={}", m.id, m.sender_id, m.content, m.created_at, m.status);
                }
            }
        }
    }
}

