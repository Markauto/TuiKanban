use crate::{
    model::{self, Board, Card, Priority},
    recent::{self, RecentBoard},
    store::Store,
};
use anyhow::{ensure, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{
    io::{self, IsTerminal},
    time::Duration,
};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const BG: Color = Color::Rgb(17, 23, 34);
const HELP: &str = "NAVIGATE\n  ←/→ or h/l       Select column\n  ↑/↓ or j/k       Select card\n  Home / End       First / last card\n  Enter            View full card (↑/↓ scroll)\n\nCARDS\n  n                New card in selected column\n  e                Edit selected card\n  H / L            Move card left / right\n  p                Cycle priority\n  a                Archive / restore card\n  d                Delete with confirmation\n\nCOLUMNS\n  N / E            Add / rename column\n  [ / ]            Reorder selected column\n  s                Stack below previous / unstack\n  Tab / Shift+Tab  Next / previous column\n\nBOARD\n  b                Create / open / switch boards\n  /                Search ID, text and tags\n  Esc              Clear search / close dialog\n  v                Toggle active / archived cards\n  r                Reload from disk\n  ?                This help\n  q / Ctrl+C       Quit\n\nEDITOR\n  Tab / Shift+Tab  Change field\n  ←/→ or Space     Cycle priority / column\n  ←/→ Home/End     Move text cursor\n  Enter            New line in description\n  Ctrl+S           Save card\n  Esc              Cancel\n\nChanges save immediately. CLI changes refresh automatically.\nExport data and remove columns with `kanban --help`.";

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}
pub fn run(mut store: Store, allow_picker: bool) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "The TUI requires an interactive terminal; use `kanban list` in scripts"
    );
    let mut active = true;
    let board = match store.read() {
        Ok(board) => board,
        Err(_) if allow_picker && !store.path.exists() => {
            active = false;
            Board::new("Welcome".into(), vec!["Todo".into()])?
        }
        Err(error) => return Err(error),
    };
    let mut picker = if active {
        None
    } else {
        Some(BoardPicker::new())
    };
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        previous_hook(info);
    }));
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let mut app = App::new(board);
    if active {
        app.status = remember_status(&store);
    }
    loop {
        terminal.draw(|f| {
            if let Some(picker) = &picker {
                picker.draw(f);
            } else {
                app.draw(f);
            }
        })?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if let Some(current) = &mut picker {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        break;
                    }
                    if current.draft.is_none()
                        && matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
                    {
                        if !active {
                            break;
                        }
                        picker = None;
                    } else if let Some((next, board)) = current.key(key) {
                        store = next;
                        app = App::new(board);
                        app.status = remember_status(&store);
                        active = true;
                        picker = None;
                    }
                } else if matches!(app.mode, Mode::Normal) && key.code == KeyCode::Char('b') {
                    picker = Some(BoardPicker::new());
                } else if app.key(key, &store)? {
                    break;
                }
            }
        }
        if !active {
            continue;
        }
        match store.read() {
            Ok(board) => {
                app.board = board;
                app.clamp();
            }
            Err(error) => app.status = format!("Reload failed: {error}"),
        }
    }
    Ok(())
}
fn remember_status(store: &Store) -> String {
    match recent::remember(store) {
        Ok(()) => format!("Board: {}", store.path.display()),
        Err(error) => format!("Board opened; recent history unavailable: {error:#}"),
    }
}

struct BoardPicker {
    entries: Vec<RecentBoard>,
    selected: usize,
    draft: Option<Box<Draft>>,
    creating: bool,
    error: String,
}
impl BoardPicker {
    fn new() -> Self {
        let result = recent::history_path().and_then(|path| recent::read(&path));
        let (entries, error) = match result {
            Ok(entries) => (entries, String::new()),
            Err(error) => (Vec::new(), format!("History unavailable: {error:#}")),
        };
        Self {
            entries,
            selected: 0,
            draft: None,
            creating: false,
            error,
        }
    }
    fn key(&mut self, key: KeyEvent) -> Option<(Store, Board)> {
        if let Some(draft) = &mut self.draft {
            match key.code {
                KeyCode::Esc => {
                    self.draft = None;
                    self.error.clear();
                }
                KeyCode::Tab | KeyCode::BackTab if self.creating => draft.field = 1 - draft.field,
                KeyCode::Enter => {
                    let result = (|| -> Result<(Store, Board)> {
                        let path = draft.fields[1].trim();
                        ensure!(!path.is_empty(), "Enter a JSON file path");
                        let store = Store::new(path.into());
                        if self.creating {
                            let board = Board::new(
                                draft.fields[0].trim().into(),
                                vec!["Todo".into(), "In Progress".into(), "Done".into()],
                            )?;
                            store.init(board)?;
                        }
                        let board = store.read()?;
                        Ok((store, board))
                    })();
                    match result {
                        Ok(opened) => return Some(opened),
                        Err(error) => self.error = format!("{error:#}"),
                    }
                }
                _ => draft.input(key),
            }
        } else {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1))
                }
                KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
                KeyCode::Enter => {
                    if let Some(entry) = self.entries.get(self.selected) {
                        let store = Store::new(entry.path.clone());
                        match store.read() {
                            Ok(board) => return Some((store, board)),
                            Err(error) => self.error = format!("{error:#}"),
                        }
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('o') => {
                    self.creating = key.code == KeyCode::Char('n');
                    self.error.clear();
                    let mut draft = Draft::new(String::new(), None);
                    if self.creating {
                        match crate::cli::default_board_path() {
                            Ok(path) => {
                                draft.fields[1] = path
                                    .with_file_name("boards")
                                    .join(format!(
                                        "board-{}.json",
                                        chrono::Utc::now().format("%Y%m%d-%H%M%S-%f")
                                    ))
                                    .display()
                                    .to_string();
                                draft.cursors[1] = draft.fields[1].len();
                            }
                            Err(error) => self.error = format!("{error:#}"),
                        }
                    } else {
                        draft.field = 1;
                    }
                    self.draft = Some(Box::new(draft));
                }
                _ => {}
            }
        }
        None
    }
    fn draw(&self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Clear, area);
        if let Some(draft) = &self.draft {
            let text = if self.creating {
                format!("Name: {}\n\nJSON path: {}\n\nTab change field · Enter create · Esc cancel\n\n{}", draft.displayed(0), draft.displayed(1), self.error)
            } else {
                format!(
                    "JSON path: {}\n\nEnter open · Esc cancel\n\n{}",
                    draft.displayed(1),
                    self.error
                )
            };
            popup(
                f,
                if self.creating {
                    " Create board "
                } else {
                    " Open board "
                },
                &text,
                90,
                16,
                0,
            );
            return;
        }
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(4),
        ])
        .split(area);
        f.render_widget(Paragraph::new("KANBAN · Boards\nn Create new · o Open JSON · ↑/↓ Select · Enter Switch · Esc/q Back or quit"), chunks[0]);
        let items: Vec<_> = self
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(format!(
                    "{}\n  {}{}",
                    clean(&entry.name),
                    clean(&entry.path.display().to_string()),
                    if entry.path.exists() {
                        ""
                    } else {
                        " [missing]"
                    }
                ))
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(self.selected));
        f.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Recent boards "),
                )
                .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
                .highlight_symbol("› "),
            chunks[1],
            &mut state,
        );
        let message = if !self.error.is_empty() {
            self.error.as_str()
        } else if self.entries.is_empty() {
            "No recent boards. Press n to create one or o to open a JSON file."
        } else {
            "Each board saves immediately to its own JSON file."
        };
        f.render_widget(
            Paragraph::new(clean(message)).wrap(Wrap { trim: true }),
            chunks[2],
        );
    }
}

#[derive(Default)]
enum Mode {
    #[default]
    Normal,
    Search,
    Help(u16),
    Detail(u16),
    Edit(Box<Draft>),
    Delete(u64),
    Column {
        original: Option<String>,
        draft: Box<Draft>,
    },
}
struct Draft {
    original: Option<Card>,
    fields: [String; 4],
    cursors: [usize; 4],
    priority: Priority,
    column: String,
    field: usize,
    error: String,
}
impl Draft {
    fn new(column: String, card: Option<Card>) -> Self {
        let mut draft = match card {
            Some(c) => Self {
                cursors: [0; 4],
                fields: [
                    c.title.clone(),
                    c.description.clone(),
                    c.tags.join(", "),
                    c.due.map(|d| d.to_string()).unwrap_or_default(),
                ],
                priority: c.priority,
                column: c.column.clone(),
                original: Some(c),
                field: 0,
                error: String::new(),
            },
            None => Self {
                original: None,
                fields: Default::default(),
                cursors: [0; 4],
                priority: Priority::Medium,
                column,
                field: 0,
                error: String::new(),
            },
        };
        draft.cursors = std::array::from_fn(|i| draft.fields[i].len());
        draft
    }
    fn input(&mut self, key: KeyEvent) {
        let text = &mut self.fields[self.field];
        let cursor = &mut self.cursors[self.field];
        match key.code {
            KeyCode::Left => {
                *cursor = text[..*cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i)
            }
            KeyCode::Right => *cursor += text[*cursor..].chars().next().map_or(0, char::len_utf8),
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = text.len(),
            KeyCode::Backspace if *cursor > 0 => {
                let previous = text[..*cursor].char_indices().next_back().unwrap().0;
                text.drain(previous..*cursor);
                *cursor = previous;
            }
            KeyCode::Delete if *cursor < text.len() => {
                text.remove(*cursor);
            }
            KeyCode::Enter if self.field == 1 => {
                text.insert(*cursor, '\n');
                *cursor += 1;
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !c.is_control() =>
            {
                text.insert(*cursor, c);
                *cursor += c.len_utf8();
            }
            _ => {}
        }
    }
    fn displayed(&self, index: usize) -> String {
        let mut value = match index {
            0..=3 => self.fields[index].clone(),
            4 => self.priority.to_string(),
            _ => self.column.clone(),
        };
        if self.field == index {
            let cursor = if index < 4 {
                self.cursors[index]
            } else {
                value.len()
            };
            value.insert(cursor, '▏');
        }
        clean(&value)
    }
    fn save(&self, store: &Store) -> Result<u64> {
        let due = if self.fields[3].trim().is_empty() {
            None
        } else {
            Some(model::date(self.fields[3].trim())?)
        };
        store.update(|b| {
            let column = b.column(&self.column)?;
            if let Some(original) = &self.original {
                let card = b.card_mut(original.id)?;
                ensure!(
                    card.updated_at == original.updated_at,
                    "Card changed in another session. Cancel and reopen the editor."
                );
                card.title = self.fields[0].trim().into();
                card.description = self.fields[1].clone();
                card.tags = model::tags(&self.fields[2]);
                card.due = due;
                card.priority = self.priority;
                card.column = column;
                card.touch();
                Ok(card.id)
            } else {
                b.add(
                    self.fields[0].clone(),
                    &column,
                    self.fields[1].clone(),
                    self.priority,
                    model::tags(&self.fields[2]),
                    due,
                )
            }
        })
    }
}
struct App {
    board: Board,
    column: usize,
    row: usize,
    query: String,
    archived: bool,
    mode: Mode,
    status: String,
}
impl App {
    fn new(board: Board) -> Self {
        Self {
            board,
            column: 0,
            row: 0,
            query: String::new(),
            archived: false,
            mode: Mode::Normal,
            status: "Ready · changes save automatically".into(),
        }
    }
    fn cards(&self, column: usize) -> Vec<&Card> {
        self.board
            .cards
            .iter()
            .filter(|c| {
                c.column == self.board.columns[column]
                    && c.archived == self.archived
                    && c.matches(&self.query)
            })
            .collect()
    }
    fn selected(&self) -> Option<&Card> {
        self.cards(self.column).get(self.row).copied()
    }
    fn clamp(&mut self) {
        self.column = self.column.min(self.board.columns.len().saturating_sub(1));
        self.row = self
            .row
            .min(self.cards(self.column).len().saturating_sub(1));
    }
    fn refresh(&mut self, store: &Store) -> Result<()> {
        self.board = store.read()?;
        self.clamp();
        Ok(())
    }
    fn key(&mut self, key: KeyEvent, store: &Store) -> Result<bool> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }
        let mode = std::mem::take(&mut self.mode);
        self.mode = match mode {
            Mode::Normal => {
                if key.code == KeyCode::Char('q') {
                    return Ok(true);
                }
                match self.normal_key(key, store) {
                    Ok(mode) => mode,
                    Err(e) => {
                        self.status = format!("Error: {e}");
                        Mode::Normal
                    }
                }
            }
            Mode::Search => {
                match key.code {
                    KeyCode::Esc => {
                        self.query.clear();
                        self.row = 0;
                        return Ok(false);
                    }
                    KeyCode::Enter => return Ok(false),
                    KeyCode::Backspace => {
                        self.query.pop();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.query.push(c)
                    }
                    _ => {}
                }
                self.row = 0;
                Mode::Search
            }
            Mode::Help(scroll) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => Mode::Normal,
                KeyCode::Down | KeyCode::Char('j') => {
                    Mode::Help(scroll.saturating_add(1).min(HELP.lines().count() as u16))
                }
                KeyCode::Up | KeyCode::Char('k') => Mode::Help(scroll.saturating_sub(1)),
                _ => Mode::Help(scroll),
            },
            Mode::Detail(scroll) => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => Mode::Normal,
                KeyCode::Down | KeyCode::Char('j') => Mode::Detail(scroll.saturating_add(1)),
                KeyCode::Up | KeyCode::Char('k') => Mode::Detail(scroll.saturating_sub(1)),
                _ => Mode::Detail(scroll),
            },
            Mode::Delete(id) => {
                if key.code == KeyCode::Char('y') {
                    match store.update(|b| {
                        b.card(id)?;
                        b.cards.retain(|c| c.id != id);
                        Ok(())
                    }) {
                        Ok(()) => {
                            self.status = format!("Deleted #{id}");
                            self.refresh(store)?;
                        }
                        Err(e) => self.status = format!("Error: {e}"),
                    }
                    Mode::Normal
                } else if matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
                    Mode::Normal
                } else {
                    Mode::Delete(id)
                }
            }
            Mode::Column {
                original,
                mut draft,
            } => {
                if key.code == KeyCode::Esc {
                    Mode::Normal
                } else if key.code == KeyCode::Enter
                    || (key.code == KeyCode::Char('s')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    let name = draft.fields[0].trim().to_owned();
                    match store.update(|b| match &original {
                        Some(old) => b.rename_column(old, &name),
                        None => b.add_column(&name),
                    }) {
                        Ok(()) => {
                            self.refresh(store)?;
                            self.column = self
                                .board
                                .columns
                                .iter()
                                .position(|c| c == &name)
                                .unwrap_or(0);
                            self.row = 0;
                            self.status = format!("Saved column {name}");
                            Mode::Normal
                        }
                        Err(e) => {
                            draft.error = e.to_string();
                            Mode::Column { original, draft }
                        }
                    }
                } else {
                    draft.input(key);
                    Mode::Column { original, draft }
                }
            }
            Mode::Edit(mut draft) => {
                if key.code == KeyCode::Esc {
                    Mode::Normal
                } else if key.code == KeyCode::Char('s')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    match draft.save(store) {
                        Ok(id) => {
                            self.status = format!("Saved #{id}");
                            self.refresh(store)?;
                            self.column = self
                                .board
                                .columns
                                .iter()
                                .position(|c| c == &draft.column)
                                .unwrap_or(0);
                            self.query.clear();
                            self.row = self
                                .cards(self.column)
                                .iter()
                                .position(|c| c.id == id)
                                .unwrap_or(0);
                            Mode::Normal
                        }
                        Err(e) => {
                            draft.error = e.to_string();
                            Mode::Edit(draft)
                        }
                    }
                } else {
                    match key.code {
                        KeyCode::Tab => draft.field = (draft.field + 1) % 6,
                        KeyCode::BackTab => draft.field = (draft.field + 5) % 6,
                        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if draft.field >= 4 => {
                            if draft.field == 4 {
                                draft.priority = if key.code == KeyCode::Left {
                                    draft.priority.next().next().next()
                                } else {
                                    draft.priority.next()
                                };
                            } else {
                                let index = self
                                    .board
                                    .columns
                                    .iter()
                                    .position(|c| c == &draft.column)
                                    .unwrap_or(0);
                                let count = self.board.columns.len();
                                let next = if key.code == KeyCode::Left {
                                    (index + count - 1) % count
                                } else {
                                    (index + 1) % count
                                };
                                draft.column = self.board.columns[next].clone();
                            }
                        }
                        _ if draft.field < 4 => draft.input(key),
                        _ => {}
                    }
                    Mode::Edit(draft)
                }
            }
        };
        self.clamp();
        Ok(false)
    }
    fn normal_key(&mut self, key: KeyEvent, store: &Store) -> Result<Mode> {
        let id = self.selected().map(|c| c.id);
        match key.code {
            KeyCode::Char('N') | KeyCode::Char('E') => {
                let original = (key.code == KeyCode::Char('E'))
                    .then(|| self.board.columns[self.column].clone());
                let mut draft = Draft::new(String::new(), None);
                draft.fields[0] = original.clone().unwrap_or_default();
                draft.cursors[0] = draft.fields[0].len();
                return Ok(Mode::Column {
                    original,
                    draft: Box::new(draft),
                });
            }
            KeyCode::Char('s') => {
                let name = self.board.columns[self.column].clone();
                store.update(|b| b.stack_column(&name, !b.stacked_columns.contains(&name)))?;
                self.refresh(store)?;
                self.status = "Column layout saved".into();
            }
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let name = self.board.columns[self.column].clone();
                let target = if key.code == KeyCode::Char('[') {
                    self.column.saturating_sub(1)
                } else {
                    (self.column + 1).min(self.board.columns.len() - 1)
                };
                store.update(|b| b.order_column(&name, target + 1))?;
                self.refresh(store)?;
                self.column = self
                    .board
                    .columns
                    .iter()
                    .position(|c| c == &name)
                    .unwrap_or(0);
                self.status = "Column order saved".into();
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.column = self.column.saturating_sub(1);
                self.row = 0;
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.column = (self.column + 1).min(self.board.columns.len() - 1);
                self.row = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => self.row = self.row.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => self.row = self.row.saturating_sub(1),
            KeyCode::Home => self.row = 0,
            KeyCode::End => self.row = self.cards(self.column).len().saturating_sub(1),
            KeyCode::Char('?') => return Ok(Mode::Help(0)),
            KeyCode::Char('/') => return Ok(Mode::Search),
            KeyCode::Esc => {
                self.query.clear();
                self.row = 0;
            }
            KeyCode::Char('v') => {
                self.archived = !self.archived;
                self.row = 0;
            }
            KeyCode::Char('r') => {
                self.refresh(store)?;
                self.status = "Reloaded board".into();
            }
            KeyCode::Char('n') => {
                self.archived = false;
                return Ok(Mode::Edit(Box::new(Draft::new(
                    self.board.columns[self.column].clone(),
                    None,
                ))));
            }
            KeyCode::Char('e') if id.is_some() => {
                return Ok(Mode::Edit(Box::new(Draft::new(
                    self.board.columns[self.column].clone(),
                    self.selected().cloned(),
                ))))
            }
            KeyCode::Enter if id.is_some() => return Ok(Mode::Detail(0)),
            KeyCode::Char('d') if id.is_some() => return Ok(Mode::Delete(id.unwrap())),
            KeyCode::Char('a') if id.is_some() => {
                let id = id.unwrap();
                let archived = !self.archived;
                store.update(|b| {
                    let c = b.card_mut(id)?;
                    c.archived = archived;
                    c.touch();
                    Ok(())
                })?;
                self.status = format!("{} #{id}", if archived { "Archived" } else { "Restored" });
                self.refresh(store)?;
            }
            KeyCode::Char('p') if id.is_some() => {
                store.update(|b| {
                    let c = b.card_mut(id.unwrap())?;
                    c.priority = c.priority.next();
                    c.touch();
                    Ok(())
                })?;
                self.refresh(store)?;
                self.status = "Priority updated".into();
            }
            KeyCode::Char('H') | KeyCode::Char('L') if id.is_some() => {
                let target = if key.code == KeyCode::Char('H') {
                    self.column.saturating_sub(1)
                } else {
                    (self.column + 1).min(self.board.columns.len() - 1)
                };
                let column = self.board.columns[target].clone();
                store.update(|b| b.move_card(id.unwrap(), &column))?;
                self.refresh(store)?;
                self.column = target;
                self.row = self
                    .cards(target)
                    .iter()
                    .position(|c| Some(c.id) == id)
                    .unwrap_or(0);
                self.status = format!("Moved #{} to {column}", id.unwrap());
            }
            _ => {}
        }
        Ok(Mode::Normal)
    }
    fn draw(&self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(
            Block::default().style(Style::default().bg(BG).fg(Color::White)),
            area,
        );
        if area.width < 30 || area.height < 10 {
            f.render_widget(
                Paragraph::new("Terminal too small. Resize to at least 30×10. Ctrl+C to quit.")
                    .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(2),
                Constraint::Length(2),
            ])
            .split(area);
        let count = self
            .board
            .cards
            .iter()
            .filter(|c| c.archived == self.archived)
            .count();
        let header = Line::from(vec![
            Span::styled(
                " KANBAN ",
                Style::default()
                    .fg(BG)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}  ", clean(&self.board.name))),
            Span::styled(
                format!(
                    "{count} {} cards",
                    if self.archived { "archived" } else { "active" }
                ),
                Style::default().fg(MUTED),
            ),
        ]);
        f.render_widget(
            Paragraph::new(header).block(Block::default().borders(Borders::BOTTOM)),
            chunks[0],
        );
        let cells = column_cells(&self.board, self.column, chunks[1]);
        for &(column, cell) in &cells {
            let cards = self.cards(column);
            let selected = column == self.column;
            let border = if selected { ACCENT } else { MUTED };
            let title = format!(" {} · {} ", clean(&self.board.columns[column]), cards.len());
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(border));
            if cards.is_empty() {
                f.render_widget(
                    Paragraph::new("\n  No cards here\n\n  n  Create a card")
                        .style(Style::default().fg(MUTED))
                        .block(block),
                    cell,
                );
            } else {
                let items: Vec<_> = cards
                    .iter()
                    .map(|card| {
                        let due = card
                            .due
                            .map(|d| {
                                format!(
                                    " · {d}{} ",
                                    if d < chrono::Utc::now().date_naive() {
                                        " !"
                                    } else {
                                        ""
                                    }
                                )
                            })
                            .unwrap_or_default();
                        ListItem::new(vec![
                            Line::from(vec![
                                Span::styled(format!("#{} ", card.id), Style::default().fg(ACCENT)),
                                Span::styled(
                                    clean(&card.title),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                            ]),
                            Line::from(vec![
                                Span::styled(
                                    format!("{}", card.priority),
                                    Style::default().fg(priority_color(card.priority)),
                                ),
                                Span::styled(due, Style::default().fg(Color::Gray)),
                            ]),
                            Line::from(Span::styled(
                                if card.tags.is_empty() {
                                    " ".into()
                                } else {
                                    format!("#{}", card.tags.join(" #"))
                                },
                                Style::default().fg(MUTED),
                            )),
                            Line::from(""),
                        ])
                    })
                    .collect();
                let list = List::new(items)
                    .block(block)
                    .highlight_style(Style::default().bg(Color::Rgb(36, 53, 72)))
                    .highlight_symbol("▌ ");
                let mut state = ListState::default().with_selected(if selected {
                    Some(self.row)
                } else {
                    None
                });
                f.render_stateful_widget(list, cell, &mut state);
            }
        }
        let search = if matches!(self.mode, Mode::Search) {
            format!(" / {}▏   Enter apply · Esc clear", clean(&self.query))
        } else if !self.query.is_empty() {
            format!(" Filter: {} · Esc clear", clean(&self.query))
        } else {
            format!(
                " {}  · columns {}–{}/{}",
                clean(&self.status),
                cells.first().map_or(1, |(c, _)| c + 1),
                cells.last().map_or(1, |(c, _)| c + 1),
                self.board.columns.len()
            )
        };
        f.render_widget(
            Paragraph::new(search).style(Style::default().fg(ACCENT)),
            chunks[2],
        );
        f.render_widget(
            Paragraph::new(
                " ←↓↑→ navigate  b boards  n/e cards  N/E columns  s stack  [/] reorder  ? help  q quit",
            )
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true }),
            chunks[3],
        );
        match &self.mode {
            Mode::Help(scroll) => popup(f, " Keyboard shortcuts · ↑/↓ scroll · Esc close ", HELP, 72, 42, *scroll),
            Mode::Detail(scroll) => if let Some(c) = self.selected() {
                let text = format!("#{} {}\n\nColumn: {}\nPriority: {}\nTags: {}\nDue: {}\nCreated: {}\nUpdated: {}\n\n{}", c.id, c.title, c.column, c.priority, c.tags.join(", "), c.due.map(|d| d.to_string()).unwrap_or_else(|| "None".into()), c.created_at, c.updated_at, c.description);
                popup(f, " Card details · ↑/↓ scroll · Esc close ", &text, 85, 30, *scroll);
            },
            Mode::Delete(id) => popup(f, " Delete card ", &format!("Permanently delete card #{id}?\n\nThis cannot be undone.\n\ny Delete    n / Esc Cancel"), 56, 9, 0),
            Mode::Edit(draft) => draw_editor(f, draft),
            Mode::Column { original, draft } => popup(f,
                if original.is_some() { " Rename column " } else { " Add column " },
                &format!("Name: {}\n\nEnter / Ctrl+S save · Esc cancel\n{}", draft.displayed(0), draft.error), 70, 8, 0),
            _ => {}
        }
    }
}
// Page whole lanes horizontally and long stacks vertically, keeping selection visible.
fn column_cells(board: &Board, selected: usize, area: Rect) -> Vec<(usize, Rect)> {
    let stacks = board.column_stacks();
    let lane = stacks
        .iter()
        .position(|s| s.contains(&selected))
        .unwrap_or(0);
    let visible = (usize::from(area.width / 28)).max(1).min(stacks.len());
    let start = lane / visible * visible;
    let end = (start + visible).min(stacks.len());
    let lanes = Layout::horizontal(vec![
        Constraint::Ratio(1, (end - start) as u32);
        end - start
    ])
    .split(area);
    let mut cells = Vec::new();
    for (offset, stack) in stacks[start..end].iter().enumerate() {
        let rows = (usize::from(area.height / 6)).max(1).min(stack.len());
        let selected_row = stack.iter().position(|c| *c == selected).unwrap_or(0);
        let first = selected_row / rows * rows;
        let last = (first + rows).min(stack.len());
        let rects = Layout::vertical(vec![
            Constraint::Ratio(1, (last - first) as u32);
            last - first
        ])
        .split(lanes[offset]);
        cells.extend(
            stack[first..last]
                .iter()
                .copied()
                .zip(rects.iter().copied()),
        );
    }
    cells
}
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}
fn priority_color(p: Priority) -> Color {
    match p {
        Priority::Low => Color::Green,
        Priority::Medium => Color::Blue,
        Priority::High => Color::Yellow,
        Priority::Urgent => Color::Red,
    }
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
fn popup(f: &mut Frame, title: &str, text: &str, width: u16, height: u16, scroll: u16) {
    let area = centered(f.area(), width, height);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(clean(text))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(ACCENT)),
            )
            .style(Style::default().bg(BG).fg(Color::White))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}
fn draw_editor(f: &mut Frame, draft: &Draft) {
    let area = centered(f.area(), 86, 27);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(if draft.original.is_some() {
            " Edit card "
        } else {
            " New card "
        })
        .style(Style::default().bg(BG).fg(Color::White))
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 20 {
        let labels = [
            "Title",
            "Description",
            "Tags (comma separated)",
            "Due (YYYY-MM-DD)",
            "Priority (←/→)",
            "Column (←/→)",
        ];
        let value = draft.displayed(draft.field);
        f.render_widget(
            Paragraph::new(format!(
                "{} ({}/6)\n{}\n\nTab next · Ctrl+S save · Esc cancel\n{}",
                labels[draft.field],
                draft.field + 1,
                clean(&value),
                draft.error
            ))
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(6),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .split(inner);
    let labels = [
        "Title",
        "Description (Enter for newline)",
        "Tags (comma separated)",
        "Due (YYYY-MM-DD, empty clears)",
        "Priority (←/→ or Space)",
        "Column (←/→ or Space)",
    ];
    for index in 0..6 {
        let value = draft.displayed(index);
        let color = if draft.field == index { ACCENT } else { MUTED };
        let prefix = if index < 4 {
            &draft.fields[index][..draft.cursors[index]]
        } else {
            ""
        };
        let cursor_line = prefix.chars().filter(|c| *c == '\n').count();
        let cursor_width = Span::raw(prefix.rsplit('\n').next().unwrap_or("")).width();
        let scroll = if draft.field == index {
            cursor_line
                .saturating_sub(rows[index].height.saturating_sub(3) as usize)
                .min(u16::MAX as usize) as u16
        } else {
            0
        };
        let horizontal = if draft.field == index {
            cursor_width
                .saturating_sub(rows[index].width.saturating_sub(4) as usize)
                .min(u16::MAX as usize) as u16
        } else {
            0
        };
        f.render_widget(
            Paragraph::new(value).scroll((scroll, horizontal)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(labels[index])
                    .border_style(Style::default().fg(color)),
            ),
            rows[index],
        );
    }
    f.render_widget(
        Paragraph::new(if draft.error.is_empty() {
            "Tab next · Shift+Tab previous · Ctrl+S save · Esc cancel"
        } else {
            &draft.error
        })
        .style(Style::default().fg(if draft.error.is_empty() {
            ACCENT
        } else {
            Color::Red
        }))
        .wrap(Wrap { trim: true }),
        rows[6],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    fn board() -> Board {
        Board::new(
            "Launch".into(),
            vec!["Todo".into(), "Doing".into(), "Done".into()],
        )
        .unwrap()
    }
    #[test]
    fn board_picker_creates_opens_and_reports_errors_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.json");
        let mut picker = BoardPicker {
            entries: Vec::new(),
            selected: 0,
            draft: None,
            creating: false,
            error: String::new(),
        };
        let press =
            |picker: &mut BoardPicker, code| picker.key(KeyEvent::new(code, KeyModifiers::NONE));
        press(&mut picker, KeyCode::Char('o'));
        press(&mut picker, KeyCode::Enter);
        assert!(picker.error.contains("path"));
        press(&mut picker, KeyCode::Esc);
        // Populate the create form directly to keep environment access out of unit tests.
        let mut draft = Draft::new(String::new(), None);
        draft.fields[0] = "Project λ".into();
        draft.fields[1] = path.display().to_string();
        picker.creating = true;
        picker.draft = Some(Box::new(draft));
        let (_, board) = press(&mut picker, KeyCode::Enter).unwrap();
        assert_eq!(board.name, "Project λ");
        let before = std::fs::read(&path).unwrap();
        assert!(press(&mut picker, KeyCode::Enter).is_none());
        assert!(picker.error.contains("already exists"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        press(&mut picker, KeyCode::Esc);
        picker.entries.push(RecentBoard {
            name: board.name,
            path: path.clone(),
        });
        assert!(press(&mut picker, KeyCode::Enter).is_some());
        std::fs::remove_file(path).unwrap();
        assert!(press(&mut picker, KeyCode::Enter).is_none());
        assert!(picker.error.contains("Cannot read"));
        for (width, height) in [(110, 32), (35, 12), (15, 5)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| picker.draw(f)).unwrap();
            press(&mut picker, KeyCode::Char('o'));
            terminal.draw(|f| picker.draw(f)).unwrap();
            press(&mut picker, KeyCode::Esc);
        }
    }

    #[test]
    fn columns_can_be_created_renamed_reordered_and_stacked() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("board.json"));
        store.init(board()).unwrap();
        let mut app = App::new(store.read().unwrap());
        let press = |app: &mut App, code| {
            app.key(KeyEvent::new(code, KeyModifiers::NONE), &store)
                .unwrap();
        };
        press(&mut app, KeyCode::Char('N'));
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Column { .. }));
        for c in "Review λ".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.board.columns[app.column], "Review λ");
        press(&mut app, KeyCode::Char('E'));
        press(&mut app, KeyCode::End);
        press(&mut app, KeyCode::Char('!'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('['));
        press(&mut app, KeyCode::Char('s'));
        let saved = store.read().unwrap();
        assert_eq!(saved.columns, ["Todo", "Doing", "Review λ!", "Done"]);
        assert_eq!(saved.stacked_columns, ["Review λ!"]);
        press(&mut app, KeyCode::Char('s'));
        assert!(store.read().unwrap().stacked_columns.is_empty());
        press(&mut app, KeyCode::Char('N'));
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(store.read().unwrap().columns.len(), 4);
    }
    #[test]
    fn stacked_columns_render_above_each_other_and_selection_stays_visible() {
        let mut b = board();
        b.stack_column("Doing", true).unwrap();
        let cells = column_cells(&b, 1, Rect::new(0, 3, 100, 24));
        assert_eq!(cells[0].1.x, cells[1].1.x);
        assert!(cells[0].1.y < cells[1].1.y);
        assert!(cells[2].1.x > cells[1].1.x);
        b.stack_column("Done", true).unwrap();
        for selected in 0..3 {
            let cells = column_cells(&b, selected, Rect::new(0, 3, 30, 6));
            assert_eq!(cells.len(), 1);
            assert_eq!(cells[0].0, selected);
        }
        let app = App::new(b);
        let mut terminal = Terminal::new(TestBackend::new(100, 35)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let rows: Vec<String> = terminal
            .backend()
            .buffer()
            .content
            .chunks(100)
            .map(|row| row.iter().map(|c| c.symbol()).collect())
            .collect();
        let positions: Vec<_> = ["Todo", "Doing", "Done"]
            .iter()
            .map(|name| rows.iter().position(|row| row.contains(name)).unwrap())
            .collect();
        assert!(positions[0] < positions[1] && positions[1] < positions[2]);
    }
    #[test]
    fn editor_cursor_handles_unicode_and_deletion() {
        let mut draft = Draft::new("Todo".into(), None);
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        for c in "aλ界".chars() {
            draft.input(key(KeyCode::Char(c)));
        }
        draft.input(key(KeyCode::Left));
        draft.input(key(KeyCode::Backspace));
        draft.input(key(KeyCode::Char('β')));
        assert_eq!(draft.fields[0], "aβ界");
        draft.input(key(KeyCode::Home));
        draft.input(key(KeyCode::Delete));
        assert_eq!(draft.fields[0], "β界");
        assert_eq!(draft.displayed(0), "▏β界");
        draft.input(key(KeyCode::End));
        draft.input(key(KeyCode::Char('!')));
        assert_eq!(draft.displayed(0), "β界!▏");
    }
    #[test]
    fn renders_empty_board_and_small_sizes() {
        for (width, height) in [(100, 30), (35, 12), (10, 5)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let app = App::new(board());
            terminal.draw(|f| app.draw(f)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains(if width < 30 { "Terminal" } else { "KANBAN" }));
        }
    }
    #[test]
    fn keyboard_create_move_archive_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("board.json"));
        store.init(board()).unwrap();
        let mut app = App::new(store.read().unwrap());
        let press = |app: &mut App, code| {
            app.key(KeyEvent::new(code, KeyModifiers::NONE), &store)
                .unwrap()
        };
        press(&mut app, KeyCode::Char('n'));
        for c in "Ship λ".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        app.key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &store,
        )
        .unwrap();
        assert_eq!(store.read().unwrap().cards[0].title, "Ship λ");
        press(&mut app, KeyCode::Char('L'));
        assert_eq!(store.read().unwrap().cards[0].column, "Doing");
        press(&mut app, KeyCode::Char('a'));
        assert!(store.read().unwrap().cards[0].archived);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('λ'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected().unwrap().title, "Ship λ");
    }
    #[test]
    fn editor_does_not_overwrite_concurrent_edit() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("board.json"));
        let mut b = board();
        b.add(
            "Original".into(),
            "Todo",
            String::new(),
            Priority::Low,
            vec![],
            None,
        )
        .unwrap();
        store.init(b).unwrap();
        let draft = Draft::new("Todo".into(), Some(store.read().unwrap().cards[0].clone()));
        store
            .update(|b| {
                b.card_mut(1)?.updated_at = "2025-01-01T00:00:00Z".into();
                Ok(())
            })
            .unwrap();
        assert!(draft.save(&store).is_err());
    }
    #[test]
    fn renders_cards_and_every_dialog() {
        let mut b = board();
        b.add(
            "Fix launch".into(),
            "Todo",
            "Details".into(),
            Priority::Urgent,
            vec!["release".into()],
            None,
        )
        .unwrap();
        let mut app = App::new(b);
        let mut terminal = Terminal::new(TestBackend::new(100, 35)).unwrap();
        for mode in [
            Mode::Normal,
            Mode::Column {
                original: None,
                draft: Box::new(Draft::new(String::new(), None)),
            },
            Mode::Help(0),
            Mode::Detail(0),
            Mode::Delete(1),
            Mode::Edit(Box::new(Draft::new("Todo".into(), app.selected().cloned()))),
        ] {
            app.mode = mode;
            terminal.draw(|f| app.draw(f)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains("KANBAN"));
        }
    }
}
