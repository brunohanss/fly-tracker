use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEvent {
    ToggleImageStyle,
    ToggleForeground,
    Scroll(i16),
    CycleFocus,
    TogglePause,
    Step(i64),
    SelectTrack(i32),
    ChangeSpeed(i8),
    Tick,
    SelectScreen(u8),
    ToggleHelp,
    CloseHelp,
    Quit,
    Resize,
}

pub fn translate(event: Event) -> Option<UiEvent> {
    match event {
        Event::Resize(_, _) => Some(UiEvent::Resize),
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UiEvent::Quit)
            }
            KeyCode::Char('q') => Some(UiEvent::Quit),
            KeyCode::Char('?') => Some(UiEvent::ToggleHelp),
            KeyCode::Char('m') => Some(UiEvent::ToggleForeground),
            KeyCode::Char('g') => Some(UiEvent::ToggleImageStyle),
            KeyCode::Esc => Some(UiEvent::CloseHelp),
            KeyCode::Tab => Some(UiEvent::CycleFocus),
            KeyCode::Char(' ') => Some(UiEvent::TogglePause),
            KeyCode::Right => Some(UiEvent::Step(
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    10
                } else {
                    1
                },
            )),
            KeyCode::Left => Some(UiEvent::Step(
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    -10
                } else {
                    -1
                },
            )),
            KeyCode::Up => Some(UiEvent::SelectTrack(-1)),
            KeyCode::Down => Some(UiEvent::SelectTrack(1)),
            KeyCode::PageDown => Some(UiEvent::Scroll(5)),
            KeyCode::PageUp => Some(UiEvent::Scroll(-5)),
            KeyCode::Char('+') | KeyCode::Char('=') => Some(UiEvent::ChangeSpeed(1)),
            KeyCode::Char('-') => Some(UiEvent::ChangeSpeed(-1)),
            KeyCode::Char(number @ '1'..='5') if key.modifiers.is_empty() => {
                Some(UiEvent::SelectScreen(number as u8 - b'1'))
            }
            _ => None,
        },
        _ => None,
    }
}
