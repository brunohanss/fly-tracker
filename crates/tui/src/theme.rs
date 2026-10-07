use ratatui::style::{Color, Modifier, Style};

pub fn fault() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

pub fn secondary() -> Style {
    Style::default().fg(Color::Gray)
}
