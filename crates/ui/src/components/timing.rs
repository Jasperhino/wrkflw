// Per-job timing chart — horizontal bars sized against the longest run.
//
// Rows carry an optional `weight` (this row's duration relative to the
// longest, from the executor's wall-clock stamps). With a weight the bar
// length is proportional; without one (skipped jobs, results from before
// timing landed) we fall back to uniform status-coloured bars.

use crate::theme::COLORS;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use wrkflw_executor::JobStatus;

#[derive(Clone)]
pub struct TimingRow<'a> {
    pub name: &'a str,
    pub status: Option<JobStatus>, // None = pending
    pub label: &'a str,            // e.g. "1m 47s" or "—"
    /// Duration relative to the longest row, 0.0–1.0. `None` = no timing
    /// data, fall back to the status-based fill.
    pub weight: Option<f32>,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, rows: &[TimingRow]) {
    if area.width < 12 {
        return;
    }
    // We aim for: NAME (10) | BAR (rest - 6) | LABEL (5)
    let bar_width = area.width.saturating_sub(18) as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for row in rows {
        let (color, status_fill) = bar_props(row.status.clone());
        // A real duration weight overrides the status-based fill; keep at
        // least a sliver visible for rows that did run.
        let fill = match row.weight {
            Some(w) if row.status.is_some() => w.clamp(0.0, 1.0).max(0.02),
            _ => status_fill,
        };
        let filled = (fill * bar_width as f32).round() as usize;
        let empty = bar_width.saturating_sub(filled);

        lines.push(Line::from(vec![
            Span::styled(
                pad_right(row.name, 10),
                Style::default().fg(COLORS.text_dim),
            ),
            Span::styled("█".repeat(filled), Style::default().fg(color)),
            Span::styled("·".repeat(empty), Style::default().fg(COLORS.border)),
            Span::raw(" "),
            Span::styled(
                pad_left(row.label, 5),
                Style::default().fg(COLORS.text_muted),
            ),
        ]));
    }
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no jobs yet",
            Style::default().fg(COLORS.text_muted),
        )));
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "critical path ",
                Style::default()
                    .fg(COLORS.text_muted)
                    .add_modifier(Modifier::DIM),
            ),
            Span::styled(summarise(rows), Style::default().fg(COLORS.text_dim)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn bar_props(s: Option<JobStatus>) -> (ratatui::style::Color, f32) {
    match s {
        Some(JobStatus::Success) => (COLORS.success, 1.0),
        Some(JobStatus::Failure) => (COLORS.error, 1.0),
        Some(JobStatus::Skipped) => (COLORS.warning, 0.4),
        None => (COLORS.info, 0.0), // pending — empty
    }
}

fn pad_right(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    while out.chars().count() < n {
        out.push(' ');
    }
    out
}

fn pad_left(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count >= n {
        return s.to_string();
    }
    let mut out = String::new();
    for _ in 0..(n - count) {
        out.push(' ');
    }
    out.push_str(s);
    out
}

fn summarise(rows: &[TimingRow]) -> String {
    let names: Vec<&str> = rows
        .iter()
        .filter(|r| matches!(r.status, Some(JobStatus::Success | JobStatus::Failure)))
        .map(|r| r.name)
        .collect();
    if names.is_empty() {
        "(awaiting first job)".to_string()
    } else {
        names.join(" → ")
    }
}
