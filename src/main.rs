use ratatui::{DefaultTerminal, Frame};

#[derive(Debug, Default)]
pub struct App {
    counter: u8,
    exit: bool,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    // ratatui::run(app)?;
    ratatui::run(|terminal| App::default().run(terminal))
    Ok(())
}

fn app(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    loop {
        terminal.draw(render)?;
        if crossterm::event::read()?.is_key_press() {
            break Ok(());
        }
    }
}

fn render(frame: &mut Frame) {
    frame.render_widget("hello world", frame.area());
}
