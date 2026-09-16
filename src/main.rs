mod auth;
mod db;
mod keyring_store;
mod model;
mod phone;
mod update;
mod view;

use db::SqliteStorage;
use deezchatz_sdk_rust::{ClientConfig, DeezChatzClient};
use keyring_store::{KeyringManager, StoredSession};
use model::{Cmd, Model, Msg};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing_subscriber::EnvFilter;

const DEFAULT_GOOGLE_CLIENT_ID: &str =
    "715076094331-7mtbcp2hvd382r645ss6tsri18ekk6om.apps.googleusercontent.com";

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

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
    tracing::info!("[CLI/TUI OPENED] Retrieving DB encryption password from OS keyring...");

    let db_key = KeyringManager::get_or_create_db_key()
        .map_err(|e| color_eyre::eyre::eyre!("Keyring error: {}", e))?;

    tracing::info!("[CLI/TUI OPENED] Retrieved password from OS keyring: {}", db_key);

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

    // Channel for Elm Architecture message loop
    let (tx, mut rx) = mpsc::channel::<Msg>(100);

    // Spawn task to forward crossterm keyboard events
    let key_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false) {
                if let Ok(crossterm::event::Event::Key(key)) = crossterm::event::read() {
                    if key.kind == crossterm::event::KeyEventKind::Press {
                        if key_tx.send(Msg::Key(key)).await.is_err() {
                            break;
                        }
                    }
                }
            } else {
                // Yield to Tokio runtime to allow task cancellation during shutdown
                tokio::task::yield_now().await;
            }

            // If the main event loop exited, the receiver is dropped
            if key_tx.is_closed() {
                break;
            }
        }
    });

    // If existing session, auto-connect to MQTT broker
    if let Some(s) = &session {
        let client_clone = client.clone();
        let uid = s.user_id.clone();
        let did = s.device_id.clone();
        let mqtt_tx = tx.clone();
        let storage_clone = storage.clone();
        tokio::spawn(async move {
            let mut c = client_clone.lock().await;
            match c.connect(&uid, &did).await {
                Ok(receiver) => {
                    let _ = mqtt_tx.send(Msg::SetStatus("Connected to broker".into())).await;
                    spawn_mqtt_receiver(receiver, mqtt_tx, client_clone.clone(), storage_clone);
                }
                Err(e) => {
                    let _ = mqtt_tx.send(Msg::SetStatus(format!("Connection error: {}", e))).await;
                }
            }
        });
    }

    tracing::info!("[CLI/TUI LAUNCH] Launching Ratatui terminal UI with DB key: {}", db_key);

    // Initialize Ratatui terminal
    crossterm::terminal::enable_raw_mode()?;
    let mut terminal = ratatui::init();

    let mut model = Model::new(storage, session);
    model.status_message = Some(format!("DB Key: {}", db_key));

    // Main Elm Architecture loop
    while !model.should_quit {
        terminal.draw(|f| view::render(&model, f))?;

        if let Some(msg) = rx.recv().await {
            let cmd = update::update(&mut model, msg);

            match cmd {
                Cmd::None => {}
                Cmd::TriggerLogin { phone_number } => {
                    let login_tx = tx.clone();
                    let client_clone = client.clone();
                    let phone_clone = phone_number.clone();
                    let _ = tx.send(Msg::LoginStarted).await;

                    tokio::spawn(async move {
                        let google_client_id = std::env::var("GOOGLE_CLIENT_ID")
                            .unwrap_or_else(|_| DEFAULT_GOOGLE_CLIENT_ID.to_string());

                        // Run blocking local web server OAuth in a threadpool
                        let oauth_res = tokio::task::spawn_blocking(move || {
                            auth::login_via_browser(&google_client_id, 8080)
                        })
                        .await;

                        match oauth_res {
                            Ok(Ok(auth)) => {
                                let mut c = client_clone.lock().await;
                                match c
                                    .register_with_pkce(
                                        &auth.code,
                                        Some(&auth.verifier),
                                        &auth.redirect_uri,
                                        &phone_clone,
                                    )
                                    .await
                                {
                                    Ok(_) => {
                                        let user_id = c.user_id().unwrap_or("user_registered").to_string();
                                        let device_id = c.device_id().unwrap_or("device_registered").to_string();
                                        let session = StoredSession {
                                            user_id,
                                            device_id,
                                            phone_number: phone_clone,
                                        };
                                        let _ = KeyringManager::save_session(&session);
                                        let _ = login_tx.send(Msg::LoginSuccess(session)).await;
                                    }
                                    Err(e) => {
                                        let _ = login_tx
                                            .send(Msg::LoginFailed(format!("Registration error: {}", e)))
                                            .await;
                                    }
                                }
                            }
                            Ok(Err(e)) => {
                                let _ = login_tx.send(Msg::LoginFailed(e)).await;
                            }
                            Err(e) => {
                                let _ = login_tx
                                    .send(Msg::LoginFailed(format!("Task execution error: {}", e)))
                                    .await;
                            }
                        }
                    });
                }
                Cmd::SendMessage { recipient, text } => {
                    let client_clone = client.clone();
                    let send_tx = tx.clone();
                    let text_clone = text.clone();
                    let recipient_clone = recipient.clone();
                    tokio::spawn(async move {
                        let c = client_clone.lock().await;

                        match c.send_text_message(&recipient_clone, &text_clone).await {
                            Ok(sent) => {
                                let _ = send_tx
                                    .send(Msg::MessageSent {
                                        id: sent.message_id,
                                        recipient: recipient_clone,
                                        recipient_user_id: Some(sent.recipient_user_id),
                                        content: text_clone,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = send_tx
                                    .send(Msg::MessageSendFailed(e.to_string()))
                                    .await;
                            }
                        }
                    });
                }
                Cmd::ConnectMqtt => {
                    if let Some(s) = &model.session {
                        let client_clone = client.clone();
                        let uid = s.user_id.clone();
                        let did = s.device_id.clone();
                        let mqtt_tx = tx.clone();
                        let storage_clone = model.storage.clone();
                        tokio::spawn(async move {
                            let mut c = client_clone.lock().await;
                            if let Ok(receiver) = c.connect(&uid, &did).await {
                                spawn_mqtt_receiver(receiver, mqtt_tx, client_clone.clone(), storage_clone);
                            }
                        });
                    }
                }
            }
        }
    }

    // Clean up and restore terminal
    ratatui::restore();
    crossterm::terminal::disable_raw_mode()?;

    Ok(())
}

fn spawn_mqtt_receiver(
    mut receiver: tokio::sync::mpsc::Receiver<deezchatz_sdk_rust::Event>,
    mqtt_tx: tokio::sync::mpsc::Sender<Msg>,
    client: Arc<tokio::sync::Mutex<DeezChatzClient>>,
    storage: SqliteStorage,
) {
    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            if let deezchatz_sdk_rust::Event::MessageReceived { ref sender, .. } = event {
                // If contact is not yet known or has no phone mapped, resolve profile in background
                if storage.get_phone_by_user_id(sender).is_none() {
                    let client_c = client.clone();
                    let res_tx = mqtt_tx.clone();
                    let uid = sender.clone();
                    tokio::spawn(async move {
                        let c = client_c.lock().await;
                        if let Ok(bundle) = c.get_sync_bundle(&uid).await {
                            let _ = res_tx
                                .send(Msg::ContactResolved {
                                    user_id: uid,
                                    phone: String::new(),
                                    display_name: bundle.display_name,
                                })
                                .await;
                        }
                    });
                }
            }
            if mqtt_tx.send(Msg::Sdk(event)).await.is_err() {
                break;
            }
        }
    });
}
