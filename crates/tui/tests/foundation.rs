use std::convert::Infallible;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use tui::{
    app::{Dashboard, Screen},
    event::{UiEvent, translate},
    layout::{self, LayoutClass},
};

fn render(app: &Dashboard, width: u16, height: u16) -> Result<String, Infallible> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    terminal.draw(|frame| app.render(frame))?;
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..height {
        for x in 0..width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    Ok(text)
}

#[test]
fn every_screen_preserves_lockout_and_unavailable_data() -> Result<(), Infallible> {
    for screen in Screen::ALL {
        for (width, height) in [(160, 45), (80, 24), (40, 12)] {
            let app = Dashboard {
                screen,
                ..Dashboard::default()
            };
            let text = render(&app, width, height)?;
            assert!(text.contains("SAFETY LOCKOUT"));
            assert!(text.contains("OUTPUT DISABLED"));
            assert!(!text.contains("OUTPUT PERMITTED"));
            assert!(
                text.contains("unavailable")
                    || text.contains("No completed runs")
                    || text.contains("No track data")
            );
        }
    }
    Ok(())
}

#[test]
fn help_does_not_hide_safety() -> Result<(), Infallible> {
    let app = Dashboard {
        help: true,
        ..Dashboard::default()
    };
    let text = render(&app, 80, 24)?;
    assert!(text.contains("SAFETY LOCKOUT"));
    assert!(text.contains("OUTPUT DISABLED"));
    assert!(text.contains("Close help"));
    Ok(())
}

#[test]
fn tiny_and_zero_terminals_do_not_panic() -> Result<(), Infallible> {
    for width in 0..=20 {
        for height in 0..=8 {
            let text = render(&Dashboard::default(), width, height)?;
            if width >= 8 && (1..4).contains(&height) {
                assert!(text.contains("LOCKOUT"));
            }
        }
    }
    Ok(())
}

#[test]
fn layout_classes_have_explicit_boundaries() {
    assert_eq!(layout::class(Rect::new(0, 0, 120, 35)), LayoutClass::Large);
    assert_eq!(layout::class(Rect::new(0, 0, 80, 24)), LayoutClass::Medium);
    assert_eq!(layout::class(Rect::new(0, 0, 79, 24)), LayoutClass::Minimal);
}

#[test]
fn navigation_help_quit_and_invalid_selection() {
    let mut app = Dashboard::default();
    for (index, screen) in Screen::ALL.iter().enumerate() {
        app.update(UiEvent::SelectScreen(index as u8));
        assert_eq!(app.screen, *screen);
    }
    app.update(UiEvent::SelectScreen(255));
    assert_eq!(app.screen, Screen::Calibration);
    app.update(UiEvent::ToggleHelp);
    assert!(app.help);
    app.update(UiEvent::CloseHelp);
    assert!(!app.help);
    app.update(UiEvent::Quit);
    assert!(app.quit);
}

#[test]
fn windows_release_and_repeat_events_do_not_duplicate_actions() {
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let key = KeyEvent::new_with_kind(KeyCode::Char('?'), KeyModifiers::NONE, kind);
        assert_eq!(translate(Event::Key(key)), None);
    }
    assert_eq!(
        translate(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        ))),
        Some(UiEvent::Quit)
    );
    for code in [KeyCode::Char('r'), KeyCode::Char('s')] {
        assert_eq!(
            translate(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))),
            None
        );
    }
}
