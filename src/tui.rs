use crate::error::Result;
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
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filter,
    ConfirmDelete,
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
    pub sessions: Vec<SessionEntry>,
    pub selected_session_idx: usize,
    pub toast: Option<(String, Instant)>,
    pub should_quit: bool,
}

impl App {
    pub fn new(root: PathBuf) -> Result<Self> {
        let db_path = root.join(".agent-mem").join("mem.db");
        let mut app = Self {
            root,
            db_path,
            active_tab: ActiveTab::Rules,
            input_mode: InputMode::Normal,
            filter_query: String::new(),
            rules: Vec::new(),
            filtered_indices: Vec::new(),
            selected_rule_idx: 0,
            sessions: Vec::new(),
            selected_session_idx: 0,
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
            self.sessions = store.session_list(100).unwrap_or_default();
        } else {
            self.rules.clear();
            self.sessions.clear();
        }
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
            ActiveTab::Help => {}
        }
    }

    pub fn switch_tab(&mut self) {
        self.active_tab = match self.active_tab {
            ActiveTab::Rules => ActiveTab::Sessions,
            ActiveTab::Sessions => ActiveTab::Help,
            ActiveTab::Help => ActiveTab::Rules,
        };
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.should_quit = true
                }
                KeyCode::Tab => self.switch_tab(),
                KeyCode::Char('1') => self.active_tab = ActiveTab::Rules,
                KeyCode::Char('2') => self.active_tab = ActiveTab::Sessions,
                KeyCode::Char('?') => self.active_tab = ActiveTab::Help,
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
                KeyCode::Char('r') => {
                    self.reload_data()?;
                    self.set_toast("Database reloaded");
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
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Enter => {
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
        ActiveTab::Help => render_help_tab(f, chunks[1]),
    }

    render_footer(f, app, chunks[2]);

    if app.input_mode == InputMode::ConfirmDelete {
        render_delete_modal(f, app);
    }
}

fn render_header(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24), // Title
            Constraint::Min(20),    // Tabs
            Constraint::Length(28), // Meta info
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
    let help_label = " ? Help ";

    let titles = vec![rules_label, sessions_label, help_label.to_string()];
    let active_index = match app.active_tab {
        ActiveTab::Rules => 0,
        ActiveTab::Sessions => 1,
        ActiveTab::Help => 2,
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

            let content = Line::from(vec![
                Span::styled(prefix, Style::default().fg(EMERALD)),
                Span::styled(format!("{} ", icon), icon_style),
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

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Key: ", Style::default().fg(MUTED)),
                Span::styled(&rule.key, Style::default().fg(ACCENT).bold()),
                status_badge,
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

fn render_help_tab(f: &mut ratatui::Frame, area: Rect) {
    let help_text = vec![
        Line::from(Span::styled(
            "agent-mem TUI Keyboard Cheatsheet",
            Style::default().fg(ACCENT).bold(),
        )),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Navigation:",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    j / ↓       ", Style::default().fg(ACCENT)),
            Span::styled("Move cursor down", Style::default().fg(MUTED)),
        ]),
        Line::from(vec![
            Span::styled("    k / ↑       ", Style::default().fg(ACCENT)),
            Span::styled("Move cursor up", Style::default().fg(MUTED)),
        ]),
        Line::from(vec![
            Span::styled("    Tab         ", Style::default().fg(ACCENT)),
            Span::styled(
                "Switch between Rules, Sessions, and Help tabs",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    1, 2, ?     ", Style::default().fg(ACCENT)),
            Span::styled("Directly jump to tab", Style::default().fg(MUTED)),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Rule Operations:",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    /           ", Style::default().fg(ACCENT)),
            Span::styled(
                "Enter real-time search filter (Esc to clear)",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    a           ", Style::default().fg(ACCENT)),
            Span::styled(
                "Toggle Archive / Unarchive selected rule",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    d or x      ", Style::default().fg(ACCENT)),
            Span::styled(
                "Delete selected rule (prompts confirmation)",
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("    r           ", Style::default().fg(ACCENT)),
            Span::styled("Reload data from database", Style::default().fg(MUTED)),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Application:",
            Style::default().fg(EMERALD).bold(),
        )]),
        Line::from(vec![
            Span::styled("    q / Ctrl+c  ", Style::default().fg(ACCENT)),
            Span::styled(
                "Quit agent-mem TUI and restore terminal cleanly",
                Style::default().fg(MUTED),
            ),
        ]),
    ];

    let p = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(" Help & Shortcuts ")
                .title_style(Style::default().fg(MUTED))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(SUBTLE)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_footer(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let left_spans = match app.input_mode {
        InputMode::Normal => vec![
            Span::styled(" [Tab] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Switch Tab  ", Style::default().fg(MUTED)),
            Span::styled("[/] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Filter  ", Style::default().fg(MUTED)),
            Span::styled("[a] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Toggle Archive  ", Style::default().fg(MUTED)),
            Span::styled("[d] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Delete  ", Style::default().fg(MUTED)),
            Span::styled("[r] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Reload  ", Style::default().fg(MUTED)),
            Span::styled("[q] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Quit", Style::default().fg(MUTED)),
        ],
        InputMode::Filter => vec![
            Span::styled(
                " Type query to filter in real-time... ",
                Style::default().fg(EMERALD).bold(),
            ),
            Span::styled("[Enter] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Confirm  ", Style::default().fg(MUTED)),
            Span::styled("[Esc] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Exit Search", Style::default().fg(MUTED)),
        ],
        InputMode::ConfirmDelete => vec![
            Span::styled(" CONFIRM DELETION: ", Style::default().fg(AMBER).bold()),
            Span::styled("[y] ", Style::default().fg(ACCENT).bold()),
            Span::styled("Confirm Delete  ", Style::default().fg(AMBER)),
            Span::styled("[n / Esc] ", Style::default().fg(ACCENT).bold()),
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
        .constraints([Constraint::Min(20), Constraint::Length(35)])
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
            "  [y] Yes, permanently delete    [n/Esc] Cancel  ",
            Style::default().fg(MUTED),
        )]),
    ];

    let p = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center);
    f.render_widget(p, area);
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
