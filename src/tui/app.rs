use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Parent,             // ".." navigate up
    Dir(PathBuf),       // subdirectory
    LineFile(PathBuf),  // *.line file
}

impl Entry {
    pub fn display_name(&self) -> String {
        match self {
            Entry::Parent => "..".to_string(),
            Entry::Dir(p) => format!("{}/", p.file_name().unwrap_or_default().to_string_lossy()),
            Entry::LineFile(p) => p.file_name().unwrap_or_default().to_string_lossy().to_string(),
        }
    }
}

#[derive(Debug)]
pub struct FileBrowser {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub selected_file: Option<PathBuf>,
}

impl FileBrowser {
    pub fn new(cwd: PathBuf) -> Self {
        let mut fb = FileBrowser { cwd: cwd.clone(), entries: vec![], cursor: 0, selected_file: None };
        fb.reload();
        fb
    }

    pub fn reload(&mut self) {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.cwd) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().map_or(false, |ext| ext == "line") {
                    files.push(path);
                }
            }
        }
        dirs.sort();
        files.sort();
        self.entries = vec![Entry::Parent];
        self.entries.extend(dirs.into_iter().map(Entry::Dir));
        self.entries.extend(files.into_iter().map(Entry::LineFile));
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        // Clear selected file if it's no longer in this dir
        if let Some(sel) = &self.selected_file {
            if sel.parent() != Some(&self.cwd) {
                self.selected_file = None;
            }
        }
    }

    pub fn navigate_up(&mut self) { self.cursor = if self.cursor == 0 { self.entries.len() - 1 } else { self.cursor - 1 }; }
    pub fn navigate_down(&mut self) { self.cursor = if self.cursor + 1 >= self.entries.len() { 0 } else { self.cursor + 1 }; }

    /// Enter the currently selected entry. Returns true if a .line file was selected.
    pub fn enter(&mut self) -> bool {
        match self.entries.get(self.cursor).cloned() {
            Some(Entry::Parent) => {
                if let Some(parent) = self.cwd.parent().map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf())) {
                    self.cwd = parent;
                    self.cursor = 0;
                    self.selected_file = None;
                    self.reload();
                }
                false
            }
            Some(Entry::Dir(path)) => {
                self.cwd = path;
                self.cursor = 0;
                self.selected_file = None;
                self.reload();
                false
            }
            Some(Entry::LineFile(path)) => {
                self.selected_file = Some(path);
                true
            }
            None => false,
        }
    }
}

// ── TUI mode ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TuiMode {
    FileBrowser,  // a .line file is selected → show flowchart
    RunList,      // no file selected → show run list
}

// ── Modal ────────────────────────────────────────────────────────────────

pub const DRIVERS: &[&str] = &["stdio", "anthropic", "mock", "ollama", "openai", "bedrock", "vertex"];

#[derive(Debug)]
pub struct ModalState {
    pub file: PathBuf,
    pub driver_idx: usize,
    pub input_keys: Vec<String>,   // names of declared pipeline inputs
    pub input_values: Vec<String>, // parallel to input_keys
    pub pipeline_names: Vec<String>,
    pub pipeline_idx: usize,
    pub focused_field: usize,      // 0=driver, 1..=input_keys.len()=inputs, last=pipeline selector
}

impl ModalState {
    pub fn total_fields(&self) -> usize {
        1 + self.input_keys.len() + if self.pipeline_names.len() > 1 { 1 } else { 0 }
    }

    pub fn next_field(&mut self) { self.focused_field = (self.focused_field + 1) % self.total_fields(); }
    pub fn prev_field(&mut self) { self.focused_field = if self.focused_field == 0 { self.total_fields() - 1 } else { self.focused_field - 1 }; }

    pub fn cycle_driver_forward(&mut self) { self.driver_idx = (self.driver_idx + 1) % DRIVERS.len(); }
    pub fn cycle_driver_backward(&mut self) { self.driver_idx = if self.driver_idx == 0 { DRIVERS.len() - 1 } else { self.driver_idx - 1 }; }

    pub fn driver(&self) -> &str { DRIVERS[self.driver_idx] }
    pub fn selected_pipeline(&self) -> Option<&str> {
        if self.pipeline_names.len() > 1 { Some(&self.pipeline_names[self.pipeline_idx]) } else { None }
    }

    pub fn build_launch_args(&self) -> Vec<String> {
        let mut args = vec![
            "run".to_string(),
            self.file.to_string_lossy().to_string(),
            "--driver".to_string(),
            self.driver().to_string(),
        ];
        for (k, v) in self.input_keys.iter().zip(self.input_values.iter()) {
            if !v.is_empty() {
                args.push("--input".to_string());
                args.push(format!("{}={}", k, v));
            }
        }
        if let Some(name) = self.selected_pipeline() {
            args.push("--pipeline".to_string());
            args.push(name.to_string());
        }
        args
    }
}

// ── App ──────────────────────────────────────────────────────────────────

pub struct App {
    pub browser: FileBrowser,
    pub run_list: Vec<crate::runtime::state::RunState>,
    pub selected_run: usize,
    pub event_logs: std::collections::HashMap<String, Vec<String>>,
    pub modal: Option<ModalState>,
    pub flowchart_lines: Vec<String>,
    pub flowchart_cursor: usize,
    pub graph_stages: Vec<String>,  // ordered stage names for cursor navigation
    pub event_rx: tokio::sync::mpsc::Receiver<(String, String)>,  // (run_id, ndjson_line)
    pub delete_confirm: Option<String>,  // run_id pending delete confirmation
    pub pane_focus: PaneFocus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneFocus {
    Files,
    Middle,
    Detail,
}

impl App {
    pub fn new(event_rx: tokio::sync::mpsc::Receiver<(String, String)>) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;
        let run_list = crate::runtime::state::list_runs().unwrap_or_default();
        Ok(App {
            browser: FileBrowser::new(cwd),
            run_list,
            selected_run: 0,
            event_logs: std::collections::HashMap::new(),
            modal: None,
            flowchart_lines: vec![],
            flowchart_cursor: 0,
            graph_stages: vec![],
            event_rx,
            delete_confirm: None,
            pane_focus: PaneFocus::Files,
        })
    }

    pub fn mode(&self) -> TuiMode {
        if self.browser.selected_file.is_some() { TuiMode::FileBrowser } else { TuiMode::RunList }
    }

    pub async fn tick(&mut self) {
        // Drain event channel
        while let Ok((run_id, line)) = self.event_rx.try_recv() {
            self.event_logs.entry(run_id).or_default().push(line);
        }
        // Refresh run list
        if let Ok(runs) = crate::runtime::state::list_runs() {
            self.run_list = runs;
        }
    }

    pub fn update_flowchart(&mut self) {
        let Some(file) = &self.browser.selected_file else { return; };
        let file = file.clone();
        match crate::cli::load_items(&file) {
            Ok(items) => {
                let pipeline_name = self.modal.as_ref().and_then(|m| m.selected_pipeline());
                match crate::tui::visualizer::build_graph(&items, pipeline_name) {
                    Ok(graph) => {
                        self.graph_stages = graph.stages.clone();
                        self.flowchart_lines = crate::tui::visualizer::render_graph(&graph);
                        self.flowchart_cursor = 0;
                    }
                    Err(_) => {
                        self.flowchart_lines = vec!["(could not build graph)".to_string()];
                        self.graph_stages = vec![];
                    }
                }
            }
            Err(_) => {
                self.flowchart_lines = vec!["(could not parse file)".to_string()];
                self.graph_stages = vec![];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_browser_lists_line_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.line"), "").unwrap();
        std::fs::write(dir.path().join("b.line"), "").unwrap();
        std::fs::write(dir.path().join("readme.md"), "").unwrap();
        let fb = FileBrowser::new(dir.path().to_path_buf());
        let names: Vec<_> = fb.entries.iter().map(|e| e.display_name()).collect();
        assert!(names.contains(&"..".to_string()));
        assert!(names.contains(&"a.line".to_string()));
        assert!(names.contains(&"b.line".to_string()));
        assert!(!names.contains(&"readme.md".to_string()));
    }

    #[test]
    fn test_file_browser_navigate_down_wraps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.line"), "").unwrap();
        let mut fb = FileBrowser::new(dir.path().to_path_buf());
        assert_eq!(fb.entries.len(), 2); // ".." and "a.line"
        fb.navigate_down(); // cursor=1
        fb.navigate_down(); // wraps to 0
        assert_eq!(fb.cursor, 0);
    }

    #[test]
    fn test_file_browser_enter_selects_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.line"), "").unwrap();
        let mut fb = FileBrowser::new(dir.path().to_path_buf());
        fb.navigate_down(); // cursor on a.line (index 1, past "..")
        let selected = fb.enter();
        assert!(selected);
        assert!(fb.selected_file.is_some());
    }

    #[test]
    fn test_file_browser_navigate_up_changes_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("sub");
        std::fs::create_dir(&subdir).unwrap();
        let mut fb = FileBrowser::new(subdir.clone());
        // cursor=0 is "..", enter navigates up
        fb.enter();
        assert_eq!(fb.cwd, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_modal_build_launch_args_basic() {
        let modal = ModalState {
            file: PathBuf::from("/tmp/test.line"),
            driver_idx: 1,  // anthropic
            input_keys: vec!["code".to_string()],
            input_values: vec!["file:///tmp/code.rs".to_string()],
            pipeline_names: vec!["p".to_string()],
            pipeline_idx: 0,
            focused_field: 0,
        };
        let args = modal.build_launch_args();
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--driver".to_string()));
        assert!(args.contains(&"anthropic".to_string()));
        assert!(args.contains(&"--input".to_string()));
        assert!(args.contains(&"code=file:///tmp/code.rs".to_string()));
    }
}
