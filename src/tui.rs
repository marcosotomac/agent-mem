use crate::error::Result;
use crate::init::GitDoctorReport;
use crate::installer::{ClientStatus, check_all_clients, install_all_or_target, install_client};
use crate::store::{RuleRecord, SessionEntry, Store};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap};
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// shadcn / geist color palette constants
const ACCENT: Color = Color::White;
const EMERALD: Color = Color::Indexed(150); // Soft emerald (#a7d998)
const AMBER: Color = Color::Indexed(216); // Soft amber (#fab283)
const MUTED: Color = Color::Indexed(245); // Zinc-400
const SUBTLE: Color = Color::Indexed(238); // Zinc-700
const BG_SELECT: Color = Color::Indexed(236); // Subtle selection gray

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Rules,
    Sessions,
    Projects,
    Doctor,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filter,
    ConfirmDelete,
    NewRule {
        field: usize, // 0: Key, 1: Value, 2: Anchor
        key: String,
        val: String,
        anchor: String,
    },
    EditRule {
        field: usize, // 0: Value, 1: Anchor
        key: String,
        val: String,
        anchor: String,
    },
    NewSession {
        summary: String,
    },
}

pub struct App {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub active_tab: ActiveTab,
    pub input_mode: InputMode,
    pub filter_query: String,
    pub rules: Vec<RuleRecord>,
    pub filtered_indices: Vec<usize>,
    pub selected_rule_idx: usize,
    pub relations: Vec<crate::store::RelationRecord>,
    pub sessions: Vec<SessionEntry>,
    pub selected_session_idx: usize,
    pub projects: Vec<crate::registry::ProjectRecord>,
    pub selected_project_idx: usize,
    pub git_doctor: GitDoctorReport,
    pub client_statuses: Vec<ClientStatus>,
    pub selected_client_idx: usize,
    pub toast: Option<(String, Instant)>,
    pub should_quit: bool,
}

impl App {
    pub fn new(root: PathBuf) -> Result<Self> {
        let db_path = root.join(".agent-mem").join("mem.db");
        let git_doctor = crate::init::inspect_git_health(&root);
        let client_statuses = check_all_clients();

        let mut app = Self {
            root,
            db_path,
            active_tab: ActiveTab::Rules,
            input_mode: InputMode::Normal,
            filter_query: String::new(),
            rules: Vec::new(),
            filtered_indices: Vec::new(),
            selected_rule_idx: 0,
            relations: Vec::new(),
            sessions: Vec::new(),
            selected_session_idx: 0,
            projects: Vec::new(),
            selected_project_idx: 0,
            git_doctor,
            client_statuses,
            selected_client_idx: 0,
            toast: None,
            should_quit: false,
        };
        app.reload_data()?;
        Ok(app)
    }

    pub fn reload_data(&mut self) -> Result<()> {
        if self.db_path.exists() {
            let store = Store::open(&self.db_path, false)?;
            self.rules = store.dump_all().unwrap_or_default();
            self.relations = store.get_all_relations().unwrap_or_default();
            self.sessions = store.session_list(100).unwrap_or_default();
        } else {
            self.rules.clear();
            self.relations.clear();
            self.sessions.clear();
        }
        if let Ok(mut reg) = crate::registry::ProjectRegistry::load() {
            if self.db_path.exists() {
                let _ = reg.register(&self.root);
            }
            self.projects = reg.projects;
        }
        self.git_doctor = crate::init::inspect_git_health(&self.root);
        self.client_statuses = check_all_clients();
        self.apply_filter();
        Ok(())
    }

    pub fn apply_filter(&mut self) {
        if self.filter_query.is_empty() {
            self.filtered_indices = (0..self.rules.len()).collect();
        } else {
            let q = self.filter_query.to_lowercase();
            self.filtered_indices = self
                .rules
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.key.to_lowercase().contains(&q)
                        || r.val.to_lowercase().contains(&q)
                        || r.anchor
                            .as_deref()
                            .map(|a| a.to_lowercase().contains(&q))
                            .unwrap_or(false)
                        || r.archive_reason
                            .as_deref()
                            .map(|ar| ar.to_lowercase().contains(&q))
                            .unwrap_or(false)
                })
                .map(|(i, _)| i)
                .collect();
        }

        if self.filtered_indices.is_empty() {
            self.selected_rule_idx = 0;
        } else if self.selected_rule_idx >= self.filtered_indices.len() {
            self.selected_rule_idx = self.filtered_indices.len().saturating_sub(1);
        }
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn current_selected_rule(&self) -> Option<&RuleRecord> {
        let actual_idx = *self.filtered_indices.get(self.selected_rule_idx)?;
        self.rules.get(actual_idx)
    }

    pub fn toggle_archive_selected(&mut self) -> Result<()> {
        let rule_key = match self.current_selected_rule() {
            Some(r) => r.key.clone(),
            None => return Ok(()),
        };
        let is_currently_archived = self
            .current_selected_rule()
            .map(|r| r.is_archived())
            .unwrap_or(false);

        if self.db_path.exists() {
            let mut store = Store::open(&self.db_path, true)?;
            if is_currently_archived {
                store.unarchive(&rule_key)?;
                self.set_toast(format!("Reactivated rule: {}", rule_key));
            } else {
                store.archive(&rule_key, Some("Archived via TUI"))?;
                self.set_toast(format!("Archived rule: {}", rule_key));
            }
            let rules_file = self.root.join(".agent-rules");
            if rules_file.exists() {
                let _ = store.export_to_file(&rules_file);
            }
        }
        self.reload_data()
    }

    pub fn delete_selected_rule(&mut self) -> Result<()> {
        let rule_key = match self.current_selected_rule() {
            Some(r) => r.key.clone(),
            None => return Ok(()),
        };

        if self.db_path.exists() {
            let mut store = Store::open(&self.db_path, true)?;
            store.del(&rule_key)?;
            let rules_file = self.root.join(".agent-rules");
            if rules_file.exists() {
                let _ = store.export_to_file(&rules_file);
            }
            self.set_toast(format!("Deleted rule: {}", rule_key));
        }
        self.reload_data()
    }

    pub fn create_rule(&mut self, key: &str, val: &str, anchor: Option<&str>) -> Result<bool> {
        if key.trim().is_empty() || val.trim().is_empty() {
            self.set_toast("Rule Key and Value cannot be empty!");
            return Ok(false);
        }

        let mut store = Store::open(&self.db_path, true)?;
        store.set_with_anchor(
            key.trim(),
            val.trim(),
            anchor.filter(|a| !a.trim().is_empty()),
        )?;
        let rules_file = self.root.join(".agent-rules");
        if rules_file.exists() {
            let _ = store.export_to_file(&rules_file);
        }
        self.set_toast(format!("Saved rule: {}", key.trim()));
        self.reload_data()?;
        Ok(true)
    }

    pub fn add_session(&mut self, summary: &str) -> Result<bool> {
        if summary.trim().is_empty() {
            self.set_toast("Session summary cannot be empty!");
            return Ok(false);
        }

        let mut store = Store::open(&self.db_path, true)?;
        let id = store.session_add(summary.trim())?;
        self.set_toast(format!("Recorded checkpoint #{}", id));
        self.reload_data()?;
        Ok(true)
    }

    pub fn run_git_sync(&mut self) -> Result<()> {
        let rules_file = self.root.join(".agent-rules");
        let mut store = Store::open(&self.db_path, true)?;
        let report = if rules_file.exists() {
            store.sync_with_file(&rules_file)?
        } else {
            store.sync_export(&rules_file)?
        };
        self.set_toast(format!(
            "Git Sync: {} imported, {} total rules",
            report.imported, report.total
        ));
        self.reload_data()
    }

    pub fn install_selected_client_mcp(&mut self) -> Result<()> {
        let detected_clients: Vec<&ClientStatus> = self
            .client_statuses
            .iter()
            .filter(|c| c.installed || c.configured)
            .collect();

        if let Some(target) = detected_clients.get(self.selected_client_idx) {
            let res = install_client(target.client)?;
            self.set_toast(format!("Configured MCP for {}", res.client.display_name()));
            self.client_statuses = check_all_clients();
        }
        Ok(())
    }

    pub fn install_all_clients_mcp(&mut self) -> Result<()> {
        let results = install_all_or_target(None)?;
        self.set_toast(format!("Configured MCP for {} AI clients", results.len()));
        self.client_statuses = check_all_clients();
        Ok(())
    }

    pub fn switch_project(&mut self, idx: usize) -> Result<()> {
        if let Some(p) = self.projects.get(idx).cloned() {
            let new_root = PathBuf::from(&p.canonical_path);
            if new_root.exists() {
                self.root = new_root;
                self.db_path = self.root.join(".agent-mem").join("mem.db");
                self.git_doctor = crate::init::inspect_git_health(&self.root);
                self.reload_data()?;
                self.active_tab = ActiveTab::Rules;
                self.set_toast(format!("Switched to project: {}", p.name));
            } else {
                self.set_toast(format!("Directory no longer exists: {}", p.name));
            }
        }
        Ok(())
    }

    pub fn deregister_selected_project(&mut self) -> Result<()> {
        if let Some(p) = self.projects.get(self.selected_project_idx) {
            let id = p.id.clone();
            let name = p.name.clone();
            if let Ok(mut reg) = crate::registry::ProjectRegistry::load() {
                let _ = reg.deregister(&id);
                self.projects = reg.projects;
                if self.selected_project_idx >= self.projects.len() && !self.projects.is_empty() {
                    self.selected_project_idx = self.projects.len() - 1;
                }
                self.set_toast(format!("Deregistered project: {}", name));
            }
        }
        Ok(())
    }

    pub fn next_item(&mut self) {
        match self.active_tab {
            ActiveTab::Rules => {
                if !self.filtered_indices.is_empty() {
                    self.selected_rule_idx =
                        (self.selected_rule_idx + 1).min(self.filtered_indices.len() - 1);
                }
            }
            ActiveTab::Sessions => {
                if !self.sessions.is_empty() {
                    self.selected_session_idx =
                        (self.selected_session_idx + 1).min(self.sessions.len() - 1);
                }
            }
            ActiveTab::Projects => {
                if !self.projects.is_empty() {
                    self.selected_project_idx =
                        (self.selected_project_idx + 1).min(self.projects.len() - 1);
                }
            }
            ActiveTab::Doctor => {
                let detected_count = self
                    .client_statuses
                    .iter()
                    .filter(|c| c.installed || c.configured)
                    .count();
                if detected_count > 0 {
                    self.selected_client_idx =
                        (self.selected_client_idx + 1).min(detected_count - 1);
                }
            }
            ActiveTab::Help => {}
        }
    }

    pub fn prev_item(&mut self) {
        match self.active_tab {
            ActiveTab::Rules => {
                self.selected_rule_idx = self.selected_rule_idx.saturating_sub(1);
            }
            ActiveTab::Sessions => {
                self.selected_session_idx = self.selected_session_idx.saturating_sub(1);
            }
            ActiveTab::Projects => {
                self.selected_project_idx = self.selected_project_idx.saturating_sub(1);
            }
            ActiveTab::Doctor => {
                self.selected_client_idx = self.selected_client_idx.saturating_sub(1);
            }
            ActiveTab::Help => {}
        }
    }

    pub fn switch_tab(&mut self) {
        self.active_tab = match self.active_tab {
            ActiveTab::Rules => ActiveTab::Sessions,
            ActiveTab::Sessions => ActiveTab::Projects,
            ActiveTab::Projects => ActiveTab::Doctor,
            ActiveTab::Doctor => ActiveTab::Help,
            ActiveTab::Help => ActiveTab::Rules,
        };
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match &mut self.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.should_quit = true
                }
                KeyCode::Tab => self.switch_tab(),
                KeyCode::Char('1') => self.active_tab = ActiveTab::Rules,
                KeyCode::Char('2') => self.active_tab = ActiveTab::Sessions,
                KeyCode::Char('3') | KeyCode::Char('p') => self.active_tab = ActiveTab::Projects,
                KeyCode::Char('4') => self.active_tab = ActiveTab::Doctor,
                KeyCode::Char('?') | KeyCode::Char('5') => self.active_tab = ActiveTab::Help,
                KeyCode::Enter if self.active_tab == ActiveTab::Projects => {
                    self.switch_project(self.selected_project_idx)?;
                }
                KeyCode::Char('x') | KeyCode::Char('d')
                    if self.active_tab == ActiveTab::Projects =>
                {
                    self.deregister_selected_project()?;
                }
                KeyCode::Char('/') if self.active_tab == ActiveTab::Rules => {
                    self.input_mode = InputMode::Filter;
                }
                KeyCode::Char('j') | KeyCode::Down => self.next_item(),
                KeyCode::Char('k') | KeyCode::Up => self.prev_item(),
                KeyCode::Char('a') if self.active_tab == ActiveTab::Rules => {
                    self.toggle_archive_selected()?;
                }
                KeyCode::Char('d') | KeyCode::Char('x') if self.active_tab == ActiveTab::Rules => {
                    if self.current_selected_rule().is_some() {
                        self.input_mode = InputMode::ConfirmDelete;
                    }
                }
                KeyCode::Char('n') if self.active_tab == ActiveTab::Rules => {
                    self.input_mode = InputMode::NewRule {
                        field: 0,
                        key: String::new(),
                        val: String::new(),
                        anchor: String::new(),
                    };
                }
                KeyCode::Char('e') if self.active_tab == ActiveTab::Rules => {
                    if let Some(rule) = self.current_selected_rule() {
                        self.input_mode = InputMode::EditRule {
                            field: 0,
                            key: rule.key.clone(),
                            val: rule.val.clone(),
                            anchor: rule.anchor.clone().unwrap_or_default(),
                        };
                    }
                }
                KeyCode::Char('c') => {
                    self.input_mode = InputMode::NewSession {
                        summary: String::new(),
                    };
                }
                KeyCode::Char('S') => {
                    self.run_git_sync()?;
                }
                KeyCode::Char('i') if self.active_tab == ActiveTab::Doctor => {
                    self.install_selected_client_mcp()?;
                }
                KeyCode::Char('I') if self.active_tab == ActiveTab::Doctor => {
                    self.install_all_clients_mcp()?;
                }
                KeyCode::Char('r') => {
                    self.reload_data()?;
                    self.set_toast("Database and diagnostics reloaded");
                }
                KeyCode::Esc => {
                    if !self.filter_query.is_empty() {
                        self.filter_query.clear();
                        self.apply_filter();
                        self.set_toast("Filter cleared");
                    }
                }
                _ => {}
            },
            InputMode::Filter => match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Backspace => {
                    self.filter_query.pop();
                    self.apply_filter();
                }
                KeyCode::Char(c) => {
                    self.filter_query.push(c);
                    self.apply_filter();
                }
                _ => {}
            },
            InputMode::ConfirmDelete => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.delete_selected_rule()?;
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.set_toast("Deletion cancelled");
                }
                _ => {}
            },
            InputMode::NewRule {
                field,
                key: k,
                val: v,
                anchor: a,
            } => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.set_toast("Rule creation cancelled");
                }
                KeyCode::Tab => {
                    *field = (*field + 1) % 3;
                }
                KeyCode::BackTab => {
                    *field = (*field + 2) % 3;
                }
                KeyCode::Enter => {
                    if *field < 2 {
                        *field += 1;
                    } else {
                        let k_clone = k.clone();
                        let v_clone = v.clone();
                        let a_clone = a.clone();
                        if self.create_rule(&k_clone, &v_clone, Some(&a_clone))? {
                            self.input_mode = InputMode::Normal;
                        }
                    }
                }
                KeyCode::Backspace => match *field {
                    0 => {
                        k.pop();
                    }
                    1 => {
                        v.pop();
                    }
                    2 => {
                        a.pop();
                    }
                    _ => {}
                },
                KeyCode::Char(c) => match *field {
                    0 => k.push(c),
                    1 => v.push(c),
                    2 => a.push(c),
                    _ => {}
                },
                _ => {}
            },
            InputMode::EditRule {
                field,
                key: k,
                val: v,
                anchor: a,
            } => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.set_toast("Rule edit cancelled");
                }
                KeyCode::Tab => {
                    *field = (*field + 1) % 2;
                }
                KeyCode::BackTab => {
                    *field = (*field + 1) % 2;
                }
                KeyCode::Enter => {
                    if *field < 1 {
                        *field += 1;
                    } else {
                        let k_clone = k.clone();
                        let v_clone = v.clone();
                        let a_clone = a.clone();
                        if self.create_rule(&k_clone, &v_clone, Some(&a_clone))? {
                            self.input_mode = InputMode::Normal;
                        }
                    }
                }
                KeyCode::Backspace => match *field {
                    0 => {
                        v.pop();
                    }
                    1 => {
                        a.pop();
                    }
                    _ => {}
                },
                KeyCode::Char(c) => match *field {
                    0 => v.push(c),
                    1 => a.push(c),
                    _ => {}
                },
                _ => {}
            },
            InputMode::NewSession { summary } => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.set_toast("Session checkpoint cancelled");
                }
                KeyCode::Enter => {
                    let s_clone = summary.clone();
                    if self.add_session(&s_clone)? {
                        self.input_mode = InputMode::Normal;
                    }
                }
                KeyCode::Backspace => {
                    summary.pop();
                }
                KeyCode::Char(c) => {
                    summary.push(c);
                }
                _ => {}
            },
        }
        Ok(())
    }
}

pub fn run(root: &Path) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set up panic hook to ensure terminal is ALWAYS restored!
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        default_hook(panic_info);
    }));

    let mut app = App::new(root.to_path_buf())?;
    let res = run_event_loop(&mut terminal, &mut app);

    // Restore terminal cleanly
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    res
}

fn run_event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|f| render_ui(f, app))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == event::KeyEventKind::Press
        {
            app.handle_key(key)?;
        }

        // Clean expired toasts after 3 seconds
        if let Some((_, timestamp)) = app.toast
            && timestamp.elapsed() > Duration::from_secs(3)
        {
            app.toast = None;
        }
    }
    Ok(())
}

fn render_ui(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header & Tabs
            Constraint::Min(8),    // Main Content
            Constraint::Length(1), // Footer / Status bar
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);

    match app.active_tab {
        ActiveTab::Rules => render_rules_tab(f, app, chunks[1]),
        ActiveTab::Sessions => render_sessions_tab(f, app, chunks[1]),
        ActiveTab::Projects => render_projects_tab(f, app, chunks[1]),
        ActiveTab::Doctor => render_doctor_tab(f, app, chunks[1]),
        ActiveTab::Help => render_help_tab(f, chunks[1]),
    }

    render_footer(f, app, chunks[2]);

    match &app.input_mode {
        InputMode::ConfirmDelete => render_delete_modal(f, app),
        InputMode::NewRule {
            field,
            key,
            val,
            anchor,
        } => render_new_rule_modal(f, *field, key, val, anchor),
        InputMode::EditRule {
            field,
            key,
            val,
            anchor,
        } => render_edit_rule_modal(f, *field, key, val, anchor),
        InputMode::NewSession { summary } => render_new_session_modal(f, summary),
        InputMode::Normal | InputMode::Filter => {}
    }
}

fn render_header(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24), // Title
            Constraint::Min(25),    // Tabs
            Constraint::Length(30), // Meta info
        ])
        .split(area);

    // Title
    let title = Paragraph::new(Line::from(vec![
        Span::styled(" agent-mem ", Style::default().fg(ACCENT).bold()),
        Span::styled("tui", Style::default().fg(EMERALD).bold()),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SUBTLE)),
    );
    f.render_widget(title, header_chunks[0]);

    // Tabs
    let rules_label = format!(" 1 Rules ({}) ", app.rules.len());
    let sessions_label = format!(" 2 Sessions ({}) ", app.sessions.len());
    let projects_label = format!(" 3 Projects ({}) ", app.projects.len());
    let doctor_label = " 4 Doctor & AIs ";
    let help_label = " ? Help ";

    let titles = vec![
        rules_label,
        sessions_label,
        projects_label,
        doctor_label.to_string(),
        help_label.to_string(),
    ];
    let active_index = match app.active_tab {
        ActiveTab::Rules => 0,
        ActiveTab::Sessions => 1,
        ActiveTab::Projects => 2,
        ActiveTab::Doctor => 3,
        ActiveTab::Help => 4,
    };

    let tabs = Tabs::new(titles)
        .select(active_index)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(SUBTLE)),
        )
        .highlight_style(Style::default().fg(ACCENT).bold())
        .style(Style::default().fg(MUTED))
        .divider(Span::styled("·", Style::default().fg(SUBTLE)));
    f.render_widget(tabs, header_chunks[1]);

    // Meta / Repo
    let repo_name = app
        .root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());
    let meta = Paragraph::new(Line::from(vec![
        Span::styled("repo: ", Style::default().fg(MUTED)),
        Span::styled(repo_name, Style::default().fg(ACCENT)),
        Span::styled(" · ", Style::default().fg(SUBTLE)),
        Span::styled("WAL", Style::default().fg(EMERALD)),
    ]))
    .alignment(Alignment::Right)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SUBTLE)),
    );
    f.render_widget(meta, header_chunks[2]);
}

fn render_rules_tab(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);

    // Left column: Filter bar + Rule list
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(main_chunks[0]);

    // Search filter input
    let search_border_style = if app.input_mode == InputMode::Filter {
        Style::default().fg(EMERALD).bold()
    } else if !app.filter_query.is_empty() {
        Style::default().fg(AMBER)
    } else {
        Style::default().fg(SUBTLE)
    };

    let filter_text = if app.filter_query.is_empty() && app.input_mode != InputMode::Filter {
        Span::styled("press '/' to search...", Style::default().fg(MUTED))
    } else {
        Span::styled(&app.filter_query, Style::default().fg(ACCENT).bold())
    };

    let search_box = Paragraph::new(Line::from(vec![
        Span::styled("🔍 ", Style::default()),
        filter_text,
    ]))
    .block(
        Block::default()
            .title(" Filter ")
            .title_style(Style::default().fg(MUTED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(search_border_style),
    );
    f.render_widget(search_box, left_chunks[0]);

    // Rules List
    let items: Vec<ListItem> = app
        .filtered_indices
        .iter()
        .enumerate()
        .map(|(display_idx, &actual_idx)| {
            let rule = &app.rules[actual_idx];
            let is_selected = display_idx == app.selected_rule_idx;

            let (icon, icon_style) = if rule.is_archived() {
                ("⊘", Style::default().fg(AMBER))
            } else {
                ("✓", Style::default().fg(EMERALD))
            };

            let key_style = if is_selected {
                Style::default().fg(ACCENT).bold()
            } else if rule.is_archived() {
                Style::default().fg(MUTED)
            } else {
                Style::default().fg(ACCENT)
            };

            let prefix = if is_selected { "▶ " } else { "  " };

            let kind_badge = match rule.kind.as_str() {
                "decision" => Span::styled("[dec] ", Style::default().fg(Color::Cyan)),
                "gotcha" => Span::styled("[gotcha] ", Style::default().fg(AMBER)),
                "pattern" => Span::styled("[pat] ", Style::default().fg(Color::Magenta)),
                _ => Span::styled("[rule] ", Style::default().fg(MUTED)),
            };

            let content = Line::from(vec![
                Span::styled(prefix, Style::default().fg(EMERALD)),
                Span::styled(format!("{} ", icon), icon_style),
                kind_badge,
                Span::styled(&rule.key, key_style),
            ]);

            let item_style = if is_selected {
                Style::default().bg(BG_SELECT)
            } else {
                Style::default()
            };

            ListItem::new(content).style(item_style)
        })
        .collect();

    let list_title = if !app.filter_query.is_empty() {
        format!(
            " Rules ({}/{}) ",
            app.filtered_indices.len(),
            app.rules.len()
        )
    } else {
        format!(" Rules ({}) ", app.rules.len())
    };

    let rules_list = List::new(items).block(
        Block::default()
            .title(list_title)
            .title_style(Style::default().fg(MUTED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SUBTLE)),
    );
    f.render_widget(rules_list, left_chunks[1]);

    // Right column: Detail Inspector
    let selected_rule = app.current_selected_rule();
    let detail_block = Block::default()
        .title(" Inspector ")
        .title_style(Style::default().fg(MUTED))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE));

    if let Some(rule) = selected_rule {
        let status_badge = if rule.is_archived() {
            Span::styled(" [ARCHIVED] ", Style::default().fg(AMBER).bold())
        } else {
            Span::styled(" [ACTIVE] ", Style::default().fg(EMERALD).bold())
        };

        let kind_badge = match rule.kind.as_str() {
            "decision" => Span::styled(" [DECISION] ", Style::default().fg(Color::Cyan).bold()),
            "gotcha" => Span::styled(" [GOTCHA] ", Style::default().fg(AMBER).bold()),
            "pattern" => Span::styled(" [PATTERN] ", Style::default().fg(Color::Magenta).bold()),
            _ => Span::styled(" [RULE] ", Style::default().fg(MUTED).bold()),
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Key: ", Style::default().fg(MUTED)),
                Span::styled(&rule.key, Style::default().fg(ACCENT).bold()),
                status_badge,
                kind_badge,
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Content:",
                Style::default().fg(MUTED).underlined(),
            )),
            Line::from(Span::styled(&rule.val, Style::default().fg(ACCENT))),
            Line::from(""),
        ];

        if let Some(anchor) = &rule.anchor {
            lines.push(Line::from(vec![
                Span::styled("Source Anchor: ", Style::default().fg(MUTED)),
                Span::styled(format!("📍 {}", anchor), Style::default().fg(EMERALD)),
            ]));
            lines.push(Line::from(""));
        }

        // Graph relations attached to this rule
        let connected_rels: Vec<&crate::store::RelationRecord> = app
            .relations
            .iter()
            .filter(|r| r.source_key == rule.key || r.target_key == rule.key)
            .collect();

        if !connected_rels.is_empty() {
            lines.push(Line::from(Span::styled(
                "Knowledge Graph Relations:",
                Style::default().fg(MUTED).underlined(),
            )));
            for rel in connected_rels {
                if rel.source_key == rule.key {
                    lines.push(Line::from(vec![
                        Span::styled("  ➜ ", Style::default().fg(EMERALD)),
                        Span::styled(&rel.rel_type, Style::default().fg(Color::Cyan)),
                        Span::styled(" ➜ ", Style::default().fg(SUBTLE)),
                        Span::styled(&rel.target_key, Style::default().fg(ACCENT)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("  ⬅ ", Style::default().fg(AMBER)),
                        Span::styled(&rel.rel_type, Style::default().fg(Color::Cyan)),
                        Span::styled(" by ", Style::default().fg(SUBTLE)),
                        Span::styled(&rel.source_key, Style::default().fg(ACCENT)),
                    ]));
                }
            }
            lines.push(Line::from(""));
        }

        if let Some(reason) = &rule.archive_reason {
            lines.push(Line::from(vec![
                Span::styled("Archival Reason: ", Style::default().fg(AMBER).bold()),
                Span::styled(reason, Style::default().fg(AMBER)),
            ]));
            if let Some(at) = rule.archived_at {
                lines.push(Line::from(vec![
                    Span::styled("Archived At: ", Style::default().fg(MUTED)),
                    Span::styled(format!("timestamp {}", at), Style::default().fg(MUTED)),
                ]));
            }
            lines.push(Line::from(""));
        }

        let detail_p = Paragraph::new(lines)
            .block(detail_block)
            .wrap(Wrap { trim: false });
        f.render_widget(detail_p, main_chunks[1]);
    } else {
        let empty_msg = Paragraph::new("No rule selected")
            .style(Style::default().fg(MUTED))
            .alignment(Alignment::Center)
            .block(detail_block);
        f.render_widget(empty_msg, main_chunks[1]);
    }
}

fn render_sessions_tab(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let session_items: Vec<ListItem> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(idx, (id, summary))| {
            let is_selected = idx == app.selected_session_idx;
            let prefix = if is_selected { "▶ " } else { "  " };
            let style = if is_selected {
                Style::default().bg(BG_SELECT).fg(ACCENT).bold()
            } else {
                Style::default().fg(MUTED)
            };
            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(EMERALD)),
                Span::styled(format!("[#{}] ", id), Style::default().fg(EMERALD).bold()),
                Span::styled(summary, Style::default().fg(MUTED)),
            ]);
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(session_items).block(
        Block::default()
            .title(format!(" Checkpoints ({}) ", app.sessions.len()))
            .title_style(Style::default().fg(MUTED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SUBTLE)),
    );
    f.render_widget(list, chunks[0]);

    let detail_block = Block::default()
        .title(" Checkpoint Summary ")
        .title_style(Style::default().fg(MUTED))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE));

    if let Some((id, summary)) = app.sessions.get(app.selected_session_idx) {
        let lines = vec![
            Line::from(vec![Span::styled(
                format!("Session Checkpoint #{}", id),
                Style::default().fg(ACCENT).bold(),
            )]),
            Line::from(""),
            Line::from(Span::styled(
                "Summary:",
                Style::default().fg(MUTED).underlined(),
            )),
            Line::from(Span::styled(summary, Style::default().fg(ACCENT))),
        ];
        let p = Paragraph::new(lines)
            .block(detail_block)
            .wrap(Wrap { trim: false });
        f.render_widget(p, chunks[1]);
    } else {
        let empty = Paragraph::new("No session selected")
            .style(Style::default().fg(MUTED))
            .alignment(Alignment::Center)
            .block(detail_block);
        f.render_widget(empty, chunks[1]);
    }
}

fn render_projects_tab(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);

    let project_items: Vec<ListItem> = app
        .projects
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let is_selected = idx == app.selected_project_idx;
            let is_current = Path::new(&p.canonical_path) == app.root;
            let prefix = if is_selected { "▶ " } else { "  " };
            let current_badge = if is_current { " [ACTIVE]" } else { "" };
            let style = if is_selected {
                Style::default().bg(BG_SELECT).fg(ACCENT).bold()
            } else {
                Style::default().fg(MUTED)
            };

            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(EMERALD)),
                Span::styled(
                    format!("{:<18} ", p.name),
                    Style::default()
                        .fg(if is_current { EMERALD } else { ACCENT })
                        .bold(),
                ),
                Span::styled(
                    format!("{:>2} rules", p.rules_count),
                    Style::default().fg(MUTED),
                ),
                Span::styled(current_badge, Style::default().fg(EMERALD).bold()),
            ]);
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(project_items).block(
        Block::default()
            .title(format!(
                " Registered Repositories ({}) ",
                app.projects.len()
            ))
            .title_style(Style::default().fg(MUTED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SUBTLE)),
    );
    f.render_widget(list, chunks[0]);

    let detail_block = Block::default()
        .title(" Repository Overview & Cockpit ")
        .title_style(Style::default().fg(MUTED))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE));

    if let Some(p) = app.projects.get(app.selected_project_idx) {
        let is_current = Path::new(&p.canonical_path) == app.root;
        let exists = Path::new(&p.canonical_path).exists();
        let status_span = if !exists {
            Span::styled("Missing from disk ✗", Style::default().fg(AMBER).bold())
        } else if is_current {
            Span::styled("Active Session Repo ✓", Style::default().fg(EMERALD).bold())
        } else {
            Span::styled(
                "Available (press Enter to switch) ✓",
                Style::default().fg(ACCENT),
            )
        };

        let lines = vec![
            Line::from(vec![
                Span::styled("Project: ", Style::default().fg(MUTED)),
                Span::styled(&p.name, Style::default().fg(ACCENT).bold()),
                Span::styled("  ·  ID: ", Style::default().fg(SUBTLE)),
                Span::styled(&p.id, Style::default().fg(EMERALD)),
            ]),
            Line::from(vec![
                Span::styled("Status:  ", Style::default().fg(MUTED)),
                status_span,
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Path: ", Style::default().fg(MUTED)),
                Span::styled(&p.canonical_path, Style::default().fg(ACCENT)),
            ]),
            Line::from(vec![
                Span::styled("Git:  ", Style::default().fg(MUTED)),
                Span::styled(
                    p.git_remote
                        .as_deref()
                        .unwrap_or("No git remote origin configured"),
                    Style::default().fg(if p.git_remote.is_some() {
                        EMERALD
                    } else {
                        MUTED
                    }),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Active Rules:   ", Style::default().fg(MUTED)),
                Span::styled(
                    p.rules_count.to_string(),
                    Style::default().fg(ACCENT).bold(),
                ),
                Span::styled(" active (", Style::default().fg(SUBTLE)),
                Span::styled(p.archived_count.to_string(), Style::default().fg(MUTED)),
                Span::styled(" archived)", Style::default().fg(SUBTLE)),
            ]),
            Line::from(vec![
                Span::styled("Checkpoints:    ", Style::default().fg(MUTED)),
                Span::styled(
                    p.sessions_count.to_string(),
                    Style::default().fg(ACCENT).bold(),
                ),
                Span::styled(" session checkpoints", Style::default().fg(SUBTLE)),
            ]),
            Line::from(vec![
                Span::styled("Database Size:  ", Style::default().fg(MUTED)),
                Span::styled(
                    format!("{:.1} KB", (p.db_size_bytes as f64) / 1024.0),
                    Style::default().fg(ACCENT),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Controls: ", Style::default().fg(MUTED).underlined()),
                Span::styled("[Enter] ", Style::default().fg(EMERALD).bold()),
                Span::styled("Switch to project  ·  ", Style::default().fg(MUTED)),
                Span::styled("[x] ", Style::default().fg(AMBER).bold()),
                Span::styled("Deregister from list", Style::default().fg(MUTED)),
            ]),
        ];
        let p_widget = Paragraph::new(lines)
            .block(detail_block)
            .wrap(Wrap { trim: false });
        f.render_widget(p_widget, chunks[1]);
    } else {
        let empty = Paragraph::new("No projects registered. Run 'agent-mem init' in repositories.")
            .style(Style::default().fg(MUTED))
            .alignment(Alignment::Center)
            .block(detail_block);
        f.render_widget(empty, chunks[1]);
    }
}

fn render_doctor_tab(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let top_bottom = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(8)])
        .split(area);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top_bottom[0]);

    // Storage Status
    let active_rules = app.rules.iter().filter(|r| !r.is_archived()).count();
    let archived_rules = app.rules.len() - active_rules;
    let storage_lines = vec![
        Line::from(vec![
            Span::styled("Database: ", Style::default().fg(MUTED)),
            Span::styled(
                app.db_path.display().to_string(),
                Style::default().fg(ACCENT),
            ),
        ]),
        Line::from(vec![
            Span::styled("Journal Mode: ", Style::default().fg(MUTED)),
            Span::styled("WAL (Write-Ahead Log)", Style::default().fg(EMERALD)),
        ]),
        Line::from(vec![
            Span::styled("Rules in memory: ", Style::default().fg(MUTED)),
            Span::styled(
                format!("{} active", active_rules),
                Style::default().fg(EMERALD),
            ),
            Span::styled(
                format!(" ({} archived)", archived_rules),
                Style::default().fg(AMBER),
            ),
        ]),
        Line::from(vec![
            Span::styled("Full-Text Search: ", Style::default().fg(MUTED)),
            Span::styled(
                "SQLite FTS5 BM25 (porter tokenizer)",
                Style::default().fg(ACCENT),
            ),
        ]),
    ];
    let storage_block = Paragraph::new(storage_lines).block(
        Block::default()
            .title(" Storage & Database ")
            .title_style(Style::default().fg(MUTED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SUBTLE)),
    );
    f.render_widget(storage_block, top_chunks[0]);

    // Git Status
    let git = &app.git_doctor;
    let git_lines = vec![
        Line::from(vec![
            Span::styled("Repository: ", Style::default().fg(MUTED)),
            Span::styled(
                if git.is_git_repo {
                    "Git active"
                } else {
                    "None"
                },
                Style::default().fg(EMERALD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Sync File: ", Style::default().fg(MUTED)),
            Span::styled(".agent-rules ", Style::default().fg(ACCENT)),
            Span::styled(
                if git.gitattributes_active {
                    "(merge=union ✓)"
                } else {
                    "(no union merge)"
                },
                if git.gitattributes_active {
                    Style::default().fg(EMERALD)
                } else {
                    Style::default().fg(AMBER)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled(".gitignore: ", Style::default().fg(MUTED)),
            Span::styled(
                if git.gitignore_active {
                    ".agent-mem/ ignored ✓"
                } else {
                    "not ignored ✗"
                },
                if git.gitignore_active {
                    Style::default().fg(EMERALD)
                } else {
                    Style::default().fg(AMBER)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Git Hooks: ", Style::default().fg(MUTED)),
            Span::styled(
                format!(
                    "post-commit:{}, post-merge:{}, post-checkout:{}, post-rewrite:{}",
                    if git.post_commit_active { "✓" } else { "·" },
                    if git.post_merge_active { "✓" } else { "·" },
                    if git.post_checkout_active {
                        "✓"
                    } else {
                        "·"
                    },
                    if git.post_rewrite_active { "✓" } else { "·" },
                ),
                Style::default().fg(EMERALD),
            ),
        ]),
    ];
    let git_block = Paragraph::new(git_lines).block(
        Block::default()
            .title(" Git & Team Sync ")
            .title_style(Style::default().fg(MUTED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SUBTLE)),
    );
    f.render_widget(git_block, top_chunks[1]);

    // Detected AI Clients
    let detected_clients: Vec<&ClientStatus> = app
        .client_statuses
        .iter()
        .filter(|c| c.installed || c.configured)
        .collect();

    let client_items: Vec<ListItem> = detected_clients
        .iter()
        .enumerate()
        .map(|(idx, client)| {
            let is_selected = idx == app.selected_client_idx;
            let (icon, icon_style) = if client.configured {
                ("✓ configured", Style::default().fg(EMERALD).bold())
            } else {
                (
                    "! detected (not configured)",
                    Style::default().fg(AMBER).bold(),
                )
            };
            let prefix = if is_selected { "▶ " } else { "  " };
            let p_str = client
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();

            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(EMERALD)),
                Span::styled(
                    format!("{:<20} ", client.client.display_name()),
                    Style::default().fg(ACCENT).bold(),
                ),
                Span::styled(format!("{:<28} ", icon), icon_style),
                Span::styled(p_str, Style::default().fg(MUTED)),
            ]);

            let item_style = if is_selected {
                Style::default().bg(BG_SELECT)
            } else {
                Style::default()
            };

            ListItem::new(line).style(item_style)
        })
        .collect();

    let client_list = List::new(client_items).block(
        Block::default()
            .title(format!(" Detected AI Clients & MCP ({} on system) - [i] Install selected, [I] Install ALL ", detected_clients.len()))
            .title_style(Style::default().fg(MUTED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(SUBTLE)),
    );
    f.render_widget(client_list, top_bottom[1]);
}

fn render_help_tab(f: &mut ratatui::Frame, area: Rect) {
    let help_text = vec![
        Line::from(Span::styled(
            "agent-mem TUI - The Complete Developer Cockpit",
            Style::default().fg(ACCENT).bold(),
        )),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Navigation & Tabs:",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    j / ↓ , k / ↑ ", Style::default().fg(ACCENT)),
            Span::styled(
                "Move cursor up/down across items",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    Tab           ", Style::default().fg(ACCENT)),
            Span::styled(
                "Cycle between Rules, Sessions, Projects, Doctor, and Help tabs",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    1, 2, 3, 4, ? ", Style::default().fg(ACCENT)),
            Span::styled(
                "Jump directly to Rules, Sessions, Projects, Doctor, or Help",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Rule Management (Tab 1):",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    n             ", Style::default().fg(ACCENT)),
            Span::styled(
                "Create new rule (interactive modal: Key, Value, Anchor)",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    e             ", Style::default().fg(ACCENT)),
            Span::styled(
                "Edit selected rule (Content & Anchor)",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    a             ", Style::default().fg(ACCENT)),
            Span::styled(
                "Toggle Archive / Reactivate selected rule",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    d / x         ", Style::default().fg(ACCENT)),
            Span::styled(
                "Permanently delete selected rule (prompts confirmation)",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    /             ", Style::default().fg(ACCENT)),
            Span::styled(
                "Real-time search filter across key, value, and anchor",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    S (Shift+s)   ", Style::default().fg(ACCENT)),
            Span::styled(
                "Run Git Sync with .agent-rules immediately",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Sessions & Checkpoints (Tab 2):",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    c             ", Style::default().fg(ACCENT)),
            Span::styled(
                "Record a manual session checkpoint summary",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Canonical Projects Explorer (Tab 3):",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    Enter         ", Style::default().fg(ACCENT)),
            Span::styled(
                "Switch active working repository immediately to selected project",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    x / d         ", Style::default().fg(ACCENT)),
            Span::styled(
                "Deregister selected project from canonical registry",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Doctor & AI Integration (Tab 4):",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    i             ", Style::default().fg(ACCENT)),
            Span::styled(
                "Install / configure MCP server for selected AI client",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    I (Shift+i)   ", Style::default().fg(ACCENT)),
            Span::styled(
                "Install MCP server for ALL detected AI clients in 1 step",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  General:",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    r             ", Style::default().fg(ACCENT)),
            Span::styled(
                "Reload database, rules, and system diagnostics",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    q / Ctrl+c    ", Style::default().fg(ACCENT)),
            Span::styled(
                "Cleanly exit agent-mem TUI and restore terminal",
                Style::default().fg(MUTED),
            ),
        ]),
    ];

    let p = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(" Help & Keyboard Shortcuts ")
                .title_style(Style::default().fg(MUTED))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(SUBTLE)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_footer(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let left_spans = match &app.input_mode {
        InputMode::Normal => match app.active_tab {
            ActiveTab::Rules => vec![
                Span::styled(" [Tab] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Tab  ", Style::default().fg(MUTED)),
                Span::styled("[n] ", Style::default().fg(ACCENT).bold()),
                Span::styled("New  ", Style::default().fg(MUTED)),
                Span::styled("[e] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Edit  ", Style::default().fg(MUTED)),
                Span::styled("[a] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Archive  ", Style::default().fg(MUTED)),
                Span::styled("[d] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Delete  ", Style::default().fg(MUTED)),
                Span::styled("[S] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Sync  ", Style::default().fg(MUTED)),
                Span::styled("[/] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Search  ", Style::default().fg(MUTED)),
                Span::styled("[q] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Quit", Style::default().fg(MUTED)),
            ],
            ActiveTab::Sessions => vec![
                Span::styled(" [Tab] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Tab  ", Style::default().fg(MUTED)),
                Span::styled("[c] ", Style::default().fg(ACCENT).bold()),
                Span::styled("New Checkpoint  ", Style::default().fg(MUTED)),
                Span::styled("[r] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Reload  ", Style::default().fg(MUTED)),
                Span::styled("[q] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Quit", Style::default().fg(MUTED)),
            ],
            ActiveTab::Projects => vec![
                Span::styled(" [Tab] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Tab  ", Style::default().fg(MUTED)),
                Span::styled("[Enter] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Switch Project  ", Style::default().fg(MUTED)),
                Span::styled("[x] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Deregister  ", Style::default().fg(MUTED)),
                Span::styled("[r] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Reload  ", Style::default().fg(MUTED)),
                Span::styled("[q] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Quit", Style::default().fg(MUTED)),
            ],
            ActiveTab::Doctor => vec![
                Span::styled(" [Tab] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Tab  ", Style::default().fg(MUTED)),
                Span::styled("[i] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Install MCP  ", Style::default().fg(MUTED)),
                Span::styled("[I] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Install ALL  ", Style::default().fg(MUTED)),
                Span::styled("[r] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Reload  ", Style::default().fg(MUTED)),
                Span::styled("[q] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Quit", Style::default().fg(MUTED)),
            ],
            ActiveTab::Help => vec![
                Span::styled(" [Tab] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Tab  ", Style::default().fg(MUTED)),
                Span::styled("[q] ", Style::default().fg(ACCENT).bold()),
                Span::styled("Quit", Style::default().fg(MUTED)),
            ],
        },
        InputMode::Filter => vec![
            Span::styled(" Filter Query: ", Style::default().fg(EMERALD).bold()),
            Span::styled("[Enter] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Confirm  ", Style::default().fg(MUTED)),
            Span::styled("[Esc] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Exit Search", Style::default().fg(MUTED)),
        ],
        InputMode::ConfirmDelete => vec![
            Span::styled(" CONFIRM DELETION: ", Style::default().fg(AMBER).bold()),
            Span::styled("[y] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Delete  ", Style::default().fg(AMBER)),
            Span::styled("[n / Esc] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Cancel", Style::default().fg(MUTED)),
        ],
        InputMode::NewRule { .. } | InputMode::EditRule { .. } => vec![
            Span::styled(" FORM: ", Style::default().fg(EMERALD).bold()),
            Span::styled("[Tab] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Next Field  ", Style::default().fg(MUTED)),
            Span::styled("[Enter] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Next / Submit  ", Style::default().fg(MUTED)),
            Span::styled("[Esc] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Cancel", Style::default().fg(MUTED)),
        ],
        InputMode::NewSession { .. } => vec![
            Span::styled(" NEW CHECKPOINT: ", Style::default().fg(EMERALD).bold()),
            Span::styled("[Enter] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Save Checkpoint  ", Style::default().fg(MUTED)),
            Span::styled("[Esc] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Cancel", Style::default().fg(MUTED)),
        ],
    };

    let mut right_spans = Vec::new();
    if let Some((msg, _)) = &app.toast {
        right_spans.push(Span::styled(
            format!("✓ {}  ", msg),
            Style::default().fg(EMERALD).bold(),
        ));
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(38)])
        .split(area);

    let left_p = Paragraph::new(Line::from(left_spans));
    let right_p = Paragraph::new(Line::from(right_spans)).alignment(Alignment::Right);

    f.render_widget(left_p, chunks[0]);
    f.render_widget(right_p, chunks[1]);
}

fn render_delete_modal(f: &mut ratatui::Frame, app: &App) {
    let rule_key = app
        .current_selected_rule()
        .map(|r| r.key.as_str())
        .unwrap_or("selected");

    let area = centered_rect(50, 7, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Confirm Deletion ")
        .title_style(Style::default().fg(AMBER).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(AMBER));

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Are you sure you want to delete ",
                Style::default().fg(ACCENT),
            ),
            Span::styled(format!("'{}'", rule_key), Style::default().fg(AMBER).bold()),
            Span::styled("?", Style::default().fg(ACCENT)),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  [y] Yes, delete rule    [n/Esc] Cancel  ",
            Style::default().fg(MUTED),
        )]),
    ];

    let p = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_new_rule_modal(f: &mut ratatui::Frame, field: usize, key: &str, val: &str, anchor: &str) {
    let area = centered_rect(65, 13, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" New Memory Rule ")
        .title_style(Style::default().fg(EMERALD).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(EMERALD));

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Key
            Constraint::Length(3), // Value
            Constraint::Length(3), // Anchor
            Constraint::Length(1), // Hint
        ])
        .margin(1)
        .split(area);

    f.render_widget(block, area);

    // Field 0: Key
    let key_style = if field == 0 {
        Style::default().fg(EMERALD).bold()
    } else {
        Style::default().fg(SUBTLE)
    };
    let key_box = Paragraph::new(key).block(
        Block::default()
            .title(" Rule Key (e.g. arch/auth) ")
            .title_style(Style::default().fg(if field == 0 { ACCENT } else { MUTED }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(key_style),
    );
    f.render_widget(key_box, inner[0]);

    // Field 1: Value
    let val_style = if field == 1 {
        Style::default().fg(EMERALD).bold()
    } else {
        Style::default().fg(SUBTLE)
    };
    let val_box = Paragraph::new(val).block(
        Block::default()
            .title(" Rule Content ")
            .title_style(Style::default().fg(if field == 1 { ACCENT } else { MUTED }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(val_style),
    );
    f.render_widget(val_box, inner[1]);

    // Field 2: Anchor
    let anchor_style = if field == 2 {
        Style::default().fg(EMERALD).bold()
    } else {
        Style::default().fg(SUBTLE)
    };
    let anchor_box = Paragraph::new(anchor).block(
        Block::default()
            .title(" Source Anchor (optional, e.g. src/auth.ts:12) ")
            .title_style(Style::default().fg(if field == 2 { ACCENT } else { MUTED }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(anchor_style),
    );
    f.render_widget(anchor_box, inner[2]);

    let hint = Paragraph::new("[Tab] Switch Field  ·  [Enter] Next / Submit  ·  [Esc] Cancel")
        .style(Style::default().fg(MUTED))
        .alignment(Alignment::Center);
    f.render_widget(hint, inner[3]);
}

fn render_edit_rule_modal(
    f: &mut ratatui::Frame,
    field: usize,
    key: &str,
    val: &str,
    anchor: &str,
) {
    let area = centered_rect(65, 11, f.area());
    f.render_widget(Clear, area);

    let title_str = format!(" Edit Rule: {} ", key);
    let block = Block::default()
        .title(title_str)
        .title_style(Style::default().fg(EMERALD).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(EMERALD));

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Value
            Constraint::Length(3), // Anchor
            Constraint::Length(1), // Hint
        ])
        .margin(1)
        .split(area);

    f.render_widget(block, area);

    // Field 0: Value
    let val_style = if field == 0 {
        Style::default().fg(EMERALD).bold()
    } else {
        Style::default().fg(SUBTLE)
    };
    let val_box = Paragraph::new(val).block(
        Block::default()
            .title(" Rule Content ")
            .title_style(Style::default().fg(if field == 0 { ACCENT } else { MUTED }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(val_style),
    );
    f.render_widget(val_box, inner[0]);

    // Field 1: Anchor
    let anchor_style = if field == 1 {
        Style::default().fg(EMERALD).bold()
    } else {
        Style::default().fg(SUBTLE)
    };
    let anchor_box = Paragraph::new(anchor).block(
        Block::default()
            .title(" Source Anchor (optional, e.g. src/auth.ts:12) ")
            .title_style(Style::default().fg(if field == 1 { ACCENT } else { MUTED }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(anchor_style),
    );
    f.render_widget(anchor_box, inner[1]);

    let hint = Paragraph::new("[Tab] Switch Field  ·  [Enter] Save Changes  ·  [Esc] Cancel")
        .style(Style::default().fg(MUTED))
        .alignment(Alignment::Center);
    f.render_widget(hint, inner[2]);
}

fn render_new_session_modal(f: &mut ratatui::Frame, summary: &str) {
    let area = centered_rect(65, 8, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Record Session Checkpoint ")
        .title_style(Style::default().fg(EMERALD).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(EMERALD));

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Summary input
            Constraint::Length(1), // Hint
        ])
        .margin(1)
        .split(area);

    f.render_widget(block, area);

    let summary_box = Paragraph::new(summary).block(
        Block::default()
            .title(" Checkpoint Summary (e.g. feat(auth): implemented JWT verification) ")
            .title_style(Style::default().fg(ACCENT))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(EMERALD).bold()),
    );
    f.render_widget(summary_box, inner[0]);

    let hint = Paragraph::new("[Enter] Record Checkpoint  ·  [Esc] Cancel")
        .style(Style::default().fg(MUTED))
        .alignment(Alignment::Center);
    f.render_widget(hint, inner[1]);
}

fn centered_rect(percent_x: u16, height_lines: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(r.height.saturating_sub(height_lines) / 2),
            Constraint::Length(height_lines),
            Constraint::Min(0),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
