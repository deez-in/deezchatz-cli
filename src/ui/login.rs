use crate::model::Model;
use ratatui::{
    layout::{Alignment, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::centered_rect_bounded;

pub fn render_login(model: &Model, frame: &mut Frame, area: Rect) {
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
