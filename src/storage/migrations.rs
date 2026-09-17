use rusqlite::{Connection, Result};

/// Migrations for the primary database (__primary__.db).
/// Stores identity keys, queues, contacts, chat list, and the registry of per-chat database credentials.
pub fn run_primary_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        -- Registry of per-chat database encryption keys and filenames
        CREATE TABLE IF NOT EXISTS chat_databases (
            chat_id TEXT PRIMARY KEY NOT NULL,
            db_id TEXT NOT NULL UNIQUE,
            encryption_key TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        -- SDK KeyStore tables (identity and pre-keys)
        CREATE TABLE IF NOT EXISTS identity_keys (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            private_key BLOB NOT NULL,
            public_key BLOB NOT NULL
        );

        CREATE TABLE IF NOT EXISTS signed_pre_keys (
            id INTEGER PRIMARY KEY,
            private_key BLOB NOT NULL,
            public_key BLOB NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS one_time_pre_keys (
            id INTEGER PRIMARY KEY,
            private_key BLOB NOT NULL,
            public_key BLOB NOT NULL,
            consumed INTEGER NOT NULL DEFAULT 0
        );

        -- SDK Inbox queue
        CREATE TABLE IF NOT EXISTS inbox (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            topic TEXT NOT NULL,
            payload BLOB NOT NULL,
            received_at INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'Pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            processed_at INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_inbox_status ON inbox(status);

        -- SDK Outbox queue
        CREATE TABLE IF NOT EXISTS outbox (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            recipient_id TEXT NOT NULL,
            message_id TEXT NOT NULL,
            topic TEXT NOT NULL,
            payload BLOB NOT NULL,
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'Pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            sent_at INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_outbox_status ON outbox(status);

        -- CLI Chat threads list
        CREATE TABLE IF NOT EXISTS chats (
            user_id TEXT PRIMARY KEY NOT NULL,
            phone TEXT,
            display_name TEXT,
            last_message TEXT,
            last_message_at INTEGER NOT NULL,
            unread_count INTEGER DEFAULT 0,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_chats_updated ON chats(updated_at DESC);

        -- Contacts directory
        CREATE TABLE IF NOT EXISTS contacts (
            phone TEXT PRIMARY KEY NOT NULL,
            user_id TEXT NOT NULL UNIQUE,
            name TEXT,
            created_at INTEGER NOT NULL
        );
        "
    )?;

    Ok(())
}

/// Migrations for an individual per-chat database (chats/<db_id>.db).
/// Stores only the messages and Double Ratchet session for that specific conversation.
pub fn run_chat_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY NOT NULL,
            content TEXT,
            sender_id TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            received_at INTEGER,
            status TEXT DEFAULT 'sent',
            type TEXT DEFAULT 'message',
            caption TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at ASC);

        CREATE TABLE IF NOT EXISTS sessions (
            key TEXT PRIMARY KEY NOT NULL,
            value BLOB NOT NULL,
            updated_at INTEGER NOT NULL
        );
        "
    )?;

    Ok(())
}
