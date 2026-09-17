use keyring::Entry;
use rand::Rng;
use serde::{Deserialize, Serialize};

const SERVICE_NAME: &str = "deezchatz-cli";
const DB_KEY_USER: &str = "database_encryption_key";
const SESSION_USER: &str = "current_session";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredSession {
    pub user_id: String,
    pub device_id: String,
    pub phone_number: String,
}

pub struct KeyringManager;

impl KeyringManager {
    /// Retrieves or generates a 256-bit random hex string to use as SQLCipher key
    pub fn get_or_create_db_key() -> Result<String, String> {
        let entry = Entry::new(SERVICE_NAME, DB_KEY_USER)
            .map_err(|e| format!("Failed to access keyring: {}", e))?;

        tracing::info!("[KEYRING] Querying OS keyring for existing password (service='{}', user='{}')...", SERVICE_NAME, DB_KEY_USER);

        match entry.get_password() {
            Ok(key) if !key.is_empty() => {
                tracing::info!("[KEYRING] Successfully retrieved existing password from keyring");
                Ok(key)
            }
            err => {
                tracing::info!("[KEYRING] No existing key in keyring (status: {:?}). Generating new key...", err);

                // Generate a 32-byte (256-bit) random key
                let mut rng = rand::thread_rng();
                let mut random_bytes = [0u8; 32];
                rng.fill(&mut random_bytes);
                let new_key: String = random_bytes.iter().map(|b| format!("{:02x}", b)).collect();

                tracing::info!("[KEYRING] Generated password BEFORE storing to keyring");

                entry.set_password(&new_key)
                    .map_err(|e| {
                        let err_msg = format!("Failed to save encryption key to OS keyring: {}", e);
                        tracing::error!("[KEYRING] Error: {}", err_msg);
                        err_msg
                    })?;

                tracing::info!("[KEYRING] Password successfully saved to OS keyring. Now retrieving it to verify...");

                let retrieved = entry.get_password()
                    .map_err(|e| {
                        let err_msg = format!("Failed to retrieve encryption key from OS keyring after storing: {}", e);
                        tracing::error!("[KEYRING] Error: {}", err_msg);
                        err_msg
                    })?;

                tracing::info!("[KEYRING] Retrieved password from OS keyring AFTER storing");

                Ok(retrieved)
            }
        }
    }

    /// Save the active session
    pub fn save_session(session: &StoredSession) -> Result<(), String> {
        let entry = Entry::new(SERVICE_NAME, SESSION_USER)
            .map_err(|e| format!("Failed to access keyring: {}", e))?;

        let json = serde_json::to_string(session)
            .map_err(|e| format!("Serialization error: {}", e))?;

        entry.set_password(&json)
            .map_err(|e| format!("Failed to save session to keyring: {}", e))?;

        Ok(())
    }

    /// Retrieve the stored session, if any
    pub fn get_session() -> Result<Option<StoredSession>, String> {
        let entry = Entry::new(SERVICE_NAME, SESSION_USER)
            .map_err(|e| format!("Failed to access keyring: {}", e))?;

        match entry.get_password() {
            Ok(json) if !json.is_empty() => {
                let session: StoredSession = serde_json::from_str(&json)
                    .map_err(|e| format!("Failed to parse session: {}", e))?;
                Ok(Some(session))
            }
            _ => Ok(None),
        }
    }

    /// Clear session (logout)
    #[allow(dead_code)]
    pub fn clear_session() -> Result<(), String> {
        let entry = Entry::new(SERVICE_NAME, SESSION_USER)
            .map_err(|e| format!("Failed to access keyring: {}", e))?;
        let _ = entry.delete_credential();
        Ok(())
    }
}
