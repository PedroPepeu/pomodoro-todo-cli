use std::{
    fs,
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
    DefaultTerminal, Frame,
};

const WORK_SECONDS: u64 = 25 * 60;
const BREAK_SECONDS: u64 = 5 * 60;
const TICK_RATE: Duration = Duration::from_millis(200);
const EXPORT_PATH: &str = "pomodoro-todo-export.txt";

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let terminal = ratatui::init();

    let result = App::default().run(terminal);

    ratatui::restore();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

#[derive(Clone)]
struct Todo {
    text: String,
    done: bool,
}

struct TaskFolder {
    name: String,
    todos: Vec<Todo>,
    completed: Vec<Todo>,
    selected_row: usize,
    completed_collapsed: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SelectedTaskRow {
    Active(usize),
    CompletedHeader,
    Completed(usize),
}

impl TaskFolder {
    fn visible_task_rows(&self) -> usize {
        let completed_rows = if self.completed_collapsed {
            0
        } else {
            self.completed.len()
        };
        self.todos.len() + 1 + completed_rows
    }

    fn selected_task_row(&self) -> Option<SelectedTaskRow> {
        if self.visible_task_rows() == 0 {
            return None;
        }

        if self.selected_row < self.todos.len() {
            return Some(SelectedTaskRow::Active(self.selected_row));
        }

        if self.selected_row == self.todos.len() {
            return Some(SelectedTaskRow::CompletedHeader);
        }

        if self.completed_collapsed {
            return Some(SelectedTaskRow::CompletedHeader);
        }

        let completed_index = self.selected_row - self.todos.len() - 1;
        if completed_index < self.completed.len() {
            Some(SelectedTaskRow::Completed(completed_index))
        } else {
            None
        }
    }

    fn clamp_selection(&mut self) {
        self.selected_row = self
            .selected_row
            .min(self.visible_task_rows().saturating_sub(1));
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Focus {
    Todos,
    Timer,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum InputMode {
    Todo,
    Folder,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PomodoroMode {
    Work,
    Break,
}

struct App {
    folders: Vec<TaskFolder>,
    selected_folder: usize,
    input: String,
    input_mode: InputMode,
    focus: Focus,
    mode: PomodoroMode,
    remaining: Duration,
    running: bool,
    last_tick: Instant,
    status: String,
}

impl Default for App {
    fn default() -> Self {
        Self {
            folders: vec![TaskFolder {
                name: "Inbox".to_string(),
                todos: vec![
                    Todo {
                        text: "Add your first task".to_string(),
                        done: false,
                    },
                    Todo {
                        text: "Create a task folder with f".to_string(),
                        done: false,
                    },
                ],
                completed: Vec::new(),
                selected_row: 0,
                completed_collapsed: false,
            }],
            selected_folder: 0,
            input: String::new(),
            input_mode: InputMode::Todo,
            focus: Focus::Todos,
            mode: PomodoroMode::Work,
            remaining: Duration::from_secs(WORK_SECONDS),
            running: false,
            last_tick: Instant::now(),
            status: format!("Export/import: e exports, i imports ({EXPORT_PATH})"),
        }
    }
}

impl App {
    fn run(mut self, mut terminal: DefaultTerminal) -> io::Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;

            let timeout = TICK_RATE
                .checked_sub(self.last_tick.elapsed())
                .unwrap_or(Duration::ZERO);

            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if self.handle_key(key) {
                        break;
                    }
                }
            }

            self.update_timer();
        }

        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }

        match key.code {
            KeyCode::Char('q') if self.input.is_empty() => true,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Todos => Focus::Timer,
                    Focus::Timer => Focus::Todos,
                };
                false
            }
            _ => match self.focus {
                Focus::Todos => self.handle_todo_key(key),
                Focus::Timer => self.handle_timer_key(key),
            },
        }
    }

    fn handle_todo_key(&mut self, key: KeyEvent) -> bool {
        if self.input_mode == InputMode::Folder {
            self.handle_folder_input_key(key);
            return false;
        }

        match key.code {
            KeyCode::Char('f') if self.input.is_empty() => self.start_folder_input(),
            KeyCode::Char('e') if self.input.is_empty() => self.export_tasks(),
            KeyCode::Char('i') if self.input.is_empty() => self.import_tasks(),
            KeyCode::Char(' ') if self.input.is_empty() => self.toggle_selected_task_done(),
            KeyCode::Char('c') if self.input.is_empty() => self.toggle_completed_collapse(),
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Enter => self.add_todo_from_input(),
            KeyCode::Up => self.select_previous_todo(),
            KeyCode::Down => self.select_next_todo(),
            KeyCode::Left => self.select_previous_folder(),
            KeyCode::Right => self.select_next_folder(),
            KeyCode::Delete => self.delete_selected_todo(),
            KeyCode::Esc => self.input.clear(),
            _ => {}
        }
        false
    }

    fn handle_folder_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Enter => self.add_folder_from_input(),
            KeyCode::Esc => {
                self.input.clear();
                self.input_mode = InputMode::Todo;
            }
            _ => {}
        }
    }

    fn handle_timer_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(' ') | KeyCode::Enter => self.running = !self.running,
            KeyCode::Char('r') => self.reset_timer(),
            KeyCode::Char('s') => self.switch_mode(),
            _ => {}
        }
        false
    }

    fn update_timer(&mut self) {
        if !self.running {
            self.last_tick = Instant::now();
            return;
        }

        let elapsed = self.last_tick.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }

        let elapsed_secs = elapsed.as_secs();
        self.last_tick += Duration::from_secs(elapsed_secs);

        if self.remaining.as_secs() > elapsed_secs {
            self.remaining -= Duration::from_secs(elapsed_secs);
        } else {
            self.switch_mode();
            self.running = false;
        }
    }

    fn current_folder(&self) -> &TaskFolder {
        &self.folders[self.selected_folder]
    }

    fn current_folder_mut(&mut self) -> &mut TaskFolder {
        &mut self.folders[self.selected_folder]
    }

    fn start_folder_input(&mut self) {
        self.input.clear();
        self.input_mode = InputMode::Folder;
    }

    fn add_folder_from_input(&mut self) {
        let name = self.input.trim();
        if !name.is_empty() {
            self.folders.push(TaskFolder {
                name: name.to_string(),
                todos: Vec::new(),
                completed: Vec::new(),
                selected_row: 0,
                completed_collapsed: false,
            });
            self.selected_folder = self.folders.len() - 1;
        }
        self.input.clear();
        self.input_mode = InputMode::Todo;
    }

    fn add_todo_from_input(&mut self) {
        let text = self.input.trim().to_string();
        if !text.is_empty() {
            let folder = self.current_folder_mut();
            folder.todos.push(Todo { text, done: false });
            folder.selected_row = folder.todos.len().saturating_sub(1);
            self.input.clear();
        }
    }

    fn select_previous_folder(&mut self) {
        if self.folders.is_empty() || !self.input.is_empty() {
            return;
        }

        if self.selected_folder == 0 {
            self.selected_folder = self.folders.len() - 1;
        } else {
            self.selected_folder -= 1;
        }
    }

    fn select_next_folder(&mut self) {
        if self.folders.is_empty() || !self.input.is_empty() {
            return;
        }

        self.selected_folder = (self.selected_folder + 1) % self.folders.len();
    }

    fn select_previous_todo(&mut self) {
        let folder = self.current_folder_mut();
        let row_count = folder.visible_task_rows();
        if row_count == 0 {
            folder.selected_row = 0;
        } else if folder.selected_row == 0 {
            folder.selected_row = row_count - 1;
        } else {
            folder.selected_row -= 1;
        }
        folder.clamp_selection();
    }

    fn select_next_todo(&mut self) {
        let folder = self.current_folder_mut();
        let row_count = folder.visible_task_rows();
        if row_count == 0 {
            folder.selected_row = 0;
        } else {
            folder.selected_row = (folder.selected_row + 1) % row_count;
        }
        folder.clamp_selection();
    }

    fn toggle_selected_task_done(&mut self) {
        let folder = self.current_folder_mut();
        match folder.selected_task_row() {
            Some(SelectedTaskRow::Active(index)) => {
                let mut todo = folder.todos.remove(index);
                todo.done = true;
                folder.completed.push(todo);
            }
            Some(SelectedTaskRow::Completed(index)) => {
                let mut todo = folder.completed.remove(index);
                todo.done = false;
                folder.todos.push(todo);
                folder.selected_row = folder.todos.len().saturating_sub(1);
            }
            Some(SelectedTaskRow::CompletedHeader) | None => {
                folder.completed_collapsed = !folder.completed_collapsed;
            }
        }
        folder.clamp_selection();
    }

    fn toggle_completed_collapse(&mut self) {
        let folder = self.current_folder_mut();
        folder.completed_collapsed = !folder.completed_collapsed;
        folder.clamp_selection();
    }

    fn delete_selected_todo(&mut self) {
        let folder = self.current_folder_mut();
        match folder.selected_task_row() {
            Some(SelectedTaskRow::Active(index)) => {
                folder.todos.remove(index);
            }
            Some(SelectedTaskRow::Completed(index)) => {
                folder.completed.remove(index);
            }
            Some(SelectedTaskRow::CompletedHeader) | None => {}
        }
        folder.clamp_selection();
    }

    fn export_tasks(&mut self) {
        match fs::write(EXPORT_PATH, self.to_export_text()) {
            Ok(()) => self.status = format!("Exported to {EXPORT_PATH}"),
            Err(error) => self.status = format!("Export failed: {error}"),
        }
    }

    fn import_tasks(&mut self) {
        match fs::read_to_string(EXPORT_PATH) {
            Ok(text) => match folders_from_export_text(&text) {
                Some(folders) => {
                    self.folders = folders;
                    self.selected_folder = 0;
                    self.status = format!("Imported from {EXPORT_PATH}");
                }
                None => self.status = "Import failed: invalid export text".to_string(),
            },
            Err(error) => self.status = format!("Import failed: {error}"),
        }
    }

    fn to_export_text(&self) -> String {
        let mut text = "# Pomodoro Todo Export v1\n\n".to_string();
        for folder in &self.folders {
            text.push_str(&format!("## Folder: {}\n", folder.name));
            for todo in &folder.todos {
                text.push_str(&format!("- [ ] {}\n", todo.text));
            }
            for todo in &folder.completed {
                text.push_str(&format!("- [x] {}\n", todo.text));
            }
            text.push('\n');
        }
        text
    }

    fn switch_mode(&mut self) {
        self.mode = match self.mode {
            PomodoroMode::Work => PomodoroMode::Break,
            PomodoroMode::Break => PomodoroMode::Work,
        };
        self.reset_timer();
    }

    fn reset_timer(&mut self) {
        self.running = false;
        self.remaining = Duration::from_secs(match self.mode {
            PomodoroMode::Work => WORK_SECONDS,
            PomodoroMode::Break => BREAK_SECONDS,
        });
        self.last_tick = Instant::now();
    }

    fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(12),
                Constraint::Length(3),
            ])
            .split(area);

        self.render_header(frame, chunks[0]);

        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(chunks[1]);

        self.render_todos(frame, main[0]);
        self.render_timer(frame, main[1]);
        self.render_help(frame, chunks[2]);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let title = Line::from(vec![
            Span::styled(
                "Pomodoro Todo",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::raw(" switches focus, "),
            Span::styled("f", Style::default().fg(Color::Cyan)),
            Span::raw(" creates folder, "),
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(" quits"),
        ]);
        frame.render_widget(
            Paragraph::new(title).block(Block::default().borders(Borders::ALL)),
            area,
        );
    }

    fn render_todos(&self, frame: &mut Frame, area: Rect) {
        let border_style = focused_style(self.focus == Focus::Todos);
        let block = Block::default()
            .title(" Todos ")
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(3)])
            .split(inner);

        let todo_body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Min(20)])
            .split(chunks[0]);

        self.render_folders(frame, todo_body[0]);
        self.render_current_folder_tasks(frame, todo_body[1]);
        self.render_input(frame, chunks[1]);
    }

    fn render_folders(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .folders
            .iter()
            .enumerate()
            .map(|(index, folder)| {
                let open = folder.todos.len();
                let done = folder.completed.len();
                let mut style = Style::default().fg(Color::White);
                if index == self.selected_folder {
                    style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
                }
                ListItem::new(Line::from(format!("{} ({}/{})", folder.name, open, done)))
                    .style(style)
            })
            .collect();

        frame.render_widget(
            List::new(items).block(Block::default().title(" Folders ").borders(Borders::ALL)),
            area,
        );
    }

    fn render_current_folder_tasks(&self, frame: &mut Frame, area: Rect) {
        let folder = self.current_folder();
        let mut items: Vec<ListItem> = folder
            .todos
            .iter()
            .enumerate()
            .map(|(index, todo)| {
                let mut style = Style::default().fg(Color::White);
                if folder.selected_task_row() == Some(SelectedTaskRow::Active(index)) {
                    style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
                }
                ListItem::new(Line::from(format!("[ ] {}", todo.text))).style(style)
            })
            .collect();

        let completed_icon = if folder.completed_collapsed {
            "[+]"
        } else {
            "[-]"
        };
        let mut completed_header_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        if folder.selected_task_row() == Some(SelectedTaskRow::CompletedHeader) {
            completed_header_style = completed_header_style.bg(Color::DarkGray);
        }
        items.push(
            ListItem::new(Line::from(format!(
                "{} Completed tasks ({})",
                completed_icon,
                folder.completed.len()
            )))
            .style(completed_header_style),
        );

        if !folder.completed_collapsed {
            items.extend(folder.completed.iter().enumerate().map(|(index, todo)| {
                let mut style = Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::CROSSED_OUT);
                if folder.selected_task_row() == Some(SelectedTaskRow::Completed(index)) {
                    style = style.bg(Color::DarkGray).fg(Color::White);
                }
                ListItem::new(Line::from(format!("[x] {}", todo.text))).style(style)
            }));
        }

        let list = if folder.todos.is_empty() && folder.completed.is_empty() {
            List::new(vec![ListItem::new(
                "No tasks in this folder. Type below and press Enter.",
            )
            .gray()])
        } else {
            List::new(items)
        };

        frame.render_widget(
            list.block(Block::default().title(" Tasks ").borders(Borders::ALL)),
            area,
        );
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let title = match self.input_mode {
            InputMode::Todo => format!(" New task in {} ", self.current_folder().name),
            InputMode::Folder => " New folder ".to_string(),
        };
        let input = Paragraph::new(self.input.as_str())
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        frame.render_widget(input, area);

        if self.focus == Focus::Todos {
            let cursor_x = area.x + self.input.len() as u16 + 1;
            let cursor_y = area.y + 1;
            if cursor_x < area.right() {
                frame.set_cursor_position((cursor_x, cursor_y));
            }
        }
    }

    fn render_timer(&self, frame: &mut Frame, area: Rect) {
        let border_style = focused_style(self.focus == Focus::Timer);
        let block = Block::default()
            .title(" Pomodoro ")
            .borders(Borders::ALL)
            .border_style(border_style);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(2),
            ])
            .split(inner);

        let total = match self.mode {
            PomodoroMode::Work => WORK_SECONDS,
            PomodoroMode::Break => BREAK_SECONDS,
        };
        let remaining = self.remaining.as_secs();
        let elapsed = total.saturating_sub(remaining);
        let ratio = if total == 0 {
            0.0
        } else {
            elapsed as f64 / total as f64
        };

        let mode = match self.mode {
            PomodoroMode::Work => "Work",
            PomodoroMode::Break => "Break",
        };
        let state = if self.running { "Running" } else { "Paused" };
        let timer = format!(
            "{}  |  {}  |  {}",
            mode,
            state,
            format_duration(self.remaining)
        );

        frame.render_widget(Paragraph::new(timer).centered().bold(), chunks[0]);
        frame.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(if self.mode == PomodoroMode::Work {
                    Color::Red
                } else {
                    Color::Green
                }))
                .ratio(ratio),
            chunks[1],
        );
        frame.render_widget(
            Paragraph::new("Space/Enter start-pause | r reset | s switch work/break")
                .centered()
                .fg(Color::Gray),
            chunks[2],
        );

        let (open, done) = self.task_counts();
        let stats = format!(
            "Folders: {}   Open: {}   Done: {}",
            self.folders.len(),
            open,
            done
        );
        frame.render_widget(Paragraph::new(stats).centered(), chunks[3]);
    }

    fn render_help(&self, frame: &mut Frame, area: Rect) {
        let help = match (self.focus, self.input_mode) {
            (Focus::Todos, InputMode::Todo) => "Todos: type task | Enter add | f folder | e export | i import | Arrows nav | Space complete | c collapse | Delete remove",
            (Focus::Todos, InputMode::Folder) => "Folder: type folder name | Enter create | Esc cancel",
            (Focus::Timer, _) => "Timer: Space/Enter start-pause | r reset | s switch mode | Tab todos | q quit",
        };
        let help = format!("{help} | {}", self.status);
        frame.render_widget(
            Paragraph::new(help)
                .block(Block::default().borders(Borders::ALL))
                .fg(Color::Gray),
            area,
        );
    }

    fn task_counts(&self) -> (usize, usize) {
        let open: usize = self.folders.iter().map(|folder| folder.todos.len()).sum();
        let done: usize = self
            .folders
            .iter()
            .map(|folder| folder.completed.len())
            .sum();
        (open, done)
    }
}

fn folders_from_export_text(text: &str) -> Option<Vec<TaskFolder>> {
    let mut folders = Vec::new();
    let mut current: Option<TaskFolder> = None;

    for line in text.lines() {
        if let Some(name) = line.strip_prefix("## Folder: ") {
            if let Some(folder) = current.take() {
                folders.push(folder);
            }
            current = Some(TaskFolder {
                name: name.trim().to_string(),
                todos: Vec::new(),
                completed: Vec::new(),
                selected_row: 0,
                completed_collapsed: false,
            });
        } else if let Some(task) = line.strip_prefix("- [ ] ") {
            current.as_mut()?.todos.push(Todo {
                text: task.to_string(),
                done: false,
            });
        } else if let Some(task) = line.strip_prefix("- [x] ") {
            current.as_mut()?.completed.push(Todo {
                text: task.to_string(),
                done: true,
            });
        }
    }

    if let Some(folder) = current {
        folders.push(folder);
    }

    if folders.is_empty() {
        None
    } else {
        Some(folders)
    }
}

fn focused_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}
