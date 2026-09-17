pub mod chat;
pub mod login;

use crate::model::{FocusedPane, Model, Screen};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
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
            login::render_login(model, frame, chunks[1]);
        }
        Screen::ChatList | Screen::ChatView { .. } => {
            chat::render_main_layout(model, frame, chunks[1]);
        }
        Screen::NewChatPrompt => {
            chat::render_main_layout(model, frame, chunks[1]);
            chat::render_new_chat_dialog(model, frame, chunks[1]);
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

/// Computes an adaptive centered bounding box that never overflows on small terminals
/// and never stretches unnaturally on ultra-wide / ultra-tall terminals.
pub fn centered_rect_bounded(target_width: u16, target_height: u16, area: Rect) -> Rect {
    let w = target_width.min(area.width.saturating_sub(4)).max(20);
    let h = target_height.min(area.height.saturating_sub(2)).max(7);
    let x = (area.width.saturating_sub(w)) / 2 + area.x;
    let y = (area.height.saturating_sub(h)) / 2 + area.y;
    Rect::new(x, y, w, h)
}

pub fn format_timestamp(ts: u64) -> String {
    let secs_per_day = 86400;
    let secs_per_hour = 3600;
    let secs_per_min = 60;
    let time_of_day = ts % secs_per_day;
    let hours = (time_of_day / secs_per_hour) % 24;
    let mins = (time_of_day % secs_per_hour) / secs_per_min;
    format!("{:02}:{:02}", hours, mins)
}

pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}
