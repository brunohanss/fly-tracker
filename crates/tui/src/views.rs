use crate::{app::Screen, event::UiEvent, theme};
use fly_core::{PixelPosition, SafetyState, TargetId};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};
use std::collections::{BTreeMap, VecDeque};
use telemetry::{
    DashboardSnapshot, Distribution, ReplayCommand, RunMetrics, Stage, percentage_change,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Replay,
    Tracks,
}
#[derive(Debug)]
pub struct ConnectedView {
    pub snapshot: DashboardSnapshot,
    pub stale: bool,
    pub selected: Option<TargetId>,
    pub focus: Focus,
    pub command: Option<ReplayCommand>,
    pub notice: String,
    pub runs: Vec<RunMetrics>,
    events: VecDeque<String>,
    trajectories: BTreeMap<TargetId, VecDeque<PixelPosition>>,
    image: crate::viewport::CameraImage,
    ascii: bool,
    scroll: u16,
    foreground: bool,
}
impl ConnectedView {
    pub fn new(snapshot: DashboardSnapshot) -> Self {
        let stale = snapshot.publication_time.elapsed() > std::time::Duration::from_millis(500);
        let mut view = Self {
            snapshot,
            stale,
            selected: None,
            focus: Focus::Replay,
            command: None,
            notice: String::new(),
            runs: Vec::new(),
            events: VecDeque::with_capacity(16),
            trajectories: BTreeMap::new(),
            image: crate::viewport::CameraImage::default(),
            ascii: false,
            scroll: 0,
            foreground: false,
        };
        view.prepare();
        view
    }
    pub fn receive(&mut self, snapshot: DashboardSnapshot) {
        if snapshot.counters.processed_frames < self.snapshot.counters.processed_frames {
            self.trajectories.clear();
            self.events.clear();
        }
        if self.snapshot.safety != snapshot.safety {
            self.event(format!("frame {}: {:?}", snapshot.frame.0, snapshot.safety));
        }
        if snapshot.counters.lost_targets != self.snapshot.counters.lost_targets {
            self.event(format!("Losses: {}", snapshot.counters.lost_targets));
        }
        if snapshot.counters.reacquisitions != self.snapshot.counters.reacquisitions {
            self.event(format!(
                "Reacquisitions: {}",
                snapshot.counters.reacquisitions
            ));
        }
        self.snapshot = snapshot;
        self.stale =
            self.snapshot.publication_time.elapsed() > std::time::Duration::from_millis(500);
        self.prepare();
    }
    fn event(&mut self, event: String) {
        if self.events.len() == 16 {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
    fn prepare(&mut self) {
        if self
            .selected
            .is_none_or(|id| !self.snapshot.tracks.iter().any(|track| track.id == id))
        {
            self.selected = self.snapshot.tracks.first().map(|track| track.id);
        }
        self.trajectories.retain(|id, _| {
            self.snapshot
                .tracks
                .iter()
                .take(64)
                .any(|track| track.id == *id)
        });
        for track in self.snapshot.tracks.iter().take(64) {
            let path = self
                .trajectories
                .entry(track.id)
                .or_insert_with(|| VecDeque::with_capacity(64));
            if path.back() != Some(&track.position) {
                if path.len() == 64 {
                    path.pop_front();
                }
                path.push_back(track.position);
            }
        }
        let preview = if self.foreground {
            &self.snapshot.foreground_preview
        } else {
            &self.snapshot.preview
        };
        self.image.prepare(preview);
    }
    pub fn update(&mut self, event: UiEvent) {
        match event {
            UiEvent::ToggleImageStyle => self.ascii = !self.ascii,
            UiEvent::ToggleForeground => {
                self.foreground = !self.foreground;
                self.prepare();
            }
            UiEvent::Scroll(delta) => {
                self.scroll = (i32::from(self.scroll) + i32::from(delta)).clamp(0, 512) as u16
            }
            UiEvent::SelectScreen(_) => self.scroll = 0,
            UiEvent::CycleFocus => {
                self.focus = if self.focus == Focus::Replay {
                    Focus::Tracks
                } else {
                    Focus::Replay
                };
            }
            UiEvent::TogglePause => {
                self.command = Some(ReplayCommand::TogglePause);
            }
            UiEvent::Step(step) if self.focus == Focus::Tracks => self.select(step.signum() as i32),
            UiEvent::SelectTrack(delta) => self.select(delta),
            UiEvent::Step(step) if self.snapshot.paused || self.snapshot.complete => {
                self.command = Some(ReplayCommand::Step(step));
            }
            UiEvent::Step(_) => {
                self.notice = "Pause replay before stepping".into();
            }
            UiEvent::ChangeSpeed(direction) => {
                self.command = Some(ReplayCommand::Speed(
                    (self.snapshot.replay_speed * if direction > 0 { 2.0 } else { 0.5 })
                        .clamp(0.125, 16.0),
                ));
            }
            _ => {}
        }
    }
    fn select(&mut self, delta: i32) {
        let tracks = &self.snapshot.tracks;
        if tracks.is_empty() {
            self.selected = None;
            return;
        }
        let index = self
            .selected
            .and_then(|id| tracks.iter().position(|track| track.id == id))
            .unwrap_or(0);
        self.selected = Some(
            tracks[(index as i64 + i64::from(delta)).rem_euclid(tracks.len() as i64) as usize].id,
        );
    }
    pub fn header(&self) -> String {
        format!(
            "REPLAY {} | TRACK {} | AVG FPS {:.1} | {:?} | VIRTUAL | active {:.1}s",
            if self.snapshot.complete {
                "DONE"
            } else if self.snapshot.paused {
                "PAUSED"
            } else {
                "RUNNING"
            },
            self.selected
                .map_or_else(|| "N/A".into(), |id| id.0.to_string()),
            self.snapshot.fps.unwrap_or(0.0),
            self.focus,
            self.snapshot.elapsed_seconds
        )
    }
    pub fn safety_text(&self) -> String {
        if self.stale {
            return "SAFETY UNAVAILABLE | OUTPUT DISABLED\nDashboard telemetry is stale or disconnected".into();
        }
        match self.snapshot.safety {
            SafetyState::SafetyLockout(reason) => format!("SAFETY LOCKOUT | OUTPUT DISABLED\n{reason:?} | Recorded replay evidence; physical output disabled"),
            SafetyState::Clear => "RECORDED CLEAR | OUTPUT DISABLED\nPhysical output disabled; replay evidence grants no live permission".into(),
        }
    }
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, screen: Screen) {
        let content = match screen {
            Screen::Live => self.target_details(),
            Screen::Performance => self.performance(),
            Screen::Tracks => self.track_table(),
            Screen::Safety => self.safety_details(),
            Screen::Calibration => self.calibration_details(),
        };
        if screen == Screen::Live && area.width >= 70 && area.height >= 12 {
            let columns =
                Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .split(area);
            let legend = format!(
                "O observed  . trajectory  P predicted\n+ issued virtual aim  x suppressed request\n{} | frame {}/{} | {:.3}x",
                self.snapshot.source,
                self.snapshot.counters.processed_frames,
                self.snapshot
                    .total_frames
                    .map_or_else(|| "?".into(), |count| count.to_string()),
                self.snapshot.replay_speed
            );
            let block = Block::default()
                .title(if self.foreground {
                    "Foreground mask [m] / region"
                } else {
                    "Camera grayscale [g] ASCII [m] mask"
                })
                .borders(Borders::ALL);
            let inside = block.inner(columns[0]);
            frame.render_widget(block, columns[0]);
            let regions =
                Layout::vertical([Constraint::Min(0), Constraint::Length(5)]).split(inside);
            frame.render_widget(Viewport { view: self }, regions[0]);
            frame.render_widget(
                Paragraph::new(legend).wrap(Wrap { trim: false }),
                regions[1],
            );
            frame.render_widget(
                panel(content, "Target and latency").scroll((self.scroll, 0)),
                columns[1],
            );
        } else {
            frame.render_widget(
                panel(content, screen.title()).scroll((self.scroll, 0)),
                area,
            );
        }
    }
    fn target_details(&self) -> String {
        let s = &self.snapshot;
        let c = &s.counters;
        let mut text = format!(
            "Frame processing: {}\nFrame-to-issued-command: {}\n",
            stats(
                s.latency
                    .get(Stage::FrameProcessing.index())
                    .and_then(Option::as_ref)
            ),
            stats(
                s.latency
                    .get(Stage::FrameToCommand.index())
                    .and_then(Option::as_ref)
            )
        );
        if let Some(track) = self
            .selected
            .and_then(|id| s.tracks.iter().find(|track| track.id == id))
        {
            text.push_str(&format!("Target {} {:?} | confidence {:.3}\nAge {:.1}ms | position {:.1},{:.1}\nVelocity {:.1},{:.1} px/s | lost {}\n",track.id.0,track.lifecycle,track.confidence.get(),s.timestamp.0.saturating_sub(track.first_seen.0) as f64/1000.0,track.position.x(),track.position.y(),track.velocity.x(),track.velocity.y(),track.lost_count));
        } else {
            text.push_str("No active target\n");
        }
        if let Some(servos) = s.pan_tilt {
            text.push_str(&format!(
                "Pan/tilt SIMULATION {:?}{}\nSimulated pan/tilt: {:.2}/{:.2} deg\nRequested pan/tilt: {}\nDestination pulses: {} | channels {}/{}\nSettling remaining: {} us\n",
                servos.state,
                if self.stale { " (stale snapshot)" } else { "" },
                servos.simulated.pan.degrees(), servos.simulated.tilt.degrees(),
                servos.requested.map_or_else(|| "none".into(), |position| format!("{:.2}/{:.2} deg", position.pan.degrees(), position.tilt.degrees())),
                servos.requested_pulses.map_or_else(|| "none".into(), |pulses| format!("{}/{} us (not sent)", pulses.pan.microseconds(), pulses.tilt.microseconds())),
                servos.pan_channel, servos.tilt_channel, servos.settling_remaining_us,
            ));
        }
        text.push_str(&format!("System: {:?}\nAim: {:?}\nLast request: {}\nProcessed {} | dropped {}\nAcquired {} | lost {} | reacquired {}\nLockouts {} | issued {} | suppressed {}\n",s.system,s.aim_status,s.aim.map_or_else(||"unavailable".into(),|aim|format!("{} at {:.1},{:.1}",if aim.issued {"ISSUED VIRTUAL"}else{"SUPPRESSED"},aim.request.predicted_pixel.x(),aim.request.predicted_pixel.y())),c.processed_frames,c.dropped_frames,c.acquired_targets,c.lost_targets,c.reacquisitions,c.lockouts,c.issued_aims,c.suppressed_aims));
        text.push_str(&format!("Safety backend: {}\n", s.safety_backend));
        if let telemetry::AimStatus::WaitingVirtual {
            target,
            remaining_us,
        } = s.aim_status
        {
            text.push_str(&format!(
                "Virtual aimer: target {} | {:.0} ms remaining\n",
                target.0,
                remaining_us as f64 / 1000.0
            ));
        }
        if let Some(record) = s.aim {
            text.push_str(&format!(
                "Requested target {} | normalized XY {:.4},{:.4}\n",
                record.request.target.0,
                record.request.aim.x(),
                record.request.aim.y()
            ));
        }
        if c.truth_frames > 0 {
            text.push_str(&format!(
                "Truth FP {} | FN {} | ID switches {}\n",
                c.false_positives.unwrap_or(0),
                c.false_negatives.unwrap_or(0),
                c.id_switches
            ));
        } else {
            text.push_str("Ground-truth metrics: unavailable\n");
        }
        text.push_str("Recent events:\n");
        for event in &self.events {
            text.push_str(event);
            text.push('\n');
        }
        text
    }
    fn performance(&self) -> String {
        let s = &self.snapshot;
        let mut text = format!(
            "Exact last {} samples per metric | microseconds\nStage                 P50/P95/P99/MAX\n",
            s.sample_capacity
        );
        for stage in Stage::ALL {
            text.push_str(&format!(
                "{stage:?}: {}\n",
                stats(s.latency.get(stage.index()).and_then(Option::as_ref))
            ));
        }
        text.push_str(&format!(
            "Detection error (px): {}\n",
            stats(s.detection_error_pixels.as_ref())
        ));
        text.push_str(
            "Time-window frame processing (microseconds, approximate quantile upper bounds):\n",
        );
        for window in &s.time_windows {
            text.push_str(&format!(
                "{}: {}\n",
                window.seconds.map_or_else(
                    || "session".into(),
                    |seconds| format!("{} min", seconds / 60)
                ),
                stats(
                    window
                        .latency
                        .get(Stage::FrameProcessing.index())
                        .and_then(Option::as_ref)
                )
            ));
        }
        for (index, horizon) in [0, 5, 10, 20].into_iter().enumerate() {
            text.push_str(&format!(
                "Prediction +{horizon}ms (px): {}\n",
                stats(
                    s.prediction_error_pixels
                        .get(index)
                        .and_then(Option::as_ref)
                )
            ));
        }
        text.push_str(&format!(
            "Virtual aim error (px): {}\nContinuity: {}\n",
            stats(s.virtual_aim_error_pixels.as_ref()),
            continuity(&s.counters)
        ));
        let comparable: Vec<_> = self
            .runs
            .iter()
            .filter(|run| {
                run.identity.dataset == s.identity.dataset
                    && !s.identity.hardware.starts_with("unverified:")
                    && run.identity.config_fingerprint == s.identity.config_fingerprint
                    && run.identity.hardware == s.identity.hardware
                    && run.identity.build == s.identity.build
                    && run.identity.metric_version == s.identity.metric_version
                    && run.sample_capacity == s.sample_capacity
            })
            .collect();
        if self.runs.is_empty() {
            text.push_str("Previous / best comparable runs: unavailable\n");
        } else {
            text.push_str(&format!(
                "Comparable runs: {} | incompatible: {}\n",
                comparable.len(),
                self.runs.len() - comparable.len()
            ));
            if !s.complete {
                text.push_str("Current run incomplete: changes are unavailable until completion\n");
            }
            for stage in [Stage::FrameProcessing, Stage::FrameToCommand] {
                let previous = comparable
                    .last()
                    .and_then(|run| run.latency.get(stage.index()))
                    .and_then(Option::as_ref);
                let best = comparable
                    .iter()
                    .filter_map(|run| run.latency.get(stage.index()).and_then(Option::as_ref))
                    .min_by(|a, b| a.p99.total_cmp(&b.p99));
                let current = s.latency.get(stage.index()).and_then(Option::as_ref);
                text.push_str(&format!("{stage:?} P99 lower is better: current {} | previous {} | best {} | delta {}\n",number(current.map(|v|v.p99)),number(previous.map(|v|v.p99)),number(best.map(|v|v.p99)),current.zip(previous).filter(|_|s.complete).and_then(|(a,b)|percentage_change(a.p99,b.p99)).map_or_else(||"N/A (zero/missing baseline or incomplete)".into(),|v|format!("{v:+.1}%"))));
            }
            for (index, horizon) in [0, 5, 10, 20].into_iter().enumerate() {
                let current = s
                    .prediction_error_pixels
                    .get(index)
                    .and_then(Option::as_ref)
                    .map(|d| d.rms);
                let values: Vec<_> = comparable
                    .iter()
                    .filter_map(|run| {
                        run.prediction_error_pixels
                            .get(index)
                            .and_then(Option::as_ref)
                            .map(|d| d.rms)
                    })
                    .collect();
                text.push_str(&quality_comparison(
                    &format!("Prediction +{horizon}ms RMS px"),
                    current,
                    &values,
                    s.complete,
                    false,
                ));
            }
            let detection: Vec<_> = comparable
                .iter()
                .filter_map(|run| run.detection_error_pixels.as_ref().map(|d| d.rms))
                .collect();
            text.push_str(&quality_comparison(
                "Detection RMS px",
                s.detection_error_pixels.as_ref().map(|d| d.rms),
                &detection,
                s.complete,
                false,
            ));
            let aiming: Vec<_> = comparable
                .iter()
                .filter_map(|run| run.virtual_aim_error_pixels.as_ref().map(|d| d.rms))
                .collect();
            text.push_str(&quality_comparison(
                "Virtual aim RMS px",
                s.virtual_aim_error_pixels.as_ref().map(|d| d.rms),
                &aiming,
                s.complete,
                false,
            ));
            let continuity_values: Vec<_> = comparable
                .iter()
                .filter_map(|run| continuity_value(&run.counters))
                .collect();
            text.push_str(&quality_comparison(
                "Continuity percent",
                continuity_value(&s.counters),
                &continuity_values,
                s.complete,
                true,
            ));
            let drops: Vec<_> = comparable
                .iter()
                .map(|run| run.counters.dropped_frames as f64)
                .collect();
            text.push_str(&quality_comparison(
                "Dropped frames",
                Some(s.counters.dropped_frames as f64),
                &drops,
                s.complete,
                false,
            ));
        }
        text
    }
    fn track_table(&self) -> String {
        let mut text = "ID  STATE  AGE(ms)  CONF  SPEED(px/s)  LOST\n".to_string();
        for track in self
            .snapshot
            .tracks
            .iter()
            .chain(&self.snapshot.recent_tracks)
            .take(128)
        {
            text.push_str(&format!(
                "{}{} {:?} {:.1} {:.3} {:.1} {}\n",
                if self.selected == Some(track.id) {
                    ">"
                } else {
                    " "
                },
                track.id.0,
                track.lifecycle,
                self.snapshot.timestamp.0.saturating_sub(track.first_seen.0) as f64 / 1000.0,
                track.confidence.get(),
                track.velocity.x().hypot(track.velocity.y()),
                track.lost_count
            ));
        }
        text.push_str("Removed/lost reason: unmatched observations\nSelection: Tab focus, arrows\nTrajectory: O measured, P predicted; no separate position filter estimate\n");
        text
    }
    fn safety_details(&self) -> String {
        format!(
            "{}\nSource clock: {}us | frame {}\nCamera: {}\nBackend: {}\nWatchdog: checked at command execution\nOutput interlock: virtual commands only\nPositive frames {} | unsafe commands {}\nPhysical safety authority: unavailable\nNo output override exists",
            self.safety_text(),
            self.snapshot.timestamp.0,
            self.snapshot.frame.0,
            if self.snapshot.camera_available {
                "replay source available"
            } else {
                "replay complete"
            },
            self.snapshot.safety_backend,
            self.snapshot.counters.safety_positive_frames,
            self.snapshot.counters.unsafe_commands
        )
    }
    fn calibration_details(&self) -> String {
        let mut text = format!(
            "Calibration version: {}\nMode: virtual plane or loaded mapping\nResidual RMS/max: {}\nPhysical point acquisition: unavailable\nAutomatic wall calibration: unavailable\nRetry/restart/save controls: unavailable until hardware is selected",
            self.snapshot
                .calibration_version
                .map_or_else(|| "unavailable".into(), |v| v.to_string()),
            self.snapshot.calibration_residuals.map_or_else(
                || "unavailable (no measured points)".into(),
                |v| format!("{:.6}/{:.6} normalized units", v.rms, v.maximum)
            )
        );
        if let Some(calibration) = &self.snapshot.calibration {
            text.push_str(&format!("\nCreated UTC: {}\nCamera dimensions: {}x{}\nCamera-to-normalized matrix:\n{:?}\n{:?}\n{:?}\nRejected aim mappings: {}",calibration.created_utc,calibration.camera_size.width(),calibration.camera_size.height(),calibration.matrix[0],calibration.matrix[1],calibration.matrix[2],self.snapshot.counters.calibration_rejections));
        }
        text
    }
}
fn continuity_value(c: &telemetry::Counters) -> Option<f64> {
    if c.truth_target_observations == 0 {
        None
    } else {
        Some(c.tracked_truth_observations as f64 / c.truth_target_observations as f64 * 100.0)
    }
}
fn quality_comparison(
    label: &str,
    current: Option<f64>,
    values: &[f64],
    complete: bool,
    higher_is_better: bool,
) -> String {
    let previous = values.last().copied();
    let best = values
        .iter()
        .copied()
        .reduce(|a, b| if higher_is_better { a.max(b) } else { a.min(b) });
    let change = current.zip(previous).filter(|_| complete).map_or_else(
        || "unavailable".into(),
        |(a, b)| {
            if higher_is_better {
                format!("{:+.2} percentage points", a - b)
            } else {
                percentage_change(a, b).map_or_else(
                    || "unavailable (zero baseline)".into(),
                    |value| format!("{value:+.2}%"),
                )
            }
        },
    );
    format!(
        "{label} ({} is better): current {} | previous {} | best {} | delta {}\n",
        if higher_is_better { "higher" } else { "lower" },
        number(current),
        number(previous),
        number(best),
        change
    )
}
struct Viewport<'a> {
    view: &'a ConnectedView,
}
impl Widget for Viewport<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let size = self.view.snapshot.camera_size;
        let image = self.view.image.render(area, size, self.view.ascii, buffer);
        let mut mark = |point: PixelPosition, symbol: char, color: ratatui::style::Color| {
            if let Some((x, y)) = crate::viewport::project(point, size, image) {
                buffer[(x, y)]
                    .set_char(symbol)
                    .set_fg(color)
                    .set_bg(ratatui::style::Color::Black);
            }
        };
        if let Some(path) = self
            .view
            .selected
            .and_then(|id| self.view.trajectories.get(&id))
        {
            for point in path {
                mark(*point, '.', ratatui::style::Color::DarkGray);
            }
        }
        for track in &self.view.snapshot.tracks {
            mark(track.position, 'O', ratatui::style::Color::Cyan);
        }
        for (_, point) in &self.view.snapshot.predictions {
            mark(*point, 'P', ratatui::style::Color::Cyan);
        }
        if let Some(record) = self
            .view
            .snapshot
            .aim
            .filter(|record| record.request.frame == self.view.snapshot.frame)
        {
            mark(
                record.request.predicted_pixel,
                if record.issued
                    && !self.view.stale
                    && self.view.snapshot.safety == SafetyState::Clear
                {
                    '+'
                } else {
                    'x'
                },
                if record.issued
                    && !self.view.stale
                    && self.view.snapshot.safety == SafetyState::Clear
                {
                    ratatui::style::Color::Cyan
                } else {
                    ratatui::style::Color::Red
                },
            );
        }
    }
}
fn panel(text: String, title: &str) -> Paragraph<'_> {
    Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default().title(title).borders(Borders::ALL))
}
fn stats(value: Option<&Distribution>) -> String {
    value.map_or_else(
        || "unavailable".into(),
        |v| {
            format!(
                "{:.2}/{:.2}/{:.2}/{:.2} (n={}/{})",
                v.p50, v.p95, v.p99, v.maximum, v.count, v.total_samples
            )
        },
    )
}
fn number(value: Option<f64>) -> String {
    value.map_or_else(|| "N/A".into(), |v| format!("{v:.2}"))
}
fn continuity(c: &telemetry::Counters) -> String {
    if c.truth_target_observations == 0 {
        "unavailable".into()
    } else {
        format!(
            "{:.2}% ({}/{})",
            c.tracked_truth_observations as f64 / c.truth_target_observations as f64 * 100.0,
            c.tracked_truth_observations,
            c.truth_target_observations
        )
    }
}

pub fn safety_style(view: &ConnectedView) -> ratatui::style::Style {
    if view.stale || matches!(view.snapshot.safety, SafetyState::SafetyLockout(_)) {
        theme::fault()
    } else {
        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow)
    }
}
