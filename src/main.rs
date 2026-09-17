mod auth;
mod cli;
mod credentials;
mod phone;
mod storage;
mod tui;

use clap::Parser;
use cli::Cli;
use credentials::KeyringManager;
use deezchatz_sdk_rust::{ClientConfig, DeezChatzClient};
use storage::SqliteStorage;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    // Setup local data directory
    let proj_dirs = directories::ProjectDirs::from("in", "deez", "deezchatz-cli")
        .expect("Failed to resolve user data directory");
    let data_dir = proj_dirs.data_dir();
    std::fs::create_dir_all(data_dir)?;

    // Setup file-based logging (prevents corrupting terminal UI)
    let file_appender = tracing_appender::rolling::never(data_dir, "deezchatz-cli.log");
    tracing_subscriber::fmt()
        .with_writer(file_appender)
        .with_env_filter(EnvFilter::new("info"))
        .init();

    tracing::info!("Starting DeezChatz CLI. Data dir: {:?}", data_dir);

    // Retrieve or generate SQLCipher encryption key from native OS keyring
    let db_key = KeyringManager::get_or_create_db_key()
        .map_err(|e| color_eyre::eyre::eyre!("Keyring error: {}", e))?;

    let storage = SqliteStorage::open(data_dir, &db_key)
        .map_err(|e| color_eyre::eyre::eyre!("Database error: {}", e))?;

    let session = KeyringManager::get_session().unwrap_or(None);
    if let Some(s) = &session {
        let country_code = phone::extract_country_code(&s.phone_number).unwrap_or_else(|| "91".to_string());
        let _ = storage.normalize_existing_chats(&country_code);
    }

    // Instantiate SDK client
    let sdk_config = ClientConfig::from_env();
    let storage_arc = Arc::new(storage.clone());
    let client = Arc::new(Mutex::new(DeezChatzClient::new(
        sdk_config,
        storage_arc.clone(),
        storage_arc.clone(),
        storage_arc.clone(),
        storage_arc,
    )));

    if let Some(cmd) = cli.command {
        cli::run(cmd, storage, session, client).await?;
        return Ok(());
    }

    tui::run(storage, db_key, session, client).await
}
