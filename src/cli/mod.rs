pub mod args;
pub mod commands;

pub use args::{Cli, Commands};

use std::sync::Arc;
use tokio::sync::Mutex;
use deezchatz_sdk_rust::DeezChatzClient;
use crate::credentials::StoredSession;
use crate::storage::SqliteStorage;

pub async fn run(
    cmd: Commands,
    storage: SqliteStorage,
    session: Option<StoredSession>,
    client: Arc<Mutex<DeezChatzClient>>,
) -> color_eyre::Result<()> {
    match cmd {
        Commands::Register { phone_number } => {
            commands::register::handle_register(phone_number, client).await
        }
        Commands::SendText { recipient, text } => {
            commands::send_text::handle_send_text(recipient, text, session.as_ref(), &storage, client).await
        }
        Commands::Fetch { print } => {
            commands::fetch::handle_fetch(print, session.as_ref(), &storage, client).await
        }
        Commands::Whoami => {
            commands::whoami::handle_whoami(session.as_ref())
        }
    }
}
