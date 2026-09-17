# deezchatz-cli 💬🦀

[![Rust](https://img.shields.io/badge/Rust-2024_Edition-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![Built with Ratatui](https://img.shields.io/badge/TUI-Ratatui-green.svg)](https://ratatui.rs)
[![Security: Signal Protocol](https://img.shields.io/badge/Security-Signal_Protocol-blueviolet.svg)](https://signal.org/docs/)

The **deezchatz-cli** is a high-performance, terminal-native client for the [DeezChatz](https://github.com/deez-in) secure messaging ecosystem. It offers both an interactive **Terminal User Interface (TUI)** built with [Ratatui](https://ratatui.rs) and a set of scriptable, headless **CLI subcommands** powered by [Clap](https://clap.rs).

Under the hood, it leverages [`deezchatz-sdk-rust`](https://github.com/deez-in/deezchatz-sdk) and `libsignal` to provide true **End-to-End Encryption (E2EE)** using the Signal Protocol (X3DH, Double Ratchet), combined with zero-trust stateless REST authentication (VXEdDSA) and a multi-database encrypted storage engine powered by SQLCipher.

---

## Table of Contents

- [Features](#features)
- [Architecture & Security](#architecture--security)
  - [Cryptographic Primitives](#cryptographic-primitives)
  - [Multi-Database SQLCipher Storage](#multi-database-sqlcipher-storage)
  - [OS Keyring Integration](#os-keyring-integration)
  - [Anti-Forensic Chat Deletion](#anti-forensic-chat-deletion)
- [Installation & Prerequisites](#installation--prerequisites)
  - [System Dependencies](#system-dependencies)
  - [Building from Source](#building-from-source)
- [Usage Modes](#usage-modes)
  - [1. Interactive TUI Mode](#1-interactive-tui-mode)
  - [2. Headless CLI Subcommands](#2-headless-cli-subcommands)
- [Keybindings Reference](#keybindings-reference)
- [Configuration & Environment Variables](#configuration--environment-variables)
- [Storage & File Locations](#storage--file-locations)
- [Logging & Troubleshooting](#logging--troubleshooting)
- [License](#license)

---

## Features

- **Dual Operating Modes**:
  - **Interactive TUI**: A terminal interface adhering to the Elm Architecture with responsive two-pane layout for wide displays and single-pane drill-down for compact terminals.
  - **Headless CLI Subcommands**: Scriptable commands (`register`, `send-text`, `fetch`, `whoami`) designed for automation, CI/CD pipelines, shell scripts, and headless servers.
- **End-to-End Encryption (Signal Protocol)**:
  - Extended Triple Diffie-Hellman (**X3DH**) pre-key protocol for asynchronous session initialization.
  - **Double Ratchet** providing message-by-message forward secrecy and post-compromise security.
  - **VXEdDSA** digital signatures and VRF proofs for all authenticated API requests.
- **Multi-Database Encrypted Storage**:
  - Local SQLite databases fully encrypted using **SQLCipher** (256-bit AES).
  - Isolated per-chat databases prevent cross-thread correlation and facilitate clean message lifecycle management.
- **Hardware/OS-Backed Keyring**:
  - Master database encryption keys and session tokens are generated randomly and protected by the native OS Secret Service / Keyring.
- **OAuth 2.0 PKCE Login**:
  - Browser-based Google authentication with loopback redirect (`http://127.0.0.1:8080/callback`) and PKCE verification.
- **Real-Time & Offline Sync**:
  - Real-time message streaming over MQTT with automatic reconnection.
  - Offline message batch fetching and local persistence.
- **Clean Diagnostics**:
  - Non-blocking rolling file logging prevents log traces from corrupting the Ratatui terminal UI.
- **Unix Man Page**:
  - Automatically generated man page (`deezchatz-cli.1`) produced at build time via `clap_mangen`.

---

## Architecture & Security

`deezchatz-cli` matches the security model of the mobile client (`deezchatz-mobile`):

```
┌─────────────────────────────────────────────────────────────┐
│                       OS Keyring                            │
│        (Secret Service / GNOME Keyring / Keychain)          │
└──────────────────────────────┬──────────────────────────────┘
                               │ Master DB Key
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                      __primary__.db                         │
│           (Encrypted with OS Keyring Master Key)            │
│  - Identity & Signed Pre-Keys                               │
│  - Thread Summaries & Contacts Registry                     │
│  - Per-Chat Encryption Key Registry                         │
└──────────────┬──────────────────────────────┬───────────────┘
               │ Chat Key A                   │ Chat Key B
               ▼                              ▼
┌─────────────────────────────┐┌─────────────────────────────┐
│      chats/<uuid-a>.db      ││      chats/<uuid-b>.db      │
│  - Messages with Alice      ││  - Messages with Bob        │
│  - Double Ratchet Sessions  ││  - Double Ratchet Sessions  │
└─────────────────────────────┘└─────────────────────────────┘
```

### Cryptographic Primitives
- **Curve25519 / Ed25519**: Identity keys, ephemeral keys, and signed pre-keys.
- **X3DH**: Key agreement protocol establishing shared master secrets even when the recipient is offline.
- **Double Ratchet**: Continuous ratcheting of Diffie-Hellman and symmetric KDF chains per message.
- **VXEdDSA**: Verifiable identity-signed payloads for REST calls, avoiding long-lived bearer tokens or cookies.

### Multi-Database SQLCipher Storage
1. **Primary Database (`__primary__.db`)**:
   - Encrypted with a 256-bit key stored in the native OS keyring.
   - Stores user identity keys, one-time pre-keys (OPKs), contacts, chat metadata, and mapping between chats and their dedicated database files.
2. **Per-Chat Database (`chats/<db_id>.db`)**:
   - Each chat thread is assigned a random UUID filename and its own independent 256-bit SQLCipher encryption key.
   - Houses only that contact's messages and Double Ratchet session state.

### OS Keyring Integration
On initial startup, `deezchatz-cli` generates a 256-bit random cryptographic key and commits it to the operating system's credential vault (`keyring` crate with `Secret Service` on Linux, `Keychain` on macOS, `Credential Manager` on Windows). The database key is never written to disk in plaintext.

### Anti-Forensic Chat Deletion
Deleting a chat immediately unregisters the thread from the primary database, purges the per-chat encryption key from the registry, closes the SQLite connection, and removes `<db_id>.db`, `<db_id>.db-wal`, and `<db_id>.db-shm` from the disk.

---

## Installation & Prerequisites

### System Dependencies

`deezchatz-cli` requires OpenSSL, SQLite/SQLCipher build prerequisites, and the D-Bus/Secret-Service libraries for OS keyring integration.

#### Ubuntu / Debian / Pop!_OS:
```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev libdbus-1-dev clang
```

#### Fedora / RHEL:
```bash
sudo dnf install -y gcc pkg-config openssl-devel dbus-devel clang
```

#### Arch Linux:
```bash
sudo pacman -S --needed base-devel pkgconf openssl dbus clang
```

#### macOS:
```bash
brew install openssl pkg-config
```

### Building from Source

Ensure you have a modern Rust toolchain installed (edition 2024 or Rust 1.85+):

```bash
# Clone the repository
git clone https://github.com/deez-in/deezchatz-cli.git
cd deezchatz-cli

# Build release binary (optimized with LTO and binary stripping)
cargo build --release

# The binary will be available at target/release/deezchatz-cli
./target/release/deezchatz-cli --help
```

You can install the binary system-wide or into your Cargo binary directory:
```bash
cargo install --path .
```

---

## Usage Modes

### 1. Interactive TUI Mode

Launch the full interactive terminal interface simply by running the executable without arguments:

```bash
deezchatz-cli
```

- **First Launch (Setup / Login)**: You will be presented with a phone number prompt. Enter your phone number in E.164 format (e.g. `+919876543210`) and press `Enter`. A browser window will open automatically for Google OAuth PKCE authentication. Once approved, the terminal transitions to your chat dashboard.
- **Subsequent Launches**: The client automatically restores your session from the OS keyring, connects to the real-time MQTT broker, and displays your active chats.

#### Adaptive Layout:
- **Wide Terminals ($\ge 80$ columns)**: Dual-pane interface with the conversations sidebar on the left and active chat history + message composer on the right.
- **Compact Terminals ($< 80$ columns)**: Focused single-pane view. Displays the conversation list, and smoothly drills down into the message thread when a chat is opened.

---

### 2. Headless CLI Subcommands

`deezchatz-cli` includes headless subcommands for scripting and automation.

```
Usage: deezchatz-cli [COMMAND]

Commands:
  register   Register a new device with the given phone number
  send-text  Send a text message to a user
  fetch      Fetch offline messages from the server
  whoami     Print information about the currently logged-in user
  help       Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

#### `register`
Registers a new device and associates it with your phone number via Google OAuth PKCE:
```bash
deezchatz-cli register +919876543210
```
This launches your default browser, awaits the OAuth redirect, uploads signed pre-keys to the server, and persists your credentials in the OS keyring.

#### `send-text`
Sends an end-to-end encrypted message to a recipient identified by phone number or UUID:
```bash
# Send to a phone number (E.164 format)
deezchatz-cli send-text +919876543210 "Hello from the terminal!"

# Send to a known DeezChatz user UUID
deezchatz-cli send-text "e38a2022-7901-4be3-8616-e5781a74d2aa" "Meeting starts in 5 minutes."
```
If this is the first message to the contact, `deezchatz-cli` automatically retrieves the recipient's pre-key bundle from the server, performs X3DH key agreement, establishes a Double Ratchet session, and sends the ciphertext over MQTT before cleanly disconnecting.

#### `fetch`
Polls the broker for queued offline messages, decrypts them, and records them to local storage:
```bash
# Fetch and store messages quietly
deezchatz-cli fetch

# Fetch, store, and print received messages as a structured JSON array
deezchatz-cli fetch --print
```

Example JSON output:
```json
[
  {
    "sender": "e38a2022-7901-4be3-8616-e5781a74d2aa",
    "sender_phone": "+919876543210",
    "content": "Hey! Did you get the report?",
    "timestamp": "2026-09-17T09:49:38Z"
  }
]
```

#### `whoami`
Displays details about the active user session saved in your OS keyring:
```bash
deezchatz-cli whoami
```
Output:
```
User ID: e38a2022-7901-4be3-8616-e5781a74d2aa
Device ID: 1a2b3c4d-5e6f-7a8b-9c0d-1e2f3a4b5c6d
Phone: +919876543210
```

---

## Keybindings Reference

### Conversations Pane
| Keybinding | Action |
|---|---|
| `↑` or `k` | Select previous conversation |
| `↓` or `j` | Select next conversation |
| `Enter` / `Tab` / `i` | Open selected conversation and focus message input |
| `n` | Start a new chat (prompt for phone number or User ID) |
| `PageUp` / `PageDown` | Scroll message preview |
| `q` | Quit application |
| `Ctrl+C` | Force exit immediately |

### Active Chat / Message Input Pane
| Keybinding | Action |
|---|---|
| `Enter` | Send message |
| `Esc` or `Tab` | Return focus to conversations sidebar |
| `PageUp` | Scroll up in chat history (5 messages per press) |
| `PageDown` | Scroll down in chat history |
| `End` | Jump to latest message |
| `Backspace` | Delete character from input |
| `Ctrl+C` | Force exit immediately |

### New Chat Dialog
| Keybinding | Action |
|---|---|
| `Enter` | Confirm recipient and open chat |
| `Esc` | Cancel dialog and return to conversations |
| `Backspace` | Edit entered identifier |

### Login Screen
| Keybinding | Action |
|---|---|
| `Enter` | Submit phone number and launch browser login |
| `Backspace` | Edit phone number |
| `Esc` / `Ctrl+C` | Quit application |

---

## Configuration & Environment Variables

`deezchatz-cli` works out of the box with production defaults, but behavior can be customized using environment variables:

| Variable | Description | Default |
|---|---|---|
| `GOOGLE_CLIENT_ID` | Google OAuth Client ID for PKCE browser authentication | Default built-in Client ID |
| `API_BASE_URL` | DeezChatz REST API base URL | SDK Production Endpoint |
| `MQTT_HOST` | Hostname of the MQTT broker | SDK Production Broker |
| `MQTT_PORT` | Port of the MQTT broker | `1883` / `8883` (TLS) |
| `RUST_LOG` | Tracing filter for log output (e.g. `info`, `debug`, `trace`) | `info` |

---

## Storage & File Locations

All local persistent state follows standard OS conventions via the [`directories`](https://crates.io/crates/directories) crate under `in.deez.deezchatz-cli`:

| Operating System | Path |
|---|---|
| **Linux** | `~/.local/share/deezchatz-cli/` |
| **macOS** | `~/Library/Application Support/in.deez.deezchatz-cli/` |
| **Windows** | `%APPDATA%\deez\deezchatz-cli\data\` |

Directory contents:
- `__primary__.db`: Master SQLite database containing identity keys, contact mappings, and encrypted chat database keys (SQLCipher encrypted).
- `chats/<uuid>.db`: Dedicated, independently encrypted SQLite database for each individual chat thread.
- `deezchatz-cli.log`: Dedicated application log file.

---

## Logging & Troubleshooting

To avoid disrupting or corrupting the Ratatui alternate screen buffer, standard output logging is disabled during TUI operation. All logs are directed to `deezchatz-cli.log` inside your data directory.

To inspect logs in real-time while using the TUI:
```bash
tail -f ~/.local/share/deezchatz-cli/deezchatz-cli.log
```

For verbose debug tracing, set `RUST_LOG`:
```bash
RUST_LOG=debug deezchatz-cli
```

### Common Issues

1. **Keyring / Secret Service Error**:
   - Ensure a Secret Service provider (such as `gnome-keyring` or `ksecretservice`) is installed and running if on a Linux desktop.
   - In headless or containerized environments, ensure `dbus-run-session` is available or configure an unlocked secret service daemon.
2. **Browser Fails to Open During Login**:
   - Check the console or log file for the printed OAuth URL (`Opening browser to: ...`) and manually navigate to it in your web browser.

---

## License

Copyright (c) debarkamondal <debarkamondal@gmail.com>

This project is licensed under the **MIT License**. See the [LICENSE](./LICENSE) file for details.
