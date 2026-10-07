use ratatui::layout::{Constraint, Layout, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutClass {
    Large,
    Medium,
    Minimal,
}

pub fn class(area: Rect) -> LayoutClass {
    if area.width >= 120 && area.height >= 35 {
        LayoutClass::Large
    } else if area.width >= 80 && area.height >= 24 {
        LayoutClass::Medium
    } else {
        LayoutClass::Minimal
    }
}

pub fn regions(area: Rect) -> [Rect; 4] {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(if area.height >= 8 { 3 } else { 1 }),
        Constraint::Min(0),
        Constraint::Length(u16::from(area.height >= 4)),
    ])
    .split(area);
    [rows[0], rows[1], rows[2], rows[3]]
}
