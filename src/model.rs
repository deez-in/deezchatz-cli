use crate::db::{ChatMessage, ChatSummary, SqliteStorage};
use crate::keyring_store::StoredSession;
use crossterm::event::KeyEvent;
use deezchatz_sdk_rust::Event as SdkEvent;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Login,
    ChatList,
    ChatView {
        chat_id: String,
        display_name: String,
    },
    NewChatPrompt,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum Msg {
    Key(KeyEvent),
    Sdk(SdkEvent),
    RefreshChats,
    LoginStarted,
    LoginSuccess(StoredSession),
    LoginFailed(String),
    OpenChat {
        chat_id: String,
        display_name: String,
    },
    BackToChats,
    StartNewChat,
    CancelNewChat,
    SendMessage,
    MessageSent {
        id: String,
        recipient: String,
        recipient_user_id: Option<String>,
        content: String,
    },
    MessageSendFailed(String),
    ContactResolved {
        user_id: String,
        phone: String,
        display_name: Option<String>,
    },
    SetStatus(String),
    ClearError,
    Quit,
}

#[derive(Debug)]
pub enum Cmd {
    None,
    TriggerLogin { phone_number: String },
    SendMessage { recipient: String, text: String },
    ConnectMqtt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusedPane {
    Conversations,
    Input,
}

pub struct Model {
    pub screen: Screen,
    pub focused_pane: FocusedPane,
    pub session: Option<StoredSession>,
    pub storage: SqliteStorage,
    pub chats: Vec<ChatSummary>,
    pub selected_chat_idx: usize,
    pub active_messages: Vec<ChatMessage>,
    pub message_scroll: usize,
    pub message_scroll_offset: usize,
    pub input_buffer: String,
    pub phone_input_buffer: String,
    pub login_phone_buffer: String,
    pub is_logging_in: bool,
    pub status_message: Option<String>,
    pub error_message: Option<String>,
    pub is_connected: bool,
    pub should_quit: bool,
}

impl Model {
    pub fn new(storage: SqliteStorage, session: Option<StoredSession>) -> Self {
        let initial_screen = if session.is_some() {
            Screen::ChatList
        } else {
            Screen::Login
        };

        let mut model = Self {
            screen: initial_screen,
            focused_pane: FocusedPane::Conversations,
            session,
            storage,
            chats: Vec::new(),
            selected_chat_idx: 0,
            active_messages: Vec::new(),
            message_scroll: 0,
            message_scroll_offset: 0,
            input_buffer: String::new(),
            phone_input_buffer: String::new(),
            login_phone_buffer: "+91".to_string(),
            is_logging_in: false,
            status_message: Some("Welcome to DeezChatz CLI".to_string()),
            error_message: None,
            is_connected: false,
            should_quit: false,
        };

        model.reload_chats();
        model
    }

    pub fn reload_chats(&mut self) {
        if let Ok(chats) = self.storage.get_chats() {
            self.chats = chats;
            if self.selected_chat_idx >= self.chats.len() && !self.chats.is_empty() {
                self.selected_chat_idx = self.chats.len() - 1;
            }
            if let Some(chat) = self.chats.get(self.selected_chat_idx) {
                let uid = chat.user_id.clone();
                self.reload_messages(&uid);
            }
        }
    }

    pub fn reload_messages(&mut self, chat_id: &str) {
        if let Some(session) = &self.session {
            if let Ok(msgs) = self.storage.get_messages(chat_id, &session.user_id) {
                self.active_messages = msgs;
                self.message_scroll = self.active_messages.len().saturating_sub(1);
                self.message_scroll_offset = 0;
            }
        }
    }

    pub fn active_chat_id(&self) -> Option<String> {
        match &self.screen {
            Screen::ChatView { chat_id, .. } => Some(chat_id.clone()),
            _ => self.chats.get(self.selected_chat_idx).map(|c| c.user_id.clone()),
        }
    }

    pub fn active_display_name(&self) -> Option<String> {
        match &self.screen {
            Screen::ChatView { chat_id, display_name } => {
                if uuid::Uuid::parse_str(display_name).is_ok() {
                    self.storage.get_phone_by_user_id(chat_id).or_else(|| Some(display_name.clone()))
                } else {
                    Some(display_name.clone())
                }
            }
            _ => self.chats.get(self.selected_chat_idx).map(|c| {
                c.display_name.clone().or_else(|| c.phone.clone()).unwrap_or_else(|| c.user_id.clone())
            }),
        }
    }

    pub fn user_country_code(&self) -> String {
        self.session
            .as_ref()
            .and_then(|s| crate::phone::extract_country_code(&s.phone_number))
            .unwrap_or_else(|| "91".to_string())
    }
}
