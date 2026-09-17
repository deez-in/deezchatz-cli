use std::sync::Arc;
use tokio::sync::Mutex;
use deezchatz_sdk_rust::DeezChatzClient;
use crate::auth;
use crate::credentials::{KeyringManager, StoredSession};

const DEFAULT_GOOGLE_CLIENT_ID: &str =
    "715076094331-7mtbcp2hvd382r645ss6tsri18ekk6om.apps.googleusercontent.com";

pub async fn handle_register(
    phone_number: String,
    client: Arc<Mutex<DeezChatzClient>>,
) -> color_eyre::Result<()> {
    let google_client_id = std::env::var("GOOGLE_CLIENT_ID")
        .unwrap_or_else(|_| DEFAULT_GOOGLE_CLIENT_ID.to_string());

    println!("Opening browser for authentication...");
    let auth = tokio::task::spawn_blocking(move || {
        auth::login_via_browser(&google_client_id, 8080)
    })
    .await?
    .map_err(|e| color_eyre::eyre::eyre!(e))?;

    println!("Registering device with server...");
    let mut c = client.lock().await;
    c.register_with_pkce(
        &auth.code,
        Some(&auth.verifier),
        &auth.redirect_uri,
        &phone_number,
    )
    .await?;

    let user_id = c.user_id().unwrap_or("user_registered").to_string();
    let device_id = c.device_id().unwrap_or("device_registered").to_string();
    let new_session = StoredSession {
        user_id,
        device_id,
        phone_number: phone_number.clone(),
    };
    KeyringManager::save_session(&new_session)
        .map_err(|e| color_eyre::eyre::eyre!(e))?;
    println!("Successfully registered and logged in as {}", phone_number);

    Ok(())
}
