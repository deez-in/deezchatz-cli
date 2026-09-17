use crate::credentials::StoredSession;

pub fn handle_whoami(session: Option<&StoredSession>) -> color_eyre::Result<()> {
    if let Some(s) = session {
        println!("User ID: {}", s.user_id);
        println!("Device ID: {}", s.device_id);
        println!("Phone: {}", s.phone_number);
    } else {
        println!("Not logged in.");
    }
    Ok(())
}
