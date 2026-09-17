use super::*;
use deezchatz_sdk_rust::store::{InboxStore, KeyStore, SessionStore};

#[tokio::test]
async fn test_sqlite_storage_traits_and_per_chat_databases() {
    let temp_dir = std::env::temp_dir().join(format!("test_deezchatz_{}", uuid::Uuid::new_v4()));
    let master_key = "master_super_secret_test_key_12345";

    let storage = SqliteStorage::open(&temp_dir, master_key).expect("Failed to open primary DB");

    // 1. Primary KeyStore tests
    let priv_key = [7u8; 32];
    let pub_key = [9u8; 33];
    storage.save_identity_key(&priv_key, &pub_key).await.expect("save id key");

    let retrieved = storage.get_identity_key().await.expect("get id key").expect("key exists");
    assert_eq!(retrieved.0, priv_key);
    assert_eq!(retrieved.1, pub_key);

    // 2. Per-Chat SessionStore tests (Isolated DBs)
    storage.save_session("user_alice", b"alice_session_bytes").await.expect("save alice session");
    storage.save_session("user_bob", b"bob_session_bytes").await.expect("save bob session");

    let alice_sess = storage.get_session("user_alice").await.expect("get session").expect("session exists");
    assert_eq!(alice_sess, b"alice_session_bytes");

    let bob_sess = storage.get_session("user_bob").await.expect("get session").expect("session exists");
    assert_eq!(bob_sess, b"bob_session_bytes");

    // 3. Verify per-chat DB files were created inside chats/ directory
    let chats_dir = temp_dir.join("chats");
    let entries = std::fs::read_dir(&chats_dir).expect("read chats dir");
    let db_files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "db"))
        .collect();
    assert_eq!(db_files.len(), 2, "Expected 2 isolated per-chat DB files");

    // 4. Per-Chat Messages tests
    storage.save_message("msg_1", "user_alice", "user_alice", "Hello Alice!", "my_id").expect("save message");
    storage.save_message("msg_2", "user_bob", "user_bob", "Hello Bob!", "my_id").expect("save message");

    let alice_msgs = storage.get_messages("user_alice", "my_id").expect("get alice messages");
    assert_eq!(alice_msgs.len(), 1);
    assert_eq!(alice_msgs[0].content, "Hello Alice!");

    let bob_msgs = storage.get_messages("user_bob", "my_id").expect("get bob messages");
    assert_eq!(bob_msgs.len(), 1);
    assert_eq!(bob_msgs[0].content, "Hello Bob!");

    // 5. Inbox & Outbox (Primary DB)
    let inbox_id = storage.save_to_inbox("/test/topic", b"ciphertext_123").await.expect("save to inbox");
    let pending_inbox = storage.get_pending_inbox().await.expect("get inbox");
    assert_eq!(pending_inbox.len(), 1);
    assert_eq!(pending_inbox[0].id, inbox_id);

    storage.mark_inbox_processed(inbox_id).await.expect("mark processed");
    let pending_after = storage.get_pending_inbox().await.expect("get inbox");
    assert_eq!(pending_after.len(), 0);

    // 6. Nuclear Chat Deletion test
    storage.delete_chat("user_alice").expect("delete alice chat");
    let remaining_entries = std::fs::read_dir(&chats_dir).expect("read chats dir");
    let remaining_db_files: Vec<_> = remaining_entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "db"))
        .collect();
    assert_eq!(remaining_db_files.len(), 1, "Alice's DB file should have been deleted");

    // Clean up
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_three_way_linking_and_chat_merge() {
    let temp_dir = std::env::temp_dir().join(format!("test_merge_{}", uuid::Uuid::new_v4()));
    let master_key = "master_super_secret_test_key_linking";

    let storage = SqliteStorage::open(&temp_dir, master_key).expect("Failed to open primary DB");

    let phone = "+918906620865";
    let user_id = "23589b33-70ca-4b36-9b46-288d653ac0f3";

    // 1. Simulate sending message to phone number
    storage.save_message("msg_sent_1", phone, "my_user_id", "Hello", "my_user_id").expect("save sent message");
    storage.upsert_chat(phone, Some(phone), Some(phone), "Hello", false).expect("upsert phone chat");
    // Save dummy session containing remote_user_id in phone's DB
    let session_json = serde_json::json!({
        "remote_user_id": user_id,
        "remote_device_id": "device_1"
    });
    storage.save_session(phone, &serde_json::to_vec(&session_json).unwrap()).await.expect("save session");

    // 2. Simulate receiving message from UUID
    storage.save_message("msg_recv_1", user_id, user_id, "Hi", "my_user_id").expect("save recv message");
    storage.upsert_chat(user_id, None, None, "Hi", true).expect("upsert uuid chat");

    // Assert 2 chats exist before merging
    let chats_before = storage.get_chats().expect("get chats");
    assert_eq!(chats_before.len(), 2, "Expected 2 split chats before merging");

    // 3. Trigger normalize_existing_chats (startup migration)
    storage.normalize_existing_chats("91").expect("normalize chats");

    // Assert only 1 chat exists after merging
    let chats_after = storage.get_chats().expect("get chats");
    assert_eq!(chats_after.len(), 1, "Expected exactly 1 merged chat");
    assert_eq!(chats_after[0].user_id, user_id, "Chat primary key must be user_id (UUID)");
    assert_eq!(chats_after[0].phone.as_deref(), Some(phone), "Chat phone must be populated");
    assert_eq!(chats_after[0].display_name.as_deref(), Some(phone), "Chat display_name must be populated");

    // Assert messages in user_id DB contain both messages
    let msgs = storage.get_messages(user_id, "my_user_id").expect("get messages by user_id");
    assert_eq!(msgs.len(), 2, "Both sent and received messages must be present");
    let contents: Vec<_> = msgs.iter().map(|m| m.content.as_str()).collect();
    assert!(contents.contains(&"Hello"), "Sent message must be preserved");
    assert!(contents.contains(&"Hi"), "Received message must be preserved");

    // Assert querying via phone alias also resolves to same messages
    let msgs_by_phone = storage.get_messages(phone, "my_user_id").expect("get messages by phone alias");
    assert_eq!(msgs_by_phone.len(), 2, "Alias query must route to canonical DB");

    // Clean up
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
#[ignore]
async fn test_inspect_user_db() {
    let data_dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/deezchatz-cli");
    let master_key = crate::credentials::KeyringManager::get_or_create_db_key().unwrap();

    let storage = SqliteStorage::open(&data_dir, &master_key).expect("open live db");

    if let Ok(sess) = crate::credentials::KeyringManager::get_session() {
        println!("=== KEYRING SESSION: {:?} ===", sess);
    }

    println!("=== CHATS ===");
    let chats = storage.get_chats().expect("get chats");
    for c in &chats {
        println!("Chat: user_id={}, phone={:?}, display_name={:?}, last_msg={}, unread={}", c.user_id, c.phone, c.display_name, c.last_message, c.unread_count);
    }

    println!("=== CONTACTS ===");
    let primary = storage.primary_conn.lock().unwrap();
    let mut c_stmt = primary.prepare("SELECT phone, user_id, name FROM contacts").unwrap();
    let rows = c_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))).unwrap();
    for r in rows.flatten() {
        println!("Contact: phone={}, user_id={}, name={:?}", r.0, r.1, r.2);
    }

    println!("=== CHAT DATABASES ===");
    let mut cd_stmt = primary.prepare("SELECT chat_id, db_id FROM chat_databases").unwrap();
    let cd_rows = cd_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap();
    for r in cd_rows.flatten() {
        println!("Chat DB: chat_id={}, db_id={}", r.0, r.1);
    }

    println!("=== OUTBOX ===");
    let mut o_stmt = primary.prepare("SELECT id, recipient_id, topic, status, retry_count, payload FROM outbox").unwrap();
    let o_rows = o_stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, Vec<u8>>(5)?))).unwrap();
    for r in o_rows.flatten() {
        let json_str = String::from_utf8_lossy(&r.5);
        println!("Outbox: id={}, recipient={}, status={}, payload={}", r.0, r.1, r.3, json_str);
    }

    println!("=== INBOX ===");
    let mut i_stmt = primary.prepare("SELECT id, topic, status, retry_count, payload FROM inbox").unwrap();
    let i_rows = i_stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, Vec<u8>>(4)?))).unwrap();
    for r in i_rows.flatten() {
        let json_str = String::from_utf8_lossy(&r.4);
        println!("Inbox: id={}, topic={}, status={}, payload={}", r.0, r.1, r.2, json_str);
    }

    drop(c_stmt);
    drop(cd_stmt);
    drop(o_stmt);
    drop(i_stmt);
    drop(primary);

    for c in &chats {
        println!("=== MESSAGES FOR {} ===", c.user_id);
        if let Ok(msgs) = storage.get_messages(&c.user_id, "my_id") {
            for m in msgs {
                println!("Msg: id={}, sender={}, content={}, created_at={}, status={}", m.id, m.sender_id, m.content, m.created_at, m.status);
            }
        }
    }
}
