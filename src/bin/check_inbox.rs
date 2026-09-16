use rusqlite::{Connection, OptionalExtension};

fn main() {
    let key_out = std::process::Command::new("secret-tool")
        .args(&["lookup", "service", "deezchatz-cli", "account", "database_encryption_key"])
        .output()
        .expect("Failed to get key");
    let key = String::from_utf8(key_out.stdout).unwrap().trim().to_string();

    let path = format!("{}/.local/share/deezchatz-cli/__primary__.db", std::env::var("HOME").unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(&format!("PRAGMA key = '{}';", key.replace("'", "''"))).unwrap();

    let mut stmt = conn.prepare("SELECT id, status, retry_count, payload FROM inbox").unwrap();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Vec<u8>>(3)?
        ))
    }).unwrap();

    for r in rows.flatten() {
        println!("ID: {}, Status: {}, Retries: {}", r.0, r.1, r.2);
    }
}
