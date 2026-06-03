use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use crate::runtime::state::{RunState, RunStatus};
use super::app::{App, TuiMode, PaneFocus};

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(40),
            Constraint::Percentage(35),
        ])
        .split(f.area());

    render_files(f, app, chunks[0]);
    match app.mode() {
        TuiMode::FileBrowser => render_flowchart(f, app, chunks[1]),
        TuiMode::RunList     => render_run_list(f, app, chunks[1]),
    }
    match app.mode() {
        TuiMode::FileBrowser => render_stage_detail(f, app, chunks[2]),
        TuiMode::RunList     => render_event_log(f, app, chunks[2]),
    }

    if app.modal.is_some() {
        render_modal(f, app, f.area());
    }

    if let Some(run_id) = &app.delete_confirm {
        render_confirm_bar(f, run_id, f.area());
    }
}

fn pane_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn render_files(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Files;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Files ")
        .border_style(pane_style(focused));

    let items: Vec<ListItem> = app.browser.entries.iter().enumerate().map(|(i, e)| {
        let name = e.display_name();
        let style = if Some(i) == app.browser.selected_file.as_ref().and_then(|sel| {
            app.browser.entries.iter().position(|entry| {
                if let super::app::Entry::LineFile(p) = entry { p == sel } else { false }
            })
        }) {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else if i == app.browser.cursor && focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        ListItem::new(name).style(style)
    }).collect();

    let mut state = ListState::default();
    if focused { state.select(Some(app.browser.cursor)); }

    f.render_stateful_widget(List::new(items).block(block), area, &mut state);
}

fn render_flowchart(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Middle;
    let title = app.browser.selected_file
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| format!(" Flow: {} ", n.to_string_lossy()))
        .unwrap_or_else(|| " Flow ".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(pane_style(focused));

    let lines: Vec<Line> = app.flowchart_lines.iter().enumerate().map(|(i, l)| {
        let style = if i == app.flowchart_cursor && focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        Line::styled(l.clone(), style)
    }).collect();

    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn status_badge(status: &RunStatus) -> (&'static str, Color) {
    match status {
        RunStatus::Done => ("● done", Color::Green),
        RunStatus::Failed(_) => ("○ failed", Color::Red),
        RunStatus::Running => ("◉ running", Color::Yellow),
        RunStatus::AwaitingResume { .. } => ("◉ awaiting", Color::Yellow),
        RunStatus::ParallelAwait { .. } => ("◉ parallel", Color::Yellow),
    }
}

fn render_run_list(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Middle;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Runs ")
        .border_style(pane_style(focused));

    let items: Vec<ListItem> = app.run_list.iter().enumerate().map(|(i, run)| {
        let (badge, color) = status_badge(&run.status);
        let line = format!("{:<10} {:<20} {}", badge, run.pipeline, run.started.format("%m-%d %H:%M"));
        let style = if i == app.selected_run && focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(color)
        };
        ListItem::new(line).style(style)
    }).collect();

    let mut state = ListState::default();
    if focused { state.select(Some(app.selected_run)); }

    f.render_stateful_widget(List::new(items).block(block), area, &mut state);
}

fn render_stage_detail(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Detail;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Stage Detail ")
        .border_style(pane_style(focused));

    let content = if app.graph_stages.is_empty() {
        "Select a .line file to inspect".to_string()
    } else {
        app.graph_stages.get(app.flowchart_cursor)
            .map(|stage_name| format_stage_detail(stage_name, app))
            .unwrap_or_default()
    };

    f.render_widget(Paragraph::new(content).block(block).wrap(Wrap { trim: false }), area);
}

fn format_stage_detail(stage_name: &str, app: &App) -> String {
    let Some(file) = &app.browser.selected_file else { return String::new() };
    let Ok(items) = crate::cli::load_items(file) else { return String::new() };

    let mut out = format!("stage: {}\n", stage_name);
    for item in &items {
        if let crate::ast::TlItem::Stage(s) = item {
            if s.name != stage_name { continue; }
            if let Some(r) = &s.runner { out.push_str(&format!("runner: {}\n", r)); }
            if !s.inputs.is_empty() {
                out.push_str("in:\n");
                for a in &s.inputs {
                    let opt = if a.optional { "?" } else { "" };
                    let kind = match a.kind { crate::ast::ArtifactKind::Path => "path", crate::ast::ArtifactKind::Value => "value" };
                    out.push_str(&format!("  {}{} as {}\n", a.name, opt, kind));
                }
            }
            if !s.outputs.is_empty() {
                out.push_str("out:\n");
                for a in &s.outputs {
                    let kind = match a.kind { crate::ast::ArtifactKind::Path => "path", crate::ast::ArtifactKind::Value => "value" };
                    let constraint = a.value_constraint.as_ref()
                        .map(|c| format!(" in [{}]", c.iter().map(|v| format!("\"{}\"", v)).collect::<Vec<_>>().join(", ")))
                        .unwrap_or_default();
                    out.push_str(&format!("  {} as {}{}\n", a.name, kind, constraint));
                }
            }
            if let Some(p) = &s.prompt {
                let src = match p { crate::ast::PromptSource::Inline(s) => s.chars().take(40).collect::<String>(), crate::ast::PromptSource::File(f) => format!("file({})", f) };
                out.push_str(&format!("prompt: {}\n", src));
            }
        }
    }
    out
}

fn render_event_log(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Detail;
    let run = app.run_list.get(app.selected_run);
    let title = run.map(|r| format!(" Events: {} ", r.run_id)).unwrap_or_else(|| " Events ".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(pane_style(focused));

    let content = if let Some(run) = run {
        app.event_logs.get(&run.run_id)
            .map(|lines| lines.iter().map(|l| format_event_line(l)).collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|| "(no events yet)".to_string())
    } else {
        "(no run selected)".to_string()
    };

    f.render_widget(Paragraph::new(content).block(block).wrap(Wrap { trim: false }), area);
}

fn format_event_line(raw: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        let event = v["event"].as_str().unwrap_or("?");
        let stage = v["stage"].as_str().map(|s| format!(" {}", s)).unwrap_or_default();
        return format!("✓ {}{}", event, stage);
    }
    raw.to_string()
}

fn render_confirm_bar(f: &mut Frame, run_id: &str, area: Rect) {
    let msg = format!(" Delete {}? [y/n] ", run_id);
    let bar_area = Rect { x: area.x, y: area.height.saturating_sub(1), width: area.width, height: 1 };
    f.render_widget(
        Paragraph::new(msg).style(Style::default().bg(Color::Red).fg(Color::White)),
        bar_area,
    );
}

pub fn render_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(modal) = &app.modal else { return };

    let width = 50u16.min(area.width.saturating_sub(4));
    let height = (4 + modal.input_keys.len() as u16 + 3).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect { x, y, width, height };

    // Clear background
    f.render_widget(ratatui::widgets::Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Launch: {} ", modal.file.file_name().unwrap_or_default().to_string_lossy()))
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let mut lines: Vec<Line> = Vec::new();

    let driver_style = if modal.focused_field == 0 { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
    lines.push(Line::from(vec![
        Span::raw("driver: "),
        Span::styled(format!("[ {} ]", modal.driver()), driver_style),
    ]));

    for (i, key) in modal.input_keys.iter().enumerate() {
        let val = &modal.input_values[i];
        let style = if modal.focused_field == i + 1 { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::raw(format!("{}: ", key)),
            Span::styled(format!("{}_", val), style),
        ]));
    }

    if modal.pipeline_names.len() > 1 {
        let field_idx = modal.input_keys.len() + 1;
        let style = if modal.focused_field == field_idx { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::raw("pipeline: "),
            Span::styled(modal.pipeline_names[modal.pipeline_idx].clone(), style),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled("tab:next  enter:launch  esc:cancel", Style::default().fg(Color::DarkGray)));

    f.render_widget(Paragraph::new(lines), inner);
}
