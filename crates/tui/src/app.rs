use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{event::UiEvent, layout, theme};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Live,
    Performance,
    Tracks,
    Safety,
    Calibration,
}

impl Screen {
    pub const ALL: [Self; 5] = [
        Self::Live,
        Self::Performance,
        Self::Tracks,
        Self::Safety,
        Self::Calibration,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Live => "Live",
            Self::Performance => "Performance",
            Self::Tracks => "Tracks",
            Self::Safety => "Safety",
            Self::Calibration => "Calibration",
        }
    }
}

#[derive(Debug, Default)]
pub struct Dashboard {
    pub connected: Option<crate::views::ConnectedView>,
    pub screen: Screen,
    pub help: bool,
    pub quit: bool,
}

impl Dashboard {
    pub fn update(&mut self, event: UiEvent) {
        if let Some(view) = &mut self.connected {
            view.update(event);
        }
        match event {
            UiEvent::SelectScreen(index) => {
                if let Some(screen) = Screen::ALL.get(usize::from(index)) {
                    self.screen = *screen;
                }
            }
            UiEvent::ToggleHelp => self.help = !self.help,
            UiEvent::CloseHelp => self.help = false,
            UiEvent::Quit => self.quit = true,
            UiEvent::Tick
            | UiEvent::Resize
            | UiEvent::CycleFocus
            | UiEvent::TogglePause
            | UiEvent::Step(_)
            | UiEvent::SelectTrack(_)
            | UiEvent::ChangeSpeed(_) => {}
            UiEvent::Scroll(_) | UiEvent::ToggleForeground | UiEvent::ToggleImageStyle => {}
        }
    }

    pub fn render(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        if area.is_empty() {
            return;
        }
        // Safety has first claim on even a one-row terminal.
        if area.height < 4 {
            frame.render_widget(
                Paragraph::new(self.connected.as_ref().map_or_else(
                    || "LOCKOUT: no safety data".into(),
                    |view| view.safety_text(),
                ))
                .style(theme::fault()),
                area,
            );
            return;
        }
        let [header, safety, body, footer] = layout::regions(area);
        frame.render_widget(
            Paragraph::new(self.connected.as_ref().map_or_else(
                || {
                    format!(
                        "FLY TRACKER | {} | NO SOURCE | CAMERA N/A | AIM DISABLED",
                        self.screen.title()
                    )
                },
                |view| view.header(),
            )),
            header,
        );
        frame.render_widget(
            Paragraph::new(self.connected.as_ref().map_or_else(||"SAFETY LOCKOUT | OUTPUT DISABLED\nSafety detector unavailable; no telemetry connected.".into(),|view|view.safety_text()))
                .style(self.connected.as_ref().map_or_else(theme::fault,crate::views::safety_style))
                .wrap(Wrap { trim: true }),
            safety,
        );
        if self.help {
            frame.render_widget(Clear, body);
            frame.render_widget(
                Paragraph::new("1 Live   2 Performance   3 Tracks   4 Safety   5 Calibration\n? Help   Esc Close help   q / Ctrl+C Quit\nReplay: Space pause, arrows step/rewind when paused, Shift larger step\nTab changes focus; Up/Down select track; +/- replay speed\nm image/mask; g grayscale/ASCII; PageUp/PageDown scroll\nUnsupported calibration controls remain unavailable.\nRecorded evidence cannot grant live output permission.")
                    .wrap(Wrap { trim: true })
                    .block(Block::default().title("Help").borders(Borders::ALL)),
                body,
            );
        } else {
            self.render_screen(frame, body);
        }
        frame.render_widget(
            Paragraph::new(if let Some(view)=&self.connected && !view.notice.is_empty(){view.notice.as_str()}else if self.connected.is_some() {"[1-5] Screens [Space] Pause [Tab] Focus [arrows] Select/Step [+/-] Speed [m] Mask [g] ASCII [?] Help [q] Quit"}else{"[1-5] Screens  [?] Help  [q] Quit"}).style(theme::secondary()),
            footer,
        );
    }

    fn render_screen(&self, frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
        if let Some(view) = &self.connected {
            view.render(frame, area, self.screen);
            return;
        }
        let message = match self.screen {
            Screen::Live => {
                "Camera viewport unavailable\nNo active target\nObserved / trajectory / predicted / requested aim: unavailable"
            }
            Screen::Performance => {
                "No completed runs\nCurrent / previous / best: unavailable\nP50 / P95 / P99 / MAX: unavailable"
            }
            Screen::Tracks => {
                "No track data\nID  STATE  AGE  CONFIDENCE  SPEED  LOST  ERROR\nTrajectory unavailable"
            }
            Screen::Safety => {
                "Human / dog / cat confidence: unavailable\nCamera freshness: unavailable\nSafety freshness: unavailable\nWatchdog: unavailable\nOutput interlock: not connected\nNo safety authority is connected"
            }
            Screen::Calibration => {
                "No calibration loaded\nVersion / timestamp: unavailable\nPoints / residual / RMS / maximum error: unavailable\nCalibration actions are not connected"
            }
        };
        let panel = |text, title| {
            Paragraph::new(text)
                .wrap(Wrap { trim: true })
                .block(Block::default().title(title).borders(Borders::ALL))
        };
        if self.screen == Screen::Live
            && layout::class(frame.area()) != layout::LayoutClass::Minimal
        {
            let columns =
                Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)])
                    .split(area);
            frame.render_widget(panel(message, "Live viewport"), columns[0]);
            frame.render_widget(panel("Target: unavailable\nCapture / safety / detection / tracking / prediction / aim: N/A\nTotal latency: N/A\nP50 / P95 / P99 / MAX: N/A\nSession counters: N/A", "Pipeline"), columns[1]);
        } else {
            frame.render_widget(panel(message, self.screen.title()), area);
        }
    }
}
