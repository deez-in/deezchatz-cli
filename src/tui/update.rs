use super::model::{Cmd, FocusedPane, Model, Msg, Screen};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use deezchatz_sdk_rust::Event as SdkEvent;

pub fn update(model: &mut Model, msg: Msg) -> Cmd {
    match msg {
        Msg::Quit => {
            model.should_quit = true;
            Cmd::None
        }
        Msg::SetStatus(status) => {
            model.status_message = Some(status);
            Cmd::None
        }
        Msg::ClearError => {
            model.error_message = None;
            Cmd::None
        }
        Msg::RefreshChats => {
            model.reload_chats();
            Cmd::None
        }
        Msg::LoginStarted => {
            model.is_logging_in = true;
            model.status_message = Some("Opening browser for authentication...".to_string());
            Cmd::None
        }
        Msg::LoginSuccess(session) => {
            model.is_logging_in = false;
            model.session = Some(session);
            model.screen = Screen::ChatList;
            model.status_message = Some("Logged in successfully! Connecting to broker...".to_string());
            model.reload_chats();
            Cmd::ConnectMqtt
        }
        Msg::LoginFailed(err) => {
            model.is_logging_in = false;
            model.error_message = Some(format!("Login failed: {}", err));
            model.status_message = None;
            Cmd::None
        }
        Msg::OpenChat {
            chat_id,
            display_name,
        } => {
            let _ = model.storage.mark_chat_read(&chat_id);
            model.reload_messages(&chat_id);
            model.screen = Screen::ChatView {
                chat_id,
                display_name,
            };
            model.focused_pane = FocusedPane::Input;
            model.input_buffer.clear();
            Cmd::None
        }
        Msg::BackToChats => {
            model.screen = Screen::ChatList;
            model.focused_pane = FocusedPane::Conversations;
            model.reload_chats();
            Cmd::None
        }
        Msg::StartNewChat => {
            model.phone_input_buffer.clear();
            model.screen = Screen::NewChatPrompt;
            Cmd::None
        }
        Msg::CancelNewChat => {
            model.screen = Screen::ChatList;
            model.focused_pane = FocusedPane::Conversations;
            Cmd::None
        }
        Msg::SendMessage => {
            let country_code = model.user_country_code();
            if let Screen::ChatView { chat_id, .. } = &model.screen {
                let text = model.input_buffer.trim().to_string();
                if !text.is_empty() {
                    let recipient = if uuid::Uuid::parse_str(chat_id).is_ok() {
                        model.storage.get_phone_by_user_id(chat_id).unwrap_or_else(|| chat_id.clone())
                    } else {
                        crate::phone::normalize_identifier(chat_id, Some(&country_code))
                    };
                    model.input_buffer.clear();
                    return Cmd::SendMessage { recipient, text };
                }
            } else if let Some(chat) = model.chats.get(model.selected_chat_idx) {
                let text = model.input_buffer.trim().to_string();
                if !text.is_empty() {
                    let recipient = chat.phone.clone().unwrap_or_else(|| {
                        crate::phone::normalize_identifier(&chat.user_id, Some(&country_code))
                    });
                    model.input_buffer.clear();
                    return Cmd::SendMessage { recipient, text };
                }
            }
            Cmd::None
        }
        Msg::MessageSent {
            id,
            recipient,
            recipient_user_id,
            content,
        } => {
            if let Some(session) = &model.session {
                let canonical_chat_id = if let Some(ref uid) = recipient_user_id {
                    let _ = model.storage.link_phone_and_user_id(&recipient, uid, None);
                    uid.clone()
                } else {
                    model.storage.get_user_id_by_phone(&recipient).unwrap_or_else(|| recipient.clone())
                };

                let _ = model.storage.save_message(
                    &id,
                    &canonical_chat_id,
                    &session.user_id,
                    &content,
                    &session.user_id,
                );

                let phone_opt = if recipient.starts_with('+') { Some(recipient.as_str()) } else { None };
                let _ = model.storage.upsert_chat(&canonical_chat_id, phone_opt, phone_opt, &content, false);

                // If currently viewing the chat by phone, switch ChatView to canonical_chat_id
                if let Screen::ChatView { chat_id, display_name } = &mut model.screen {
                    if *chat_id == recipient {
                        *chat_id = canonical_chat_id.clone();
                        if uuid::Uuid::parse_str(display_name).is_ok() {
                            *display_name = recipient.clone();
                        }
                    }
                }

                model.reload_messages(&canonical_chat_id);
                model.reload_chats();
            }
            Cmd::None
        }
        Msg::MessageSendFailed(err) => {
            model.error_message = Some(format!("Failed to send message: {}", err));
            Cmd::None
        }
        Msg::ContactResolved { user_id, phone, display_name } => {
            if !phone.is_empty() {
                let _ = model.storage.link_phone_and_user_id(&phone, &user_id, display_name.as_deref());
            } else if let Some(name) = &display_name {
                let _ = model.storage.upsert_chat(&user_id, None, Some(name), "", false);
            }
            model.reload_chats();
            if let Screen::ChatView { chat_id, display_name: dname } = &mut model.screen {
                if *chat_id == user_id {
                    if let Some(name) = display_name {
                        *dname = name;
                    } else if !phone.is_empty() {
                        *dname = phone;
                    }
                }
            }
            Cmd::None
        }
        Msg::Sdk(sdk_event) => match sdk_event {
            SdkEvent::Connected => {
                model.is_connected = true;
                model.status_message = Some("Connected to real-time broker".to_string());
                Cmd::None
            }
            SdkEvent::Disconnected => {
                model.is_connected = false;
                model.status_message = Some("Disconnected from broker. Reconnecting...".to_string());
                Cmd::None
            }
            SdkEvent::MessageReceived { sender, plaintext } => {
                let msg_id = uuid::Uuid::new_v4().to_string();
                let my_uid = model.session.as_ref().map(|s| s.user_id.clone()).unwrap_or_default();

                // Unpack framed payload using SDK decode_payload
                let text = match deezchatz_sdk_rust::decode_payload(&plaintext) {
                    Ok(deezchatz_sdk_rust::DecodedPayload::Text { text }) => text,
                    Ok(deezchatz_sdk_rust::DecodedPayload::Voice { .. }) => {
                        "🎤 Voice message".to_string()
                    }
                    Ok(deezchatz_sdk_rust::DecodedPayload::Image { caption, .. }) => {
                        if caption.trim().is_empty() {
                            "📷 Photo".to_string()
                        } else {
                            format!("📷 Photo: {}", caption)
                        }
                    }
                    Err(_) => String::from_utf8_lossy(&plaintext).to_string(),
                };

                let phone = model.storage.get_phone_by_user_id(&sender);
                let is_current_chat = model.active_chat_id().as_deref() == Some(&sender)
                    || (phone.is_some() && model.active_chat_id().as_deref() == phone.as_deref());

                let _ = model.storage.save_message(&msg_id, &sender, &sender, &text, &my_uid);
                let _ = model.storage.upsert_chat(
                    &sender,
                    phone.as_deref(),
                    phone.as_deref(),
                    &text,
                    !is_current_chat,
                );

                if is_current_chat {
                    let _ = model.storage.mark_chat_read(&sender);
                    model.reload_messages(&sender);
                }

                model.reload_chats();
                let display = phone.unwrap_or_else(|| sender.clone());
                model.status_message = Some(format!("New message from {}", display));
                Cmd::None
            }
        },
        Msg::Key(key) => handle_key_event(model, key),
    }
}

fn handle_key_event(model: &mut Model, key: KeyEvent) -> Cmd {
    // Global quit on Ctrl+C
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        model.should_quit = true;
        return Cmd::None;
    }

    match &model.screen {
        Screen::Login => match key.code {
            KeyCode::Enter => {
                let raw_phone = model.login_phone_buffer.trim();
                let normalized = crate::phone::normalize_identifier(raw_phone, Some("91"));
                if normalized.is_empty() || normalized.len() < 8 {
                    model.error_message = Some("Please enter a valid phone number (e.g. +91XXXXXXXXXX)".to_string());
                    Cmd::None
                } else {
                    model.error_message = None;
                    Cmd::TriggerLogin { phone_number: normalized }
                }
            }
            KeyCode::Esc => {
                model.should_quit = true;
                Cmd::None
            }
            KeyCode::Backspace => {
                model.login_phone_buffer.pop();
                Cmd::None
            }
            KeyCode::Char(c) => {
                if c.is_ascii_digit() || c == '+' || c == '-' || c == ' ' {
                    model.login_phone_buffer.push(c);
                }
                Cmd::None
            }
            _ => Cmd::None,
        },
        Screen::ChatList => match key.code {
            KeyCode::Char('q') => {
                model.should_quit = true;
                Cmd::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !model.chats.is_empty() && model.selected_chat_idx + 1 < model.chats.len() {
                    model.selected_chat_idx += 1;
                    if let Some(chat) = model.chats.get(model.selected_chat_idx) {
                        let uid = chat.user_id.clone();
                        model.reload_messages(&uid);
                    }
                }
                Cmd::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if model.selected_chat_idx > 0 {
                    model.selected_chat_idx -= 1;
                    if let Some(chat) = model.chats.get(model.selected_chat_idx) {
                        let uid = chat.user_id.clone();
                        model.reload_messages(&uid);
                    }
                }
                Cmd::None
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Char('i') => {
                if let Some(chat) = model.chats.get(model.selected_chat_idx) {
                    let chat_id = chat.user_id.clone();
                    let display_name = chat.display_name.clone().or_else(|| chat.phone.clone()).unwrap_or_else(|| chat.user_id.clone());
                    update(
                        model,
                        Msg::OpenChat {
                            chat_id,
                            display_name,
                        },
                    )
                } else {
                    // If no chats, pressing Enter or i opens New Chat prompt
                    update(model, Msg::StartNewChat)
                }
            }
            KeyCode::Char('n') => update(model, Msg::StartNewChat),
            KeyCode::PageUp => {
                model.message_scroll_offset = model.message_scroll_offset.saturating_add(5);
                Cmd::None
            }
            KeyCode::PageDown => {
                model.message_scroll_offset = model.message_scroll_offset.saturating_sub(5);
                Cmd::None
            }
            _ => Cmd::None,
        },
        Screen::ChatView { chat_id: _, display_name: _ } => match key.code {
            KeyCode::Esc | KeyCode::Tab => update(model, Msg::BackToChats),
            KeyCode::Enter => update(model, Msg::SendMessage),
            KeyCode::PageUp => {
                model.message_scroll_offset = model.message_scroll_offset.saturating_add(5);
                Cmd::None
            }
            KeyCode::PageDown => {
                model.message_scroll_offset = model.message_scroll_offset.saturating_sub(5);
                Cmd::None
            }
            KeyCode::End => {
                model.message_scroll_offset = 0;
                Cmd::None
            }
            KeyCode::Backspace => {
                model.input_buffer.pop();
                Cmd::None
            }
            KeyCode::Char(c) => {
                model.input_buffer.push(c);
                Cmd::None
            }
            _ => Cmd::None,
        },
        Screen::NewChatPrompt => match key.code {
            KeyCode::Esc => update(model, Msg::CancelNewChat),
            KeyCode::Enter => {
                let raw_target = model.phone_input_buffer.trim();
                if !raw_target.is_empty() {
                    let country_code = model.user_country_code();
                    let target = crate::phone::normalize_identifier(raw_target, Some(&country_code));
                    if target.is_empty() {
                        model.error_message = Some("Please enter a valid phone number, email, or User ID".to_string());
                        Cmd::None
                    } else {
                        model.error_message = None;
                        let canonical_id = model.storage.get_user_id_by_phone(&target).unwrap_or_else(|| target.clone());
                        let phone_opt = if target.starts_with('+') { Some(target.as_str()) } else { None };
                        let _ = model.storage.upsert_chat(&canonical_id, phone_opt, phone_opt, "Conversation started", false);
                        model.reload_chats();
                        update(
                            model,
                            Msg::OpenChat {
                                chat_id: canonical_id,
                                display_name: target,
                            },
                        )
                    }
                } else {
                    Cmd::None
                }
            }
            KeyCode::Backspace => {
                model.phone_input_buffer.pop();
                Cmd::None
            }
            KeyCode::Char(c) => {
                model.phone_input_buffer.push(c);
                Cmd::None
            }
            _ => Cmd::None,
        },
    }
}
