// Gantt tab — wall-clock timeline of the whole run: one bar per job, one
// bar per step, positioned on a shared time axis so parallelism, gaps and
// the critical path are visible at a glance.
//
// The executor stamps `started_at`/`finished_at` on every `JobResult` and
// `StepResult` (see `run_step_with_guards`); rows without stamps (skipped
// jobs, synthetic validation results) render as label-only rows with "—".
//
// Layout per row:  NAME  DURATION │ CHART
// The chart maps [t0, t1] (earliest start .. latest finish) onto the
// available columns; every bar that ran gets at least one cell so
// sub-cell steps stay visible.

use crate::app::App;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::time::SystemTime;
use wrkflw_executor::{JobStatus, StepStatus};

/// One renderable row of the chart.
struct GanttRow {
    label: String,
    /// 0 = job, 1 = step (indented, dimmer).
    depth: usize,
    /// Milliseconds relative to the run's t0. `None` = never ran.
    span_ms: Option<(u64, u64)>,
    color: Color,
    duration_label: String,
}

pub fn render_gantt_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let block = theme::block("Gantt — job & step timeline");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    let Some(idx) = workflow_idx else {
        render_message(f, inner, "no workflow selected");
        return;
    };
    let workflow = &app.workflows[idx];
    let Some(exec) = workflow.execution_details.as_ref() else {
        render_message(f, inner, "no run yet — execute the workflow first");
        return;
    };

    // Live progress rows stream in while the run executes; before the
    // first job event, show the elapsed clock instead of an empty chart.
    if exec.jobs.is_empty() {
        let elapsed = chrono::Local::now()
            .signed_duration_since(exec.start_time)
            .num_seconds()
            .max(0);
        render_message(
            f,
            inner,
            &format!(
                "run in progress — {:02}:{:02} elapsed (waiting for the first job to start)",
                elapsed / 60,
                elapsed % 60
            ),
        );
        return;
    }

    // ── Time domain ───────────────────────────────────────────────
    // A row without `finished_at` is still running — its bar (and the
    // axis) extends to "now", so live bars grow with each frame.
    let now = SystemTime::now();
    let mut starts: Vec<SystemTime> = Vec::new();
    let mut ends: Vec<SystemTime> = Vec::new();
    for job in &exec.jobs {
        if let Some(s) = job.started_at {
            starts.push(s);
            ends.push(job.finished_at.unwrap_or(now));
        }
        for step in &job.steps {
            if let Some(s) = step.started_at {
                starts.push(s);
                ends.push(step.finished_at.unwrap_or(now));
            }
        }
    }
    let (Some(&t0), Some(&t1)) = (starts.iter().min(), ends.iter().max()) else {
        render_message(f, inner, "this run carries no timing data (re-run to capture it)");
        return;
    };
    let total_ms = t1.duration_since(t0).map(|d| d.as_millis() as u64).unwrap_or(0);

    let ms_from_t0 = |t: SystemTime| -> u64 {
        t.duration_since(t0).map(|d| d.as_millis() as u64).unwrap_or(0)
    };

    // ── Rows ──────────────────────────────────────────────────────
    let mut rows: Vec<GanttRow> = Vec::new();
    for job in &exec.jobs {
        rows.push(GanttRow {
            label: job.name.clone(),
            depth: 0,
            span_ms: job
                .started_at
                .map(|s| (ms_from_t0(s), ms_from_t0(job.finished_at.unwrap_or(now)))),
            color: if job.is_running() {
                COLORS.info
            } else {
                job_color(&job.status)
            },
            duration_label: duration_label(job.started_at, job.finished_at, now),
        });
        for step in &job.steps {
            rows.push(GanttRow {
                label: step.name.clone(),
                depth: 1,
                span_ms: step
                    .started_at
                    .map(|s| (ms_from_t0(s), ms_from_t0(step.finished_at.unwrap_or(now)))),
                color: if step.is_running() {
                    COLORS.info
                } else {
                    step_color(&step.status)
                },
                duration_label: duration_label(step.started_at, step.finished_at, now),
            });
        }
    }

    // ── Geometry ──────────────────────────────────────────────────
    if inner.width < 32 || inner.height < 4 {
        render_message(f, inner, "window too small");
        return;
    }
    let name_w = (inner.width as usize / 3).clamp(16, 30);
    let dur_w = 8usize;
    // gutter(2) + NAME + ' ' + DUR + ' │' + chart
    let chart_w = inner.width as usize - 2 - name_w - 1 - dur_w - 2;

    // Header (summary) + ruler take the first two lines.
    let body_h = inner.height as usize - 2;
    // Clamp the row cursor; after a keyboard move, scroll it into view
    // (wheel scrolling doesn't set the flag, so the viewport can roam).
    if app.gantt_selected >= rows.len() {
        app.gantt_selected = rows.len().saturating_sub(1);
    }
    if app.gantt_follow {
        if app.gantt_selected < app.gantt_scroll {
            app.gantt_scroll = app.gantt_selected;
        }
        if app.gantt_selected + 1 > app.gantt_scroll + body_h {
            app.gantt_scroll = app.gantt_selected + 1 - body_h;
        }
        app.gantt_follow = false;
    }
    let max_scroll = rows.len().saturating_sub(body_h);
    if app.gantt_scroll > max_scroll {
        app.gantt_scroll = max_scroll;
    }
    let scroll = app.gantt_scroll;

    let mut lines: Vec<Line> = Vec::with_capacity(body_h + 2);

    // ── Summary line ──────────────────────────────────────────────
    let jobs_n = exec.jobs.len();
    let steps_n: usize = exec.jobs.iter().map(|j| j.steps.len()).sum();
    let longest = exec
        .jobs
        .iter()
        .filter_map(|j| match (j.started_at, j.finished_at) {
            (Some(s), Some(e)) => e.duration_since(s).ok().map(|d| (j.name.as_str(), d)),
            _ => None,
        })
        .max_by_key(|(_, d)| *d);
    let mut summary = vec![
        Span::styled(
            format!("total {}", fmt_ms(total_ms)),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  {} jobs, {} steps", jobs_n, steps_n),
            Style::default().fg(COLORS.text_dim),
        ),
    ];
    if let Some((name, d)) = longest {
        summary.push(Span::styled(
            format!("  ·  longest job: {} ({})", name, fmt_ms(d.as_millis() as u64)),
            Style::default().fg(COLORS.text_dim),
        ));
    }
    if rows.len() > body_h {
        summary.push(Span::styled(
            format!(
                "  ·  rows {}–{}/{} (wheel/↑↓)",
                scroll + 1,
                (scroll + body_h).min(rows.len()),
                rows.len()
            ),
            Style::default().fg(COLORS.text_muted),
        ));
    }
    lines.push(Line::from(summary));

    // ── Ruler ─────────────────────────────────────────────────────
    lines.push(ruler_line(name_w, dur_w, chart_w, total_ms));

    // ── Bars ──────────────────────────────────────────────────────
    for (i, row) in rows.iter().enumerate().skip(scroll).take(body_h) {
        lines.push(row_line(
            row,
            i == app.gantt_selected,
            name_w,
            dur_w,
            chart_w,
            total_ms,
        ));
    }

    // Record the pane for wheel routing and the row area for click-select.
    let shown = rows.len().saturating_sub(scroll).min(body_h) as u16;
    {
        let mut zones = app.mouse_zones.borrow_mut();
        zones.gantt = Some(inner);
        zones.gantt_rows = Some((
            Rect {
                x: inner.x,
                y: inner.y + 2,
                width: inner.width,
                height: shown,
            },
            scroll,
            rows.len(),
        ));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn row_line(
    row: &GanttRow,
    selected: bool,
    name_w: usize,
    dur_w: usize,
    chart_w: usize,
    total_ms: u64,
) -> Line<'static> {
    let (indent, mut name_style, bar_char) = if row.depth == 0 {
        (
            "",
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
            '█',
        )
    } else {
        ("  ", Style::default().fg(COLORS.text_dim), '▓')
    };
    if selected {
        name_style = name_style
            .fg(COLORS.text)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED);
    }
    let name = format!("{}{}", indent, row.label);

    let mut spans = vec![
        Span::styled(
            if selected { "▶ " } else { "  " },
            Style::default().fg(theme::current_accent()),
        ),
        Span::styled(pad_or_trim(&name, name_w), name_style),
        Span::raw(" "),
        Span::styled(
            format!("{:>width$}", row.duration_label, width = dur_w),
            Style::default().fg(COLORS.text_muted),
        ),
        Span::styled(" │", Style::default().fg(COLORS.border)),
    ];

    match row.span_ms {
        Some((start, end)) => {
            let (offset, len) = bar_cells(start, end, total_ms, chart_w);
            spans.push(Span::styled(
                " ".repeat(offset),
                Style::default(),
            ));
            spans.push(Span::styled(
                bar_char.to_string().repeat(len),
                Style::default().fg(row.color),
            ));
        }
        None => {
            spans.push(Span::styled(
                " —".to_string(),
                Style::default().fg(COLORS.text_muted),
            ));
        }
    }
    Line::from(spans)
}

/// Ruler with tick labels at 0 / ¼ / ½ / ¾ / total.
fn ruler_line(name_w: usize, dur_w: usize, chart_w: usize, total_ms: u64) -> Line<'static> {
    let mut ruler: Vec<char> = "─".repeat(chart_w).chars().collect();
    let mut place = |at: usize, text: &str| {
        // Right-align the last label so it stays inside the chart.
        let start = if at + text.len() > chart_w {
            chart_w.saturating_sub(text.len())
        } else {
            at
        };
        for (i, ch) in text.chars().enumerate() {
            if start + i < ruler.len() {
                ruler[start + i] = ch;
            }
        }
    };
    if chart_w >= 24 {
        for q in [0usize, 1, 2, 3, 4] {
            let at = chart_w.saturating_sub(1) * q / 4;
            let label = fmt_ms(total_ms * q as u64 / 4);
            place(at, &label);
        }
    } else {
        place(0, "0s");
        place(chart_w, &fmt_ms(total_ms));
    }
    Line::from(vec![
        Span::raw(" ".repeat(2 + name_w + 1 + dur_w)),
        Span::styled(" ╽", Style::default().fg(COLORS.border)),
        Span::styled(
            ruler.into_iter().collect::<String>(),
            Style::default().fg(COLORS.text_muted),
        ),
    ])
}

fn render_message(f: &mut Frame<'_>, area: Rect, msg: &str) {
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            msg.to_string(),
            Style::default().fg(COLORS.text_muted),
        ))),
        area,
    );
}

fn job_color(status: &JobStatus) -> Color {
    match status {
        JobStatus::Success => COLORS.success,
        JobStatus::Failure => COLORS.error,
        JobStatus::Skipped => COLORS.warning,
    }
}

fn step_color(status: &StepStatus) -> Color {
    match status {
        StepStatus::Success => COLORS.success,
        StepStatus::Failure => COLORS.error,
        StepStatus::Skipped => COLORS.warning,
    }
}

/// Duration text for a row; a started-but-unfinished row ticks against `now`.
fn duration_label(start: Option<SystemTime>, end: Option<SystemTime>, now: SystemTime) -> String {
    match start {
        Some(s) => {
            let e = end.unwrap_or(now);
            fmt_ms(e.duration_since(s).map(|d| d.as_millis() as u64).unwrap_or(0))
        }
        None => "—".to_string(),
    }
}

/// Map a [start, end] span (ms relative to t0) onto `width` columns.
/// Returns (offset, len); every span that ran gets at least one cell,
/// clamped so offset + len never exceeds the width.
fn bar_cells(start_ms: u64, end_ms: u64, total_ms: u64, width: usize) -> (usize, usize) {
    if width == 0 {
        return (0, 0);
    }
    if total_ms == 0 {
        return (0, width.min(1));
    }
    let scale = |ms: u64| -> usize { ((ms as u128 * width as u128) / total_ms as u128) as usize };
    let offset = scale(start_ms).min(width - 1);
    let end_cell = scale(end_ms.max(start_ms)).clamp(offset + 1, width);
    (offset, end_cell - offset)
}

/// Human duration from milliseconds: "0.4s", "12.3s", "1m 04s", "1h 02m".
/// Shared with the Timing panes (execution tab, step inspector).
pub(crate) fn fmt_ms(ms: u64) -> String {
    let secs = ms / 1000;
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else if secs >= 10 {
        format!("{}s", secs)
    } else {
        format!("{}.{}s", secs, (ms % 1000) / 100)
    }
}

fn pad_or_trim(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n && n > 0 {
        out.pop();
        out.push('…');
    }
    while out.chars().count() < n {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_cells_spans_full_width() {
        assert_eq!(bar_cells(0, 1000, 1000, 40), (0, 40));
    }

    #[test]
    fn bar_cells_minimum_one_cell() {
        // A 1ms step inside a 100s run still shows up.
        assert_eq!(bar_cells(50_000, 50_001, 100_000, 40).1, 1);
    }

    #[test]
    fn bar_cells_offset_positions_bar() {
        let (offset, len) = bar_cells(500, 1000, 1000, 40);
        assert_eq!(offset, 20);
        assert_eq!(offset + len, 40);
    }

    #[test]
    fn bar_cells_never_overflows_width() {
        for (s, e) in [(999, 1000), (1000, 1000), (0, 1)] {
            let (offset, len) = bar_cells(s, e, 1000, 7);
            assert!(offset + len <= 7, "({s},{e}) -> ({offset},{len})");
            assert!(len >= 1);
        }
    }

    #[test]
    fn bar_cells_zero_total_is_safe() {
        assert_eq!(bar_cells(0, 0, 0, 40), (0, 1));
        assert_eq!(bar_cells(0, 0, 0, 0), (0, 0));
    }

    #[test]
    fn fmt_ms_ranges() {
        assert_eq!(fmt_ms(400), "0.4s");
        assert_eq!(fmt_ms(12_340), "12s");
        assert_eq!(fmt_ms(9_800), "9.8s");
        assert_eq!(fmt_ms(64_000), "1m 04s");
        assert_eq!(fmt_ms(3_720_000), "1h 02m");
    }

    #[test]
    fn pad_or_trim_marks_truncation() {
        assert_eq!(pad_or_trim("abcdef", 4), "abc…");
        assert_eq!(pad_or_trim("ab", 4), "ab  ");
    }

    #[test]
    fn row_line_positions_bar_on_the_time_axis() {
        let row = GanttRow {
            label: "build".to_string(),
            depth: 0,
            span_ms: Some((500, 1000)),
            color: COLORS.success,
            duration_label: "0.5s".to_string(),
        };
        let line = row_line(&row, false, 16, 8, 40, 1000);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // gutter(2) + name(16) + ' ' + dur(8) + ' │' + 20 spaces + 20 cells
        let chart = &text[text.find('│').unwrap() + '│'.len_utf8()..];
        assert!(chart.starts_with(&" ".repeat(20)), "chart: {:?}", chart);
        assert_eq!(chart.chars().filter(|&c| c == '█').count(), 20);
    }

    #[test]
    fn row_line_renders_dash_for_rows_without_timing() {
        let row = GanttRow {
            label: "skipped".to_string(),
            depth: 1,
            span_ms: None,
            color: COLORS.warning,
            duration_label: "—".to_string(),
        };
        let line = row_line(&row, false, 16, 8, 40, 1000);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.ends_with(" —"), "text: {:?}", text);
        assert!(!text.contains('▓'));
    }

    #[test]
    fn row_line_marks_the_selected_row() {
        let row = GanttRow {
            label: "build".to_string(),
            depth: 0,
            span_ms: Some((0, 1000)),
            color: COLORS.success,
            duration_label: "1.0s".to_string(),
        };
        let line = row_line(&row, true, 16, 8, 40, 1000);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("▶ "), "text: {:?}", text);
    }

    #[test]
    fn ruler_labels_start_and_total() {
        let line = ruler_line(16, 8, 40, 10_000);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("0s"), "ruler: {:?}", text);
        assert!(text.contains("10s"), "ruler: {:?}", text);
    }
}
