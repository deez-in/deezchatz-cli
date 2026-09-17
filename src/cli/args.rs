use clap::{Parser, Subcommand};

/// CLI for the deezchatz chat ecosystem
#[derive(Parser, Debug)]
#[command(name = "deezchatz-cli")]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Register a new device with the given phone number
    Register {
        /// Phone number in E.164 format (e.g., +1234567890)
        phone_number: String,
    },
    /// Send a text message to a user
    SendText {
        /// Recipient phone number or User ID
        recipient: String,
        /// The message text
        text: String,
    },
    /// Fetch offline messages from the server
    Fetch {
        /// Print newly fetched messages as a JSON array
        #[arg(short, long)]
        print: bool,
    },
    /// Print information about the currently logged-in user
    Whoami,
}
