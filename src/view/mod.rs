use crate::model::{FocusedPane, Model, Screen};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

pub fn render(model: &Model, frame: &mut Frame) {
    let size = frame.area();
    let is_compact_height = size.height < 18;

    // Main layout: Header, Content, Footer
    let chunks = if is_compact_height {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Compact 1-line header
                Constraint::Min(4),    // Main Content
                Constraint::Length(1), // Compact 1-line footer
            ])
            .split(size)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Bordered 3-line header
                Constraint::Min(8),    // Main Content
                Constraint::Length(3), // Bordered 3-line footer
            ])
            .split(size)
    };

    render_header(model, frame, chunks[0], is_compact_height);

    match &model.screen {
        Screen::Login => {
            render_login(model, frame, chunks[1]);
        }
        Screen::ChatList | Screen::ChatView { .. } => {
            render_main_layout(model, frame, chunks[1]);
        }
        Screen::NewChatPrompt => {
            render_main_layout(model, frame, chunks[1]);
            render_new_chat_dialog(model, frame, chunks[1]);
        }
    }

    render_footer(model, frame, chunks[2], is_compact_height);
}

fn render_header(model: &Model, frame: &mut Frame, area: Rect, is_compact: bool) {
    let conn_status = if model.is_connected {
        Span::styled("🟢 Online", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("🔴 Offline", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    };

    let user_info = if let Some(session) = &model.session {
        format!("📱 {} (ID: {})", session.phone_number, &session.user_id[..8.min(session.user_id.len())])
    } else {
        "🔐 Setup Mode".to_string()
    };

    if is_compact {
        // Single-line compact header without box borders
        let text = if area.width >= 60 {
            Line::from(vec![
                Span::styled(" 💬 DeezChatz ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(user_info, Style::default().fg(Color::Yellow)),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                conn_status,
            ])
        } else {
            Line::from(vec![
                Span::styled(" 💬 DeezChatz ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                conn_status,
            ])
        };
        frame.render_widget(Paragraph::new(text), area);
        return;
    }

    // 3-line bordered header
    if area.width >= 75 {
        let header_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(18), // App Brand
                Constraint::Min(24),    // User Identity
                Constraint::Length(15), // Broker Status
            ])
            .split(area);

        let title_p = Paragraph::new(Span::styled(
            " 💬 DeezChatz ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)));
        frame.render_widget(title_p, header_chunks[0]);

        let user_p = Paragraph::new(Span::styled(
            format!(" {}", user_info),
            Style::default().fg(Color::Yellow),
        ))
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
        frame.render_widget(user_p, header_chunks[1]);

        let conn_p = Paragraph::new(conn_status)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
        frame.render_widget(conn_p, header_chunks[2]);
    } else {
        let header_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(16),    // App Brand
                Constraint::Length(15), // Broker Status
            ])
            .split(area);

        let title_p = Paragraph::new(Span::styled(
            " 💬 DeezChatz ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)));
        frame.render_widget(title_p, header_chunks[0]);

        let conn_p = Paragraph::new(conn_status)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
        frame.render_widget(conn_p, header_chunks[1]);
    }
}

fn render_footer(model: &Model, frame: &mut Frame, area: Rect, is_compact: bool) {
    let is_wide = frame.area().width >= 80;

    let hints = match &model.screen {
        Screen::Login => " [Enter] Continue with Google │ [Backspace] Edit Phone │ [Esc] Quit ",
        Screen::NewChatPrompt => " [Enter] Start Chat │ [Esc] Cancel ",
        _ => {
            if is_wide {
                match model.focused_pane {
                    FocusedPane::Conversations => {
                        " [↑/↓ / j/k] Select Chat │ [Tab/Enter/i] Write Message │ [n] New Chat │ [q] Quit "
                    }
                    FocusedPane::Input => {
                        " [Enter] Send │ [Esc/Tab] Switch to Chats │ [PgUp/PgDn] Scroll "
                    }
                }
            } else {
                match model.focused_pane {
                    FocusedPane::Conversations => {
                        " [↑/↓] Select │ [Enter] Open Chat │ [n] New Chat │ [q] Quit "
                    }
                    FocusedPane::Input => {
                        " [Enter] Send │ [Esc] Back to Chats │ [PgUp/PgDn] Scroll "
                    }
                }
            }
        }
    };

    if is_compact {
        let text = if let Some(err) = &model.error_message {
            Line::from(vec![
                Span::styled("❌ ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled(err, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            ])
        } else {
            Line::from(Span::styled(hints, Style::default().fg(Color::Cyan)))
        };
        frame.render_widget(Paragraph::new(text), area);
        return;
    }

    let status_line = if let Some(err) = &model.error_message {
        Line::from(vec![
            Span::styled(" ❌ ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(err, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        ])
    } else if let Some(status) = &model.status_message {
        Line::from(vec![
            Span::styled(" ℹ️ ", Style::default().fg(Color::Cyan)),
            Span::styled(status, Style::default().fg(Color::White)),
        ])
    } else {
        Line::from(Span::styled(
            " 🔒 End-to-End Encrypted (Signal Protocol) ",
            Style::default().fg(Color::DarkGray),
        ))
    };

    let footer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title_top(Line::from(Span::styled(
            hints,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));

    let p = Paragraph::new(status_line).block(footer_block);
    frame.render_widget(p, area);
}

fn render_login(model: &Model, frame: &mut Frame, area: Rect) {
    let card_area = centered_rect_bounded(54, 13, area);
    frame.render_widget(Clear, card_area);

    if model.is_logging_in {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" 🌐 Google Authentication ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

        let text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "A browser window has opened for Google sign-in.",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Complete sign-in in your browser window.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Listening for authorization on http://127.0.0.1:8080...",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press [Esc] to cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let p = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(block)
            .wrap(Wrap { trim: true });

        frame.render_widget(p, card_area);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" 🔐 Welcome to DeezChatz ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

        let text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Signal Protocol End-to-End Encrypted CLI",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled("Enter your Phone Number:", Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(vec![
                Span::styled("    [ ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    &model.login_phone_buffer,
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ]    ", Style::default().fg(Color::Cyan)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Press [Enter] to sign in with Google",
                Style::default().fg(Color::Green),
            )),
            Line::from(Span::styled(
                "Press [Esc] to quit",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let p = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(block)
            .wrap(Wrap { trim: true });

        frame.render_widget(p, card_area);

        // Hardware cursor inside the phone field
        let cursor_x = card_area.x + 6 + (model.login_phone_buffer.chars().count() as u16)
            .min(card_area.width.saturating_sub(8));
        let cursor_y = card_area.y + 5;
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

/// Adaptive layout:
/// - Wide terminals (>= 80 cols): Two-pane split (Sidebar + Active Chat / Preview)
/// - Narrow terminals (< 80 cols): Single-pane drill-down (List or Chat)
fn render_main_layout(model: &Model, frame: &mut Frame, area: Rect) {
    let is_wide = area.width >= 80;

    if is_wide {
        let sidebar_width = 32.min(area.width / 3).max(26);
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(sidebar_width),
                Constraint::Min(30),
            ])
            .split(area);

        let is_sidebar_focused = model.focused_pane == FocusedPane::Conversations;
        render_conversations_sidebar(model, frame, panels[0], is_sidebar_focused);
        render_chat_pane(model, frame, panels[1], model.focused_pane == FocusedPane::Input);
    } else {
        // Single-pane mode: Drill down into chat if in Input focus or ChatView screen
        let in_chat = model.focused_pane == FocusedPane::Input
            || matches!(model.screen, Screen::ChatView { .. });

        if in_chat {
            render_chat_pane(model, frame, area, true);
        } else {
            render_conversations_sidebar(model, frame, area, true);
        }
    }
}

fn render_conversations_sidebar(model: &Model, frame: &mut Frame, area: Rect, is_focused: bool) {
    let items: Vec<ListItem> = if model.chats.is_empty() {
        vec![ListItem::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No conversations yet.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  Press [n] to start a chat!",
                Style::default().fg(Color::Yellow),
            )),
        ])]
    } else {
        model
            .chats
            .iter()
            .enumerate()
            .map(|(idx, chat)| {
                let is_selected = idx == model.selected_chat_idx;
                let name = chat.display_name.as_deref().or(chat.phone.as_deref()).unwrap_or(&chat.user_id);
                let unread_badge = if chat.unread_count > 0 {
                    format!(" ({} new)", chat.unread_count)
                } else {
                    "".to_string()
                };

                let (pointer, name_style, bg_color) = if is_selected && is_focused {
                    (
                        "▶ ",
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                        Color::DarkGray,
                    )
                } else if is_selected {
                    (
                        "▷ ",
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                        Color::Reset,
                    )
                } else {
                    (
                        "  ",
                        Style::default().fg(Color::Gray),
                        Color::Reset,
                    )
                };

                let max_preview_len = (area.width.saturating_sub(6)) as usize;
                let preview = truncate_str(&chat.last_message, max_preview_len);

                let lines = vec![
                    Line::from(vec![
                        Span::styled(pointer, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::styled(name, name_style),
                        Span::styled(unread_badge, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::raw("   "),
                        Span::styled(preview, Style::default().fg(Color::DarkGray)),
                    ]),
                ];

                ListItem::new(lines).style(Style::default().bg(bg_color))
            })
            .collect()
    };

    let (border_color, title) = if is_focused {
        (Color::Cyan, " 💬 Conversations [FOCUSED] ")
    } else {
        (Color::DarkGray, " 💬 Conversations ")
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_chat_pane(model: &Model, frame: &mut Frame, area: Rect, is_input_focused: bool) {
    let active_id = model.active_chat_id();

    // If no chat is available (e.g. empty inbox on first login), show Welcome Card
    let (chat_id, display_name) = match (active_id, model.active_display_name()) {
        (Some(cid), Some(dname)) => (cid, dname),
        _ => {
            render_welcome_card(frame, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),    // Message thread
            Constraint::Length(3), // Input box
        ])
        .split(area);

    let msg_area = chunks[0];
    let input_area = chunks[1];

    // Build message list items
    let mut message_items: Vec<ListItem> = Vec::new();

    if model.active_messages.is_empty() {
        message_items.push(ListItem::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  👋 Starting a conversation with {}", display_name),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Type a message below and press [Enter] to send.",
                Style::default().fg(Color::DarkGray),
            )),
        ]));
    } else {
        // Calculate visible window to ensure auto-scrolling to the latest message
        let available_height = msg_area.height.saturating_sub(2) as usize;
        let total_msgs = model.active_messages.len();

        let end_idx = total_msgs.saturating_sub(model.message_scroll_offset);
        let mut start_idx = end_idx;
        let mut accumulated_lines = 0;

        let content_width = (msg_area.width.saturating_sub(8)).max(12) as usize;

        while start_idx > 0 && accumulated_lines < available_height {
            let msg = &model.active_messages[start_idx - 1];
            let approx_lines = 2 + (msg.content.chars().count() / content_width).max(1);
            if accumulated_lines + approx_lines > available_height && accumulated_lines > 0 {
                break;
            }
            accumulated_lines += approx_lines;
            start_idx -= 1;
        }

        let visible_msgs = &model.active_messages[start_idx..end_idx];

        for msg in visible_msgs {
            let time_str = format_timestamp(msg.created_at);
            if msg.is_outgoing {
                // Outgoing bubble (You)
                let lines = vec![
                    Line::from(vec![
                        Span::styled("   ▸ You ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("[{}] ✓", time_str), Style::default().fg(Color::DarkGray)),
                    ]),
                    Line::from(vec![
                        Span::raw("     "),
                        Span::styled(&msg.content, Style::default().fg(Color::White)),
                    ]),
                    Line::from(""),
                ];
                message_items.push(ListItem::new(lines));
            } else {
                // Incoming bubble (Peer)
                let lines = vec![
                    Line::from(vec![
                        Span::styled(format!(" ◂ {} ", display_name), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("[{}]", time_str), Style::default().fg(Color::DarkGray)),
                    ]),
                    Line::from(vec![
                        Span::raw("   "),
                        Span::styled(&msg.content, Style::default().fg(Color::White)),
                    ]),
                    Line::from(""),
                ];
                message_items.push(ListItem::new(lines));
            }
        }
    }

    let title = if model.message_scroll_offset > 0 {
        format!(
            " 💬 Chat: {} [📜 Scrolled up {} msgs - Press End to reset] ",
            display_name, model.message_scroll_offset
        )
    } else {
        format!(" 💬 Chat: {} ({}) ", display_name, &chat_id[..8.min(chat_id.len())])
    };

    let msg_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray));

    let msg_list = List::new(message_items).block(msg_block);
    frame.render_widget(msg_list, msg_area);

    // Input box
    let (input_border_color, input_title) = if is_input_focused {
        (
            Color::Yellow,
            " ✉️ Message [ACTIVE: Enter to send, Esc to switch] ",
        )
    } else {
        (
            Color::DarkGray,
            " ✉️ Press [Enter] or [Tab] to write ",
        )
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(input_title)
        .border_style(Style::default().fg(input_border_color).add_modifier(Modifier::BOLD));

    let input_p = Paragraph::new(format!("> {}", model.input_buffer)).block(input_block);
    frame.render_widget(input_p, input_area);

    // Place hardware cursor inside the input box when focused
    if is_input_focused {
        let max_cursor_x = input_area.x + input_area.width.saturating_sub(3);
        let cursor_x = (input_area.x + 3 + model.input_buffer.chars().count() as u16).min(max_cursor_x);
        let cursor_y = input_area.y + 1;
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

fn render_welcome_card(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 💬 Welcome to DeezChatz ")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Cyan));

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Private End-to-End Encrypted Terminal Messenger",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled("🚀 Quick Start:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  1. Press [n] to start a new conversation", Style::default().fg(Color::White))),
        Line::from(Span::styled("  2. Enter recipient's phone number (e.g. +91XXXXXXXXXX)", Style::default().fg(Color::White))),
        Line::from(Span::styled("  3. Type your message and hit [Enter] to send!", Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(Span::styled("🔒 Security Architecture:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  • Signal Protocol Double Ratchet session per contact", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("  • Individual 256-bit SQLCipher database per chat", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("  • Zero plaintext keys or messages on disk", Style::default().fg(Color::DarkGray))),
    ];

    let p = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(block)
        .wrap(Wrap { trim: true });

    frame.render_widget(p, area);
}

fn render_new_chat_dialog(model: &Model, frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect_bounded(50, 9, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ➕ Start New Conversation ")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Enter recipient's Phone Number or User ID:",
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                &model.phone_input_buffer,
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ]", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] Start Chat  │  [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });

    frame.render_widget(p, popup_area);

    // Hardware cursor inside the dialog
    let cursor_x = popup_area.x + 5 + (model.phone_input_buffer.chars().count() as u16)
        .min(popup_area.width.saturating_sub(7));
    let cursor_y = popup_area.y + 3;
    frame.set_cursor_position(Position::new(cursor_x, cursor_y));
}

/// Computes an adaptive centered bounding box that never overflows on small terminals
/// and never stretches unnaturally on ultra-wide / ultra-tall terminals.
fn centered_rect_bounded(target_width: u16, target_height: u16, area: Rect) -> Rect {
    let w = target_width.min(area.width.saturating_sub(4)).max(20);
    let h = target_height.min(area.height.saturating_sub(2)).max(7);
    let x = (area.width.saturating_sub(w)) / 2 + area.x;
    let y = (area.height.saturating_sub(h)) / 2 + area.y;
    Rect::new(x, y, w, h)
}

fn format_timestamp(ts: u64) -> String {
    let secs_per_day = 86400;
    let secs_per_hour = 3600;
    let secs_per_min = 60;
    let time_of_day = ts % secs_per_day;
    let hours = (time_of_day / secs_per_hour) % 24;
    let mins = (time_of_day % secs_per_hour) / secs_per_min;
    format!("{:02}:{:02}", hours, mins)
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}
