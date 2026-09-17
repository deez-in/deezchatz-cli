use super::SqliteStorage;
use async_trait::async_trait;
use deezchatz_sdk_rust::error::SdkError;
use deezchatz_sdk_rust::store::{
    InboxEntry, InboxStore, KeyStore, MessageStatus, OutboxEntry, OutboxStore, SessionStore,
};
use rusqlite::{params, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

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
