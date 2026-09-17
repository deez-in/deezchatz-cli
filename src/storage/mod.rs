pub mod migrations;
mod chats;
mod contacts;
mod stores;

#[cfg(test)]
mod tests;

use deezchatz_sdk_rust::error::SdkError;
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
    pub(crate) base_dir: PathBuf,
    pub(crate) primary_conn: Arc<Mutex<Connection>>,
    pub(crate) chat_conns: Arc<Mutex<HashMap<String, Arc<Mutex<Connection>>>>>,
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
}
