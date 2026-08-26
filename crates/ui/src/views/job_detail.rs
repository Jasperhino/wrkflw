// Step inspector — tabbed sub-view inside the Execution tab.
//
// Mirrors `StepDetailScreen` from screens-core.jsx of the design handoff:
// breadcrumb header, sub-tabs for Output / Env / Files / Matrix / Timeline.
// Output renders the step's command + stdout/stderr as separate styled
// sections (`c` toggles the command); Env shows the masked environment the
// step saw; Files shows its $GITHUB_OUTPUT/ENV/PATH writes; Matrix reads
// `Job.strategy`; Timeline reuses the timing chart with one row per step.

use crate::app::App;
use crate::components::timing::{self, TimingRow};
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use wrkflw_executor::{JobStatus, StepStatus};

const TABS: [&str; 5] = ["Output", "Env", "Files", "Matrix", "Timeline"];

pub fn render_job_detail_view(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    let Some(workflow_idx) = workflow_idx else {
        return;
    };
    let workflow = &app.workflows[workflow_idx];
    let Some(job_idx) = app.job_list_state.selected() else {
        return;
    };
    // The pane index is a job_names index — resolve the execution entry by
    // name (execution.jobs is in execution order, not pane order; the helper
    // also returns None while there is no execution yet).
    let Some(job) = workflow.job_execution_at(job_idx) else {
        return;
    };

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // breadcrumb
            Constraint::Length(2), // tabs
            Constraint::Min(0),    // body
        ])
        .margin(1)
        .split(area);

    render_breadcrumb(f, &workflow.name, job, outer[0]);
    render_tab_strip(f, app.step_inspector_tab, outer[1]);

    let selected_step_idx = app
        .step_table_state
        .selected()
        .filter(|&i| i < job.steps.len());

    match app.step_inspector_tab {
        0 => render_output_pane(f, app, job, selected_step_idx, outer[2]),
        1 => render_env_pane(f, app, job, selected_step_idx, outer[2]),
        2 => render_files_pane(f, app, job, selected_step_idx, outer[2]),
        3 => render_matrix_pane(f, workflow, &job.name, outer[2]),
        4 => render_timeline_pane(f, job, outer[2]),
        _ => {}
    }
}

fn render_breadcrumb(
    f: &mut Frame<'_>,
    workflow_name: &str,
    job: &crate::models::JobExecution,
    area: Rect,
) {
    let (sym, sym_style) = theme::job_status(&job.status);
    let status_text = match job.status {
        JobStatus::Success => ("success", BadgeKind::Success),
        JobStatus::Failure => ("failed", BadgeKind::Error),
        JobStatus::Skipped => ("skipped", BadgeKind::Warning),
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(20)])
        .split(area);

    let left = Paragraph::new(Line::from(vec![
        Span::styled(
            workflow_name.to_string(),
            Style::default().fg(COLORS.text_muted),
        ),
        Span::styled(" / ", Style::default().fg(COLORS.text_muted)),
        Span::styled(sym.to_string(), sym_style),
        Span::raw(" "),
        Span::styled(
            job.name.clone(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   ({} steps)", job.steps.len()),
            Style::default().fg(COLORS.text_muted),
        ),
    ]))
    .alignment(Alignment::Left);
    f.render_widget(left, chunks[0]);

    let right = Paragraph::new(Line::from(vec![theme::badge_outline(
        status_text.0,
        status_text.1,
    )]))
    .alignment(Alignment::Right);
    f.render_widget(right, chunks[1]);
}

fn render_tab_strip(f: &mut Frame<'_>, active: usize, area: Rect) {
    let mut spans: Vec<Span> = Vec::with_capacity(TABS.len() * 3);
    for (i, label) in TABS.iter().enumerate() {
        let is_active = i == active;
        spans.push(Span::styled(
            format!(" {} ", label),
            if is_active {
                Style::default()
                    .fg(theme::current_accent())
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(COLORS.text_dim)
            },
        ));
        if i + 1 < TABS.len() {
            spans.push(Span::styled("·", Style::default().fg(COLORS.text_muted)));
        }
    }
    spans.push(Span::raw("  "));
    spans.push(theme::key_chip("Tab"));
    spans.push(Span::styled(
        " switch  ",
        Style::default().fg(COLORS.text_muted),
    ));

    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
}

// ─── Output pane (default) ────────────────────────────────────────
fn render_output_pane(
    f: &mut Frame<'_>,
    app: &App,
    job: &crate::models::JobExecution,
    selected_step: Option<usize>,
    area: Rect,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(0)])
        .split(area);

    render_steps_list(f, app, job, selected_step, cols[0]);
    render_step_stdout(f, app, job, selected_step, cols[1]);
}

fn render_steps_list(
    f: &mut Frame<'_>,
    app: &App,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    area: Rect,
) {
    let block = theme::block("Steps");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    app.mouse_zones.borrow_mut().step_rows = Some((inner_area, job.steps.len()));

    let mut lines: Vec<Line> = Vec::new();
    for (i, step) in job.steps.iter().enumerate() {
        let (sym, sym_style) = if step.is_running() {
            (
                theme::spinner(app.spinner_frame),
                Style::default().fg(COLORS.info),
            )
        } else {
            theme::step_status(&step.status)
        };
        let highlighted = selected == Some(i);
        let row_style = if highlighted {
            theme::selected_style()
        } else {
            Style::default()
        };
        let name_style = match step.status {
            StepStatus::Skipped if !step.is_running() => Style::default().fg(COLORS.text_muted),
            _ => Style::default().fg(COLORS.text),
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:02} ", i + 1),
                Style::default().fg(COLORS.text_muted).patch(row_style),
            ),
            Span::styled(sym.to_string(), sym_style),
            Span::raw(" "),
            Span::styled(step.name.clone(), name_style.patch(row_style)),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no steps",
            Style::default().fg(COLORS.text_muted),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner_area);
}

/// The executor formats run-step output as
/// `Command: <cmd>\n\nStandard Output:\n…\nStandard Error:\n…` — parse it
/// back into sections so they can be rendered (and toggled) separately.
/// Output that doesn't match the format lands whole in `stdout`.
struct ParsedStepOutput {
    command: Option<String>,
    stdout: String,
    stderr: String,
}

fn parse_step_output(raw: &str) -> ParsedStepOutput {
    enum S {
        Cmd,
        Out,
        Err,
    }
    let mut command: Option<String> = None;
    let mut stdout: Vec<&str> = Vec::new();
    let mut stderr: Vec<&str> = Vec::new();
    let mut state = S::Out;
    for (i, line) in raw.split('\n').enumerate() {
        if i == 0 {
            if let Some(rest) = line.strip_prefix("Command: ") {
                command = Some(rest.to_string());
                state = S::Cmd;
                continue;
            }
        }
        match line {
            "Standard Output:" => {
                state = S::Out;
                continue;
            }
            "Standard Error:" => {
                state = S::Err;
                continue;
            }
            _ => {}
        }
        match state {
            // A multi-line `run:` block continues until the first blank
            // line (the executor writes "\n\n" after the command).
            S::Cmd => {
                if line.is_empty() {
                    state = S::Out;
                } else if let Some(c) = command.as_mut() {
                    c.push('\n');
                    c.push_str(line);
                }
            }
            S::Out => stdout.push(line),
            S::Err => stderr.push(line),
        }
    }
    let join_trimmed = |mut v: Vec<&str>| -> String {
        while v.last() == Some(&"") {
            v.pop();
        }
        v.join("\n")
    };
    ParsedStepOutput {
        command,
        stdout: join_trimmed(stdout),
        stderr: join_trimmed(stderr),
    }
}

fn render_step_stdout(
    f: &mut Frame<'_>,
    app: &App,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    area: Rect,
) {
    let step = selected.and_then(|i| job.steps.get(i));
    let title = match step {
        Some(s) => format!(
            "output — {}  (c: {})",
            s.name,
            if app.step_show_command {
                "hide command"
            } else {
                "show command"
            }
        ),
        None => "output".to_string(),
    };
    let block = theme::block_focused(&title);
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let Some(step) = step else {
        render_pane_message(f, inner_area, "(select a step on the left)");
        return;
    };
    if step.output.is_empty() {
        render_pane_message(f, inner_area, "(no output captured for this step)");
        return;
    }

    // Step output is raw process output — strip ANSI/control sequences or
    // they repaint outside the pane and ghost across views.
    let mut output = crate::log_processor::LogProcessor::strip_ansi(&step.output);
    if output.len() > 8000 {
        // Truncate on a char boundary — a fixed byte slice panics mid-UTF-8.
        let mut end = 8000;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output = format!("{}…[truncated]", &output[..end]);
    }

    let parsed = parse_step_output(&output);
    let mut rows: Vec<(String, Style)> = Vec::new();

    if app.step_show_command {
        if let Some(cmd) = &parsed.command {
            for (i, line) in cmd.split('\n').enumerate() {
                rows.push((
                    format!("{}{}", if i == 0 { "❯ " } else { "  " }, line),
                    Style::default()
                        .fg(theme::current_accent())
                        .add_modifier(Modifier::BOLD),
                ));
            }
            rows.push((String::new(), Style::default()));
        }
    }

    let has_err = !parsed.stderr.is_empty();
    if !parsed.stdout.is_empty() {
        // The stdout header only earns its line when stderr needs
        // separating from it — clean output stays clean.
        if has_err {
            rows.push((
                "─ stdout ─".to_string(),
                Style::default().fg(COLORS.text_muted),
            ));
        }
        for line in parsed.stdout.split('\n') {
            rows.push((line.to_string(), Style::default().fg(COLORS.text_dim)));
        }
    }
    if has_err {
        if !parsed.stdout.is_empty() {
            rows.push((String::new(), Style::default()));
        }
        rows.push((
            "─ stderr ─".to_string(),
            Style::default()
                .fg(COLORS.error)
                .add_modifier(Modifier::BOLD),
        ));
        for line in parsed.stderr.split('\n') {
            rows.push((line.to_string(), Style::default().fg(COLORS.error)));
        }
    }
    if rows.is_empty() {
        render_pane_message(f, inner_area, "(step produced no output)");
        return;
    }

    render_scroll_pane(f, app, inner_area, &rows);
}

fn render_pane_message(f: &mut Frame<'_>, area: Rect, msg: &str) {
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            msg.to_string(),
            Style::default().fg(COLORS.text_muted),
        ))),
        area,
    );
}

/// Shared body renderer for the inspector's scrollable panes (Output, Env,
/// Files): wraps pre-styled logical lines manually so every rendered row
/// maps 1:1 onto an addressable entry — that mapping is what makes wheel
/// scrolling (`step_output_scroll`) and drag-to-copy work.
fn render_scroll_pane(f: &mut Frame<'_>, app: &App, inner_area: Rect, rows: &[(String, Style)]) {
    let width = inner_area.width.max(1) as usize;
    let wrapped: Vec<(String, Style)> = rows
        .iter()
        .flat_map(|(text, style)| {
            let chars: Vec<char> = text.chars().collect();
            if chars.is_empty() {
                vec![(String::new(), *style)]
            } else {
                chars
                    .chunks(width)
                    .map(|c| (c.iter().collect::<String>(), *style))
                    .collect::<Vec<_>>()
            }
        })
        .collect();

    let max_rows = inner_area.height as usize;
    let total = wrapped.len();
    let scroll = app
        .step_output_scroll
        .min(total.saturating_sub(max_rows.min(total)));
    let end = (scroll + max_rows).min(total);
    let drag = app.drag_range(crate::app::CopyPane::StepOutput);
    let lines: Vec<Line> = wrapped[scroll..end]
        .iter()
        .enumerate()
        .map(|(pos, (l, style))| {
            let idx = scroll + pos;
            let style = match drag {
                Some((lo, hi)) if idx >= lo && idx <= hi => theme::selected_style(),
                _ => *style,
            };
            Line::from(Span::styled(l.clone(), style))
        })
        .collect();

    {
        let mut zones = app.mouse_zones.borrow_mut();
        zones.step_output_window = Some((inner_area, scroll));
        zones.step_output_lines = wrapped.into_iter().map(|(l, _)| l).collect();
    }

    f.render_widget(Paragraph::new(lines), inner_area);
}

// ─── Env pane ─────────────────────────────────────────────────────
//
// The executor snapshots the environment each step sees (job env with the
// step's own `env:` on top) onto its result, secret-masked at capture time.
fn render_env_pane(
    f: &mut Frame<'_>,
    app: &App,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    area: Rect,
) {
    let step = selected.and_then(|i| job.steps.get(i));
    let title = match step {
        Some(s) => format!("Environment — {}", s.name),
        None => "Environment".to_string(),
    };
    let block = theme::block_focused(&title);
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let Some(step) = step else {
        render_pane_message(f, inner_area, "(select a step on the left)");
        return;
    };
    if step.env.is_empty() {
        let msg = if step.is_running() {
            "(environment appears when the step finishes)"
        } else {
            "(no environment captured for this step)"
        };
        render_pane_message(f, inner_area, msg);
        return;
    }

    let mut rows: Vec<(String, Style)> = vec![
        (
            format!("{} variables · secret values masked", step.env.len()),
            Style::default().fg(COLORS.text_muted),
        ),
        (String::new(), Style::default()),
    ];
    for (k, v) in &step.env {
        rows.push((format!("{}={}", k, v), Style::default().fg(COLORS.text_dim)));
    }
    render_scroll_pane(f, app, inner_area, &rows);
}

// ─── Files pane ───────────────────────────────────────────────────
//
// Per-step environment-file activity: what the step wrote to
// $GITHUB_OUTPUT, $GITHUB_ENV and $GITHUB_PATH (captured by the executor
// when it applies the files after each step).
fn render_files_pane(
    f: &mut Frame<'_>,
    app: &App,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    area: Rect,
) {
    let step = selected.and_then(|i| job.steps.get(i));
    let title = match step {
        Some(s) => format!("Environment files — {}", s.name),
        None => "Environment files".to_string(),
    };
    let block = theme::block_focused(&title);
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let Some(step) = step else {
        render_pane_message(f, inner_area, "(select a step on the left)");
        return;
    };
    if step.is_running() {
        render_pane_message(f, inner_area, "(file writes appear when the step finishes)");
        return;
    }
    if step.outputs.is_empty() && step.env_writes.is_empty() && step.path_writes.is_empty() {
        render_pane_message(
            f,
            inner_area,
            "this step wrote nothing to $GITHUB_OUTPUT, $GITHUB_ENV or $GITHUB_PATH",
        );
        return;
    }

    let header = |text: String| -> (String, Style) {
        (
            text,
            Style::default()
                .fg(theme::current_accent())
                .add_modifier(Modifier::BOLD),
        )
    };
    let body_style = Style::default().fg(COLORS.text_dim);
    let mut rows: Vec<(String, Style)> = Vec::new();

    let section = |rows: &mut Vec<(String, Style)>, name: &str, entries: Vec<String>| {
        if entries.is_empty() {
            return;
        }
        if !rows.is_empty() {
            rows.push((String::new(), Style::default()));
        }
        rows.push(header(format!("{} — {}", name, entries.len())));
        for e in entries {
            rows.push((format!("  {}", e), body_style));
        }
    };

    section(
        &mut rows,
        "$GITHUB_OUTPUT",
        step.outputs
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect(),
    );
    section(
        &mut rows,
        "$GITHUB_ENV",
        step.env_writes
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect(),
    );
    section(&mut rows, "$GITHUB_PATH", step.path_writes.clone());

    render_scroll_pane(f, app, inner_area, &rows);
}

// ─── Matrix pane ──────────────────────────────────────────────────
fn render_matrix_pane(
    f: &mut Frame<'_>,
    workflow: &crate::models::Workflow,
    job_name: &str,
    area: Rect,
) {
    let block = theme::block("Matrix");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let strategy = workflow
        .definition
        .as_ref()
        .and_then(|d| d.jobs.get(job_name))
        .and_then(|j| j.strategy.as_ref());

    let Some(strategy) = strategy else {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("`{}` is not a matrix job.", job_name),
                Style::default().fg(COLORS.text_dim),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            inner_area,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        "AXES",
        Style::default()
            .fg(COLORS.highlight)
            .add_modifier(Modifier::BOLD),
    )]));

    let Some(matrix) = strategy.matrix.as_ref() else {
        lines.push(Line::from(Span::styled(
            "(matrix strategy with no axes)",
            Style::default().fg(COLORS.text_muted),
        )));
        f.render_widget(Paragraph::new(lines), inner_area);
        return;
    };

    // A runtime-expression matrix has no static axes to render — its shape
    // comes from another job's outputs at execution time.
    let Some(matrix) = matrix.as_config() else {
        lines.push(Line::from(Span::styled(
            format!(
                "(dynamic matrix: {} — resolved from needs outputs at execution time)",
                strategy
                    .matrix
                    .as_ref()
                    .and_then(wrkflw_matrix::Matrix::as_expression)
                    .unwrap_or("expression")
            ),
            Style::default().fg(COLORS.text_muted),
        )));
        f.render_widget(Paragraph::new(lines), inner_area);
        return;
    };

    for (name, value) in &matrix.parameters {
        let values: Vec<String> = match value.as_sequence() {
            Some(seq) => seq
                .iter()
                .filter_map(|n| {
                    n.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| n.as_i64().map(|i| i.to_string()))
                        .or_else(|| n.as_f64().map(|f| f.to_string()))
                })
                .collect(),
            None => vec![format!("{:?}", value)],
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}: ", name),
                Style::default().fg(theme::current_accent()),
            ),
            Span::styled(values.join(", "), Style::default().fg(COLORS.text)),
        ]));
    }

    let mut chips: Vec<Span> = Vec::new();
    if let Some(max) = strategy.max_parallel.or(matrix.max_parallel) {
        chips.push(theme::badge_outline(
            format!("max-parallel: {}", max),
            BadgeKind::Dim,
        ));
        chips.push(Span::raw(" "));
    }
    let fail_fast = strategy.fail_fast.or(matrix.fail_fast).unwrap_or(true);
    if !fail_fast {
        chips.push(theme::badge_outline("fail-fast: false", BadgeKind::Warning));
    }
    if !chips.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(chips));
    }

    // Matrix combinations — real expansion via `wrkflw_matrix::expand_matrix`.
    //
    // We can show the combos (the *what* — design screen 5's grid) but
    // not per-combo runtime status (the *how it went* — we don't track
    // status per combo, only aggregated job status). So rows are
    // labelled `queued` by default; if the parent job finished we
    // inherit its status for every row. This is honest: a future
    // executor change to surface per-combo results will drop right
    // into this render.
    match wrkflw_matrix::expand_matrix(matrix) {
        Ok(combos) if !combos.is_empty() => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("COMBINATIONS ({})", combos.len()),
                Style::default()
                    .fg(COLORS.highlight)
                    .add_modifier(Modifier::BOLD),
            )]));
            // Key order: show axes in the order they were declared,
            // plus any extra keys an `include:` entry introduced,
            // appended after (and sorted, since `MatrixCombination.values`
            // is a HashMap whose iteration order is not stable — without
            // a sort, include-only columns could jitter between frames
            // or process runs).
            let mut key_order: Vec<String> = matrix.parameters.keys().cloned().collect();
            let mut extra: Vec<String> = Vec::new();
            for c in &combos {
                for k in c.values.keys() {
                    if !key_order.contains(k) && !extra.contains(k) {
                        extra.push(k.clone());
                    }
                }
            }
            extra.sort();
            key_order.extend(extra);
            for c in combos.iter().take(32) {
                let mut spans: Vec<Span> = vec![Span::raw("  ")];
                let status_glyph = inherited_combo_glyph(workflow, job_name);
                spans.push(status_glyph);
                spans.push(Span::raw(" "));
                for (i, k) in key_order.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::styled("  ", Style::default().fg(COLORS.text_muted)));
                    }
                    spans.push(Span::styled(
                        format!("{}=", k),
                        Style::default().fg(theme::current_accent()),
                    ));
                    let v = c
                        .values
                        .get(k)
                        .map(format_yaml_scalar)
                        .unwrap_or_else(|| "—".to_string());
                    spans.push(Span::styled(v, Style::default().fg(COLORS.text)));
                }
                if c.is_included {
                    spans.push(Span::raw("  "));
                    spans.push(theme::badge_outline("+include", BadgeKind::Warning));
                }
                lines.push(Line::from(spans));
            }
            if combos.len() > 32 {
                lines.push(Line::from(vec![Span::styled(
                    format!("  … +{} more", combos.len() - 32),
                    Style::default().fg(COLORS.text_muted),
                )]));
            }
        }
        Ok(_) => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "(matrix expanded to 0 combinations — check exclude: or empty axes)",
                Style::default().fg(COLORS.text_muted),
            )]));
        }
        Err(e) => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                theme::badge_outline("expansion error", BadgeKind::Error),
                Span::raw(" "),
                Span::styled(e.to_string(), Style::default().fg(COLORS.text_dim)),
            ]));
        }
    }

    if !matrix.exclude.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("EXCLUDE ({})", matrix.exclude.len()),
            Style::default()
                .fg(COLORS.highlight)
                .add_modifier(Modifier::BOLD),
        )]));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner_area);
}

fn format_yaml_scalar(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => collapse_newlines(s),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => "~".to_string(),
        other => {
            // Sequences and maps round-trip to multi-line YAML; each
            // combo renders into a single ratatui Span, so embedded
            // newlines would silently garble the layout. Collapse
            // them into a visible ` · ` separator.
            let raw = serde_yaml::to_string(other).unwrap_or_default();
            collapse_newlines(raw.trim())
        }
    }
}

/// Replace any `\n` / `\r` with a visible separator so multi-line
/// payloads don't break single-Line rendering.
fn collapse_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_sep = false;
    for ch in s.chars() {
        if ch == '\n' || ch == '\r' {
            if !last_was_sep {
                out.push_str(" · ");
                last_was_sep = true;
            }
        } else {
            out.push(ch);
            last_was_sep = false;
        }
    }
    out
}

/// Return a small colored glyph indicating what we know about the
/// parent matrix job's state — per-combo status isn't tracked, so
/// every row mirrors the parent.
fn inherited_combo_glyph<'a>(workflow: &'a crate::models::Workflow, job_name: &'a str) -> Span<'a> {
    let job = workflow
        .execution_details
        .as_ref()
        .and_then(|e| e.jobs.iter().find(|j| j.name == job_name));
    let (glyph, color) = match job.map(|j| &j.status) {
        Some(JobStatus::Success) => (theme::symbols::SUCCESS, COLORS.success),
        Some(JobStatus::Failure) => (theme::symbols::FAILURE, COLORS.error),
        Some(JobStatus::Skipped) => (theme::symbols::SKIPPED, COLORS.warning),
        None => (theme::symbols::NOT_STARTED, COLORS.text_muted),
    };
    Span::styled(glyph.to_string(), Style::default().fg(color))
}

// ─── Timeline pane (uses timing component) ────────────────────────
fn render_timeline_pane(f: &mut Frame<'_>, job: &crate::models::JobExecution, area: Rect) {
    let block = theme::block("Step timeline");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    // Real per-step durations from the executor's wall-clock stamps; a live
    // (still-running) step ticks against "now", steps without stamps fall
    // back to a status label + uniform bar.
    let now = std::time::SystemTime::now();
    let duration_ms = |s: &crate::models::StepExecution| -> Option<u64> {
        let start = s.started_at?;
        let end = s.finished_at.unwrap_or(now);
        Some(end.duration_since(start).ok()?.as_millis() as u64)
    };
    let max_ms = job.steps.iter().filter_map(&duration_ms).max().unwrap_or(0);

    let labels: Vec<String> = job
        .steps
        .iter()
        .map(|s| match duration_ms(s) {
            Some(ms) => super::gantt_tab::fmt_ms(ms),
            None => match s.status {
                StepStatus::Success => "ok".to_string(),
                StepStatus::Failure => "fail".to_string(),
                StepStatus::Skipped => "skip".to_string(),
            },
        })
        .collect();

    let rows: Vec<TimingRow> = job
        .steps
        .iter()
        .zip(labels.iter())
        .map(|(s, label)| TimingRow {
            name: s.name.as_str(),
            // `None` = live styling: a running step renders as an
            // info-colored growing bar rather than a final status color.
            status: if s.is_running() {
                None
            } else {
                match s.status {
                    StepStatus::Success => Some(JobStatus::Success),
                    StepStatus::Failure => Some(JobStatus::Failure),
                    StepStatus::Skipped => Some(JobStatus::Skipped),
                }
            },
            label: label.as_str(),
            weight: duration_ms(s)
                .filter(|_| max_ms > 0)
                .map(|ms| ms as f32 / max_ms as f32),
        })
        .collect();

    timing::render(f, inner_area, &rows);
}

#[cfg(test)]
mod tests {
    use super::parse_step_output;

    #[test]
    fn parses_command_stdout_and_stderr_sections() {
        let raw = "Command: tools/resolve-images.sh\n\nStandard Output:\nResolution:\n  ying: rebuild\nStandard Error:\nwarning: no creds\n";
        let parsed = parse_step_output(raw);
        assert_eq!(parsed.command.as_deref(), Some("tools/resolve-images.sh"));
        assert_eq!(parsed.stdout, "Resolution:\n  ying: rebuild");
        assert_eq!(parsed.stderr, "warning: no creds");
    }

    #[test]
    fn multiline_commands_stay_in_the_command_section() {
        let raw = "Command: set -e\nmake build\n\nStandard Output:\ndone\n";
        let parsed = parse_step_output(raw);
        assert_eq!(parsed.command.as_deref(), Some("set -e\nmake build"));
        assert_eq!(parsed.stdout, "done");
        assert!(parsed.stderr.is_empty());
    }

    #[test]
    fn unstructured_output_lands_whole_in_stdout() {
        let raw = "just some words\nsecond line";
        let parsed = parse_step_output(raw);
        assert!(parsed.command.is_none());
        assert_eq!(parsed.stdout, raw);
        assert!(parsed.stderr.is_empty());
    }

    #[test]
    fn stderr_only_output_parses() {
        let raw = "Command: false\n\nStandard Error:\nboom\n";
        let parsed = parse_step_output(raw);
        assert_eq!(parsed.command.as_deref(), Some("false"));
        assert!(parsed.stdout.is_empty());
        assert_eq!(parsed.stderr, "boom");
    }
}
