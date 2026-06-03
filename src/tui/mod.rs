pub mod app;
pub mod runner;
pub mod ui;
pub mod visualizer;

use std::io::stdout;
use std::time::Duration;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use app::{App, ModalState, PaneFocus};
use crate::ast::TlItem;

pub async fn cmd_tui() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = mpsc::channel::<(String, String)>(256);
    let mut app = App::new(rx)?;

    let mut reader = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut should_quit = false;

    while !should_quit {
        terminal.draw(|f| ui::render(f, &app))?;

        tokio::select! {
            _ = tick.tick() => {
                app.tick().await;
            }
            maybe_ev = reader.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_ev {
                    should_quit = handle_key(&mut app, key, &tx).await;
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn handle_key(app: &mut App, key: KeyEvent, tx: &mpsc::Sender<(String, String)>) -> bool {
    // Quit always works
    if key.code == KeyCode::Char('q') && app.modal.is_none() {
        return true;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }

    // Delete confirmation
    if let Some(run_id) = app.delete_confirm.clone() {
        match key.code {
            KeyCode::Char('y') => {
                let run_dir = crate::runtime::state::runs_dir().join(&run_id);
                let _ = std::fs::remove_dir_all(run_dir);
                app.delete_confirm = None;
                app.tick().await;
            }
            _ => { app.delete_confirm = None; }
        }
        return false;
    }

    // Modal handling
    if app.modal.is_some() {
        match key.code {
            KeyCode::Esc => { app.modal = None; }
            KeyCode::Tab => { app.modal.as_mut().unwrap().next_field(); }
            KeyCode::BackTab => { app.modal.as_mut().unwrap().prev_field(); }
            KeyCode::Up => {
                let m = app.modal.as_mut().unwrap();
                if m.focused_field == 0 { m.cycle_driver_backward(); }
            }
            KeyCode::Down => {
                let m = app.modal.as_mut().unwrap();
                if m.focused_field == 0 { m.cycle_driver_forward(); }
            }
            KeyCode::Char(c) => {
                let m = app.modal.as_mut().unwrap();
                let field = m.focused_field;
                if field > 0 && field <= m.input_keys.len() {
                    m.input_values[field - 1].push(c);
                }
            }
            KeyCode::Backspace => {
                let m = app.modal.as_mut().unwrap();
                let field = m.focused_field;
                if field > 0 && field <= m.input_keys.len() {
                    m.input_values[field - 1].pop();
                }
            }
            KeyCode::Enter => {
                let m = app.modal.as_ref().unwrap();
                let file = m.file.clone();
                let driver = m.driver().to_string();
                let inputs: Vec<String> = m.input_keys.iter().zip(m.input_values.iter())
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect();
                let pipeline = m.selected_pipeline().map(|s| s.to_string());
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    let _ = runner::spawn_run(&file, &driver, &inputs, pipeline.as_deref(), tx_clone).await;
                });
                app.modal = None;
            }
            _ => {}
        }
        return false;
    }

    // Normal navigation
    match key.code {
        KeyCode::Char('1') => { app.pane_focus = PaneFocus::Files; }
        KeyCode::Char('2') => { app.pane_focus = PaneFocus::Middle; }
        KeyCode::Char('3') => { app.pane_focus = PaneFocus::Detail; }
        KeyCode::Tab => {
            app.pane_focus = match app.pane_focus {
                PaneFocus::Files  => PaneFocus::Middle,
                PaneFocus::Middle => PaneFocus::Detail,
                PaneFocus::Detail => PaneFocus::Files,
            };
        }
        KeyCode::Up => match app.pane_focus {
            PaneFocus::Files => { app.browser.navigate_up(); }
            PaneFocus::Middle => {
                if app.mode() == app::TuiMode::RunList {
                    if app.selected_run > 0 { app.selected_run -= 1; }
                } else if app.flowchart_cursor > 0 {
                    app.flowchart_cursor -= 1;
                }
            }
            PaneFocus::Detail => {}
        },
        KeyCode::Down => match app.pane_focus {
            PaneFocus::Files => { app.browser.navigate_down(); }
            PaneFocus::Middle => {
                if app.mode() == app::TuiMode::RunList {
                    if app.selected_run + 1 < app.run_list.len() { app.selected_run += 1; }
                } else {
                    let max = app.flowchart_lines.len().saturating_sub(1);
                    if app.flowchart_cursor < max { app.flowchart_cursor += 1; }
                }
            }
            PaneFocus::Detail => {}
        },
        KeyCode::Enter => {
            if app.pane_focus == PaneFocus::Files {
                let was_selected = app.browser.enter();
                if was_selected {
                    app.update_flowchart();
                    app.pane_focus = PaneFocus::Middle;
                }
            }
        }
        KeyCode::Char('r') => {
            if app.pane_focus == PaneFocus::Files {
                if let Some(file) = app.browser.selected_file.clone() {
                    if let Ok(items) = crate::cli::load_items(&file) {
                        let pipeline_names: Vec<String> = items.iter()
                            .filter_map(|i| if let TlItem::Pipeline(p) = i { Some(p.name.clone()) } else { None })
                            .collect();
                        let input_keys: Vec<String> = items.iter()
                            .filter_map(|i| if let TlItem::Pipeline(p) = i { Some(p) } else { None })
                            .next()
                            .map(|p| p.inputs.iter().map(|inp| inp.name.clone()).collect())
                            .unwrap_or_default();
                        let n = input_keys.len();
                        app.modal = Some(ModalState {
                            file,
                            driver_idx: 0,
                            input_keys,
                            input_values: vec![String::new(); n],
                            pipeline_names,
                            pipeline_idx: 0,
                            focused_field: 0,
                        });
                    }
                }
            }
        }
        KeyCode::Char('d') => {
            if app.pane_focus == PaneFocus::Middle {
                if let Some(run) = app.run_list.get(app.selected_run) {
                    app.delete_confirm = Some(run.run_id.clone());
                }
            }
        }
        _ => {}
    }

    false
}
