use crate::{
    controller::{self, Command, Message},
    engine::Engine,
    model::*,
    native, store,
};
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
};
use rat_widget::{
    scrolled::{Scroll, ScrollbarPolicy},
    table::{
        Table, TableState,
        selection::{RowSelection, rowselection},
        textdata::{Cell, Row},
    },
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

const THEME_KEY: &str = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
#[cfg(test)]
use crossterm::event::KeyModifiers;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Palette {
    base: Color,
    surface: Color,
    border: Color,
    text: Color,
    muted: Color,
    blue: Color,
    red: Color,
}
impl Palette {
    const LATTE: Self = Self {
        base: Color::Rgb(239, 241, 245),
        surface: Color::Rgb(204, 208, 218),
        border: Color::Rgb(172, 176, 190),
        text: Color::Rgb(76, 79, 105),
        muted: Color::Rgb(92, 95, 119),
        blue: Color::Rgb(30, 102, 245),
        red: Color::Rgb(210, 15, 57),
    };
    const MOCHA: Self = Self {
        base: Color::Rgb(30, 30, 46),
        surface: Color::Rgb(49, 50, 68),
        border: Color::Rgb(88, 91, 112),
        text: Color::Rgb(205, 214, 244),
        muted: Color::Rgb(186, 194, 222),
        blue: Color::Rgb(137, 180, 250),
        red: Color::Rgb(243, 139, 168),
    };
    fn system() -> Result<Self> {
        Ok(
            if crate::registry::number(THEME_KEY, "AppsUseLightTheme", 1)? == 0 {
                Self::MOCHA
            } else {
                Self::LATTE
            },
        )
    }
    fn status(self, state: Status) -> Color {
        match state {
            Status::Failed => self.red,
            Status::Ready
            | Status::Running
            | Status::SafeQueued
            | Status::Restart
            | Status::SignIn => self.blue,
            _ => self.muted,
        }
    }
}
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub struct App {
    pub catalog: Vec<Feature>,
    pub states: Vec<Check>,
    pub visible: Vec<usize>,
    pub table: TableState<RowSelection>,
    pub zh: bool,
    pub busy: bool,
    pub message: String,
    pub engine: Option<Arc<Engine>>,
    pub countdown: Option<(Instant, bool)>,
    pub restart: bool,
    follow_top: bool,
    palette: Palette,
}
impl App {
    pub fn new(catalog: Vec<Feature>, zh: bool) -> Result<Self> {
        let mut app = Self {
            states: vec![Check::new(Status::PendingCheck); catalog.len()],
            catalog,
            visible: vec![],
            table: TableState::default(),
            zh,
            busy: false,
            message: String::new(),
            engine: None,
            countdown: None,
            restart: false,
            follow_top: true,
            palette: Palette::system()?,
        };
        app.table.selection.set_scroll_selected(true);
        app.reorder();
        app.table.select(Some(0));
        Ok(app)
    }
    pub fn selected(&self) -> Option<usize> {
        self.table
            .selected()
            .and_then(|r| self.visible.get(r).copied())
    }
    pub fn reorder(&mut self) {
        let selected = self.selected();
        let row = self.table.selected();
        self.visible = controller::order(&self.catalog)
            .into_iter()
            .filter(|i| self.states[*i].visible(&self.catalog[*i]))
            .collect();
        if self.follow_top && !self.busy {
            self.table.select((!self.visible.is_empty()).then_some(0));
            self.table.set_row_offset(0);
        } else {
            if let Some(i) = selected {
                self.table.select(self.visible.iter().position(|v| *v == i));
            }
            if self.table.selected().is_none() && !self.visible.is_empty() {
                self.table.select(Some(0));
            }
        }
        if row != self.table.selected() {
            self.table.scroll_to_selected();
        }
    }
    #[cfg(test)]
    pub fn focus(&mut self, i: usize) {
        if let Some(row) = self.visible.iter().position(|v| *v == i) {
            self.follow_top = false;
            self.table.select(Some(row));
            self.table.scroll_to_selected();
        }
    }
    pub fn command(&mut self, command: &Command) {
        self.follow_top = false;
        self.countdown = None;
        if let Command::Scan(indices) = command {
            for &i in indices {
                self.states[i] = Check::new(Status::PendingCheck);
            }
            return;
        }
        self.busy = true;
        for i in 0..self.catalog.len() {
            let chosen = match command {
                Command::All => true,
                Command::One(n) => i == *n,
                _ => false,
            };
            if chosen && self.states[i].actionable() {
                self.states[i] = Check::new(Status::Queued)
            }
        }
        self.message = choose(self.zh, "正在执行", "Executing").into();
        self.reorder();
    }
    fn space_command(&self) -> Option<Command> {
        let i = self.selected()?;
        if self.states[i].recheckable() {
            Some(Command::Scan(vec![i]))
        } else if self.states[i].actionable() || self.catalog[i].toggle() {
            Some(Command::One(i))
        } else {
            None
        }
    }
    fn navigate(&mut self, event: &Event) -> bool {
        let before = self.table.selected();
        let outcome = rowselection::handle_events(&mut self.table, true, event);
        let changed =
            before != self.table.selected() || outcome != rat_widget::event::TableOutcome::Continue;
        if changed {
            self.follow_top = false;
        }
        changed
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        let p = self.palette;
        let style = Style::default().fg(p.text).bg(p.base);
        frame.render_widget(Block::default().style(style), frame.area());
        let area = Rect {
            x: frame.area().x + 1,
            width: frame.area().width.saturating_sub(2),
            ..frame.area()
        };
        let mut spans = vec![];
        for (key, zh, en) in [
            if self
                .selected()
                .is_some_and(|i| self.states[i].recheckable())
            {
                ("Space", "重新检测当前项", "Check item again")
            } else {
                ("Space", "执行当前项", "Run item")
            },
            ("A", "执行全部", "Run all"),
            ("R", "重启", "Restart"),
            ("Q", "退出", "Exit"),
        ] {
            if key == "R" && !self.restart {
                continue;
            }
            if !spans.is_empty() {
                spans.push(Span::styled("   ", Style::default().fg(p.muted)));
            }
            spans.push(Span::styled(
                key,
                Style::default().fg(p.blue).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" {}", choose(self.zh, zh, en)),
                Style::default().fg(p.muted),
            ));
        }
        let keys = Line::from(spans);
        let mut message = self.message.clone();
        if self.busy {
            let running = self
                .states
                .iter()
                .filter(|s| s.state == Status::Running)
                .count();
            let queued = self
                .states
                .iter()
                .filter(|s| s.state == Status::Queued)
                .count();
            message = format!(
                "{} {running} · {} {queued}",
                choose(self.zh, "执行中", "Running"),
                choose(self.zh, "等待执行", "Queued")
            );
        }
        if let Some((deadline, restart)) = self.countdown {
            let seconds = deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
            message = format!(
                "{message} · {seconds}{}",
                choose(
                    self.zh,
                    if restart {
                        " 秒后重启，任意键停止"
                    } else {
                        " 秒后关闭，任意键停止"
                    },
                    if restart {
                        "s to restart; any key stops"
                    } else {
                        "s to close; any key stops"
                    }
                )
            );
        }
        let mut lines = vec![keys];
        if !message.is_empty() {
            let color = if self.busy {
                p.blue
            } else if self.states.iter().any(|s| s.state == Status::Failed) {
                p.red
            } else if self.restart {
                p.blue
            } else {
                p.text
            };
            lines.extend(
                message
                    .lines()
                    .map(|s| Line::styled(s.to_owned(), Style::default().fg(color))),
            );
        }
        let header = Paragraph::new(lines).wrap(Wrap { trim: false });
        let top = header.line_count(area.width) as u16;
        let description = self.selected().map(|i| {
            let f = &self.catalog[i];
            let state = &self.states[i];
            let mut lines = vec![Line::from(f.purpose(self.zh))];
            if f.id == "taskbar-pins"
                && matches!(state.state, Status::Ready | Status::Done | Status::SignIn)
                && !state.detail.is_empty()
            {
                lines.push(Line::from(state.detail.as_str()));
                if state.state != Status::Done {
                    lines.push(Line::from(choose(
                        self.zh,
                        "固定项可能被旧设置隐藏；重新登录后刷新。当前打开的窗口也会显示在任务栏。",
                        "Old settings may hide saved pins until sign-in. Running windows also appear on the taskbar.",
                    )));
                }
            }
            if let Some(explanation) = state.explanation(self.zh) {
                lines.push(Line::from(explanation));
                if state.recheckable() {
                    lines.push(Line::from(format!(
                        "{}{}",
                        choose(self.zh, "诊断日志：", "Diagnostic log: "),
                        store::root().join("checks.jsonl").display(),
                    )));
                }
            }
            if state.state == Status::Failed && !state.detail.is_empty() {
                lines.push(Line::styled(
                    state.detail.as_str(),
                    Style::default().fg(p.red),
                ));
            }
            Paragraph::new(lines)
                .style(Style::default().fg(p.muted))
                .wrap(Wrap { trim: true })
                .block(
                    Block::default()
                        .borders(Borders::TOP)
                        .border_style(Style::default().fg(p.border)),
                )
        });
        let bottom = description
            .as_ref()
            .map(|text| (text.line_count(area.width) as u16).min(6))
            .unwrap_or(0);
        let layout = Layout::vertical([
            Constraint::Length(top),
            Constraint::Min(3),
            Constraint::Length(bottom),
        ])
        .split(area);
        frame.render_widget(header, layout[0]);
        let rows = self.visible.iter().map(|i| {
            let f = &self.catalog[*i];
            Row::new([
                Cell::from(f.category(self.zh).to_owned())
                    .style(Some(Style::default().fg(p.muted))),
                Cell::from(f.name(self.zh).to_owned()),
                Cell::from(self.states[*i].label(f, self.zh))
                    .style(Some(Style::default().fg(p.status(self.states[*i].state)))),
            ])
        });
        let category = if self.zh { 12 } else { 20 };
        let category = if area.width < 85 {
            category.min(12)
        } else {
            category
        };
        let status = if area.width < 85 { 16 } else { 24 };
        let table = Table::default()
            .style(style)
            .rows(rows)
            .header(
                Row::new([
                    choose(self.zh, "类别", "Category"),
                    choose(self.zh, "项目", "Item"),
                    choose(self.zh, "状态", "Status"),
                ])
                .style(Some(
                    Style::default()
                        .fg(p.muted)
                        .bg(p.base)
                        .add_modifier(Modifier::BOLD),
                )),
            )
            .widths([
                Constraint::Length(category),
                Constraint::Fill(1),
                Constraint::Length(status),
            ])
            .column_spacing(2)
            .select_row_style(Some(
                Style::default().bg(p.surface).add_modifier(Modifier::BOLD),
            ))
            .show_row_focus(false)
            .vscroll(
                Scroll::vertical()
                    .policy(ScrollbarPolicy::Collapse)
                    .style(style.fg(p.border))
                    .thumb_symbol(" ")
                    .thumb_style(style.bg(p.blue)),
            );
        frame.render_stateful_widget(table, layout[1], &mut self.table);
        if let Some(text) = description {
            frame.render_widget(text, layout[2]);
        }
    }
    fn receive(&mut self, msg: Message, epoch: usize) {
        match msg {
            Message::Ready(engine) => {
                self.restart = crate::workflow::requires_restart(&engine);
                if self.restart {
                    self.message = choose(self.zh, "待手动重启", "Pending manual restart").into()
                }
                self.engine = Some(engine)
            }
            Message::State(i, state, version) if version == epoch => {
                self.states[i] = state;
                self.reorder();
            }
            Message::State(..) => {}
            Message::Queued(indices) => {
                self.busy = true;
                self.countdown = None;
                for i in indices {
                    if self.states[i].actionable() {
                        self.states[i] = Check::new(Status::Queued);
                    }
                }
            }
            Message::Finished {
                processed,
                restart,
                auto,
                ..
            } => {
                self.busy = false;
                self.restart = restart;
                if let Some(engine) = &self.engine
                    && let Some(pending) = engine.store.pending.lock().unwrap().as_ref()
                {
                    for (i, f) in self.catalog.iter().enumerate() {
                        if pending.phase == crate::workflow::Phase::AwaitSafe
                            && pending.safe_ids.contains(&f.id)
                        {
                            self.states[i] = Check::new(Status::SafeQueued)
                        }
                    }
                }
                let failures = self
                    .engine
                    .as_ref()
                    .map(|e| {
                        let results = e.store.results.lock().unwrap();
                        e.catalog
                            .iter()
                            .filter(|f| results.get(&f.id).is_some_and(|r| !r.errors.is_empty()))
                            .count()
                    })
                    .unwrap_or(0);
                self.message = format!(
                    "{} {processed} {}",
                    choose(self.zh, "已处理", "Processed"),
                    choose(self.zh, "项", "items")
                );
                if restart && !auto {
                    self.message.push_str(choose(
                        self.zh,
                        " · 待手动重启",
                        " · Pending manual restart",
                    ))
                }
                if failures > 0 {
                    self.message.push_str(&format!(
                        " · {failures} {} {}",
                        choose(self.zh, "项未完成，日志：", "incomplete; log:"),
                        store::log_path().display()
                    ))
                }
                if auto && (failures == 0 || native::safe_mode()) {
                    self.countdown = Some((Instant::now() + Duration::from_secs(10), restart))
                }
                self.reorder();
            }
            Message::Error(error) => {
                self.busy = false;
                self.countdown = None;
                self.message = if let Some(e) = &self.engine {
                    controller::failure_message(e, &error)
                } else {
                    error
                };
                self.reorder()
            }
        }
    }
}
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}
pub fn run(mode: String) -> Result<()> {
    native::open_console()?;
    let _guard = TerminalGuard;
    let mut terminal = ratatui::try_init()?;
    native::hide_console_scrollbars();
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let mut app = App::new(catalog()?, native::chinese())?;
    let Color::Rgb(r, g, b) = app.palette.base else {
        unreachable!()
    };
    let background = native::ConsoleBackground::new(r, g, b)?;
    terminal.draw(|frame| app.draw(frame))?;
    let (tx, rx) = mpsc::channel();
    let (commands, receiver) = mpsc::channel();
    let epoch = Arc::new(AtomicUsize::new(0));
    controller::launch(tx, receiver, epoch.clone(), mode);
    event_loop(&mut terminal, &mut app, &rx, &commands, &epoch, &background)
}
fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rx: &mpsc::Receiver<Message>,
    commands: &mpsc::Sender<Command>,
    epoch: &AtomicUsize,
    background: &native::ConsoleBackground,
) -> Result<()> {
    let theme = crate::registry::Watch::new(THEME_KEY)?;
    let mut dirty = false;
    let mut last_second = None;
    loop {
        if theme.changed()? {
            let palette = Palette::system()?;
            if palette != app.palette {
                app.palette = palette;
                let Color::Rgb(r, g, b) = palette.base else {
                    unreachable!()
                };
                background.set(r, g, b)?;
                dirty = true;
            }
        }
        for msg in rx.try_iter() {
            let finished = matches!(&msg, Message::Finished { .. } | Message::Error(_));
            let mut refresh = if let Message::Finished { refresh, .. } = &msg {
                refresh.clone()
            } else {
                vec![]
            };
            app.receive(msg, epoch.load(Ordering::SeqCst));
            if finished && app.engine.is_some() {
                refresh.extend(
                    app.states
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| {
                            matches!(
                                s.state,
                                Status::PendingCheck
                                    | Status::Checking
                                    | Status::Queued
                                    | Status::Running
                            )
                        })
                        .map(|(i, _)| i)
                        .collect::<Vec<_>>(),
                );
                refresh.sort();
                refresh.dedup();
                commands.send(Command::Scan(refresh))?;
            }
            dirty = true
        }
        if let Some((deadline, restart)) = app.countdown {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                if restart {
                    commands.send(Command::Restart)?;
                    app.countdown = None;
                    app.message = choose(app.zh, "正在重启", "Restarting").into()
                } else {
                    return Ok(());
                }
            }
            if last_second != Some(remaining.as_secs()) {
                last_second = Some(remaining.as_secs());
                dirty = true
            }
        }
        if dirty {
            terminal.draw(|frame| app.draw(frame))?;
            dirty = false
        }
        if !event::poll(Duration::from_millis(20))? {
            continue;
        }
        let event = event::read()?;
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if app.countdown.take().is_some() {
                app.message.push_str(choose(
                    app.zh,
                    if app.restart {
                        " · 待手动重启"
                    } else {
                        " · 已停止自动关闭"
                    },
                    if app.restart {
                        " · Pending manual restart"
                    } else {
                        " · Automatic close stopped"
                    },
                ));
                dirty = true;
                continue;
            }
            let command = match key.code {
                KeyCode::Char('q' | 'Q') | KeyCode::Esc if !app.busy => return Ok(()),
                KeyCode::Char('r' | 'R') if !app.busy && app.restart => Some(Command::Restart),
                KeyCode::Char('a' | 'A') if !app.busy => Some(Command::All),
                KeyCode::Char(' ') if !app.busy => app.space_command(),
                _ => None,
            };
            if let Some(command) = command {
                app.command(&command);
                commands.send(command)?;
                dirty = true;
                continue;
            }
        }
        if matches!(event, Event::Resize(..)) {
            native::hide_console_scrollbars();
            dirty = true
        }
        if app.navigate(&event) {
            dirty = true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn taskbar_states_show_saved_pin_details_in_the_rendered_view() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        for zh in [true, false] {
            let mut app = App::new(catalog()?, zh)?;
            let i = app
                .catalog
                .iter()
                .position(|f| f.id == "taskbar-pins")
                .unwrap();
            app.focus(i);
            let mut terminal = Terminal::new(TestBackend::new(120, 30))?;
            for (state, label) in [
                (Status::Ready, choose(zh, "可清理", "Can clear")),
                (Status::Done, choose(zh, "无固定项", "No pins")),
                (
                    Status::SignIn,
                    choose(zh, "重新登录生效", "Sign in to apply"),
                ),
            ] {
                let detail = if state == Status::Done {
                    "Windows has no saved taskbar pins."
                } else {
                    "Windows saved 3 taskbar pins: File Explorer, Microsoft Edge, Microsoft Store"
                };
                app.states[i] = Check {
                    state,
                    detail: detail.into(),
                };
                assert_eq!(app.states[i].label(&app.catalog[i], zh), label);
                terminal.draw(|frame| app.draw(frame))?;
                let buffer = terminal.backend().buffer();
                let description: String = (app.table.table_area.bottom() + 1..30)
                    .flat_map(|y| (0..120).map(move |x| buffer[(x, y)].symbol()))
                    .collect();
                assert!(description.contains(detail));
            }
        }
        Ok(())
    }
    #[test]
    fn unavailable_states_recheck_without_executing_or_moving_selection() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        let i = app
            .catalog
            .iter()
            .position(|f| f.id == "phishing-protection")
            .unwrap();
        app.focus(i);
        for state in [Status::Unknown, Status::Restricted] {
            app.states[i] = Check::new(state);
            let command = app.space_command().unwrap();
            assert!(matches!(&command, Command::Scan(indices) if indices == &[i]));
            app.command(&command);
            assert!(!app.busy);
            assert_eq!(app.selected(), Some(i));
            assert_eq!(app.states[i].state, Status::PendingCheck);
            app.receive(Message::State(i, Check::new(Status::Ready), 0), 0);
            assert!(matches!(app.space_command(), Some(Command::One(n)) if n == i));
        }
        app.states[i] = Check::new(Status::Failed);
        assert!(matches!(app.space_command(), Some(Command::One(n)) if n == i));
        for state in [Status::Absent, Status::Inactive, Status::Deferred] {
            app.states[i] = Check::new(state);
            assert!(app.space_command().is_none());
        }
        Ok(())
    }

    #[test]
    fn check_explanations_use_neutral_text_and_execution_errors_keep_red() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        for zh in [true, false] {
            for palette in [Palette::LATTE, Palette::MOCHA] {
                let mut app = App::new(catalog()?, zh)?;
                app.palette = palette;
                let i = app.catalog.iter().position(|f| f.id == "tamper").unwrap();
                app.focus(i);
                let mut terminal = Terminal::new(TestBackend::new(90, 24))?;
                for state in [
                    Status::Unknown,
                    Status::Restricted,
                    Status::Inactive,
                    Status::Absent,
                    Status::Deferred,
                ] {
                    app.states[i] = Check {
                        state,
                        detail: "internal diagnostic information".into(),
                    };
                    terminal.draw(|frame| app.draw(frame))?;
                    let buffer = terminal.backend().buffer();
                    let bottom = app.table.table_area.bottom() + 1;
                    assert!(bottom < 23);
                    for y in bottom..24 {
                        for x in 1..89 {
                            assert_ne!(buffer[(x, y)].fg, palette.red);
                        }
                    }
                }
                app.states[i] = Check {
                    state: Status::Failed,
                    detail: "operation failed".into(),
                };
                terminal.draw(|frame| app.draw(frame))?;
                let buffer = terminal.backend().buffer();
                assert!((1..89).any(|x| buffer[(x, 23)].fg == palette.red));
            }
        }
        Ok(())
    }
    #[test]
    fn changing_the_palette_keeps_selection_and_scroll_position() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| app.draw(frame))?;
        app.focus(app.catalog.len() - 1);
        terminal.draw(|frame| app.draw(frame))?;
        let selected = app.selected();
        let offset = app.table.row_offset();
        assert!(offset > 0);
        for palette in [Palette::MOCHA, Palette::LATTE] {
            app.palette = palette;
            terminal.draw(|frame| app.draw(frame))?;
            assert_eq!(terminal.backend().buffer()[(0, 0)].bg, palette.base);
            assert_eq!(app.selected(), selected);
            assert_eq!(app.table.row_offset(), offset);
        }
        Ok(())
    }
    #[test]
    fn startup_checks_keep_the_first_row_visible_until_navigation() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        for i in 0..app.catalog.len() {
            let state = if i == 1 { Status::Ready } else { Status::Done };
            app.receive(Message::State(i, Check::new(state), 0), 0);
            terminal.draw(|frame| app.draw(frame))?;
            assert_eq!(app.table.selected(), Some(0));
            assert_eq!(app.table.row_offset(), 0);
        }
        assert_eq!(app.selected(), Some(0));
        assert!(app.navigate(&Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::End,
            KeyModifiers::NONE,
        ))));
        let selected = app.selected().unwrap();
        app.receive(Message::State(selected, Check::new(Status::Ready), 0), 0);
        terminal.draw(|frame| app.draw(frame))?;
        assert_eq!(app.selected(), Some(selected));
        assert!(
            app.table.selected().unwrap() < app.table.row_offset() + app.table.vscroll.page_len()
        );
        Ok(())
    }
    #[test]
    fn both_palettes_keep_the_description_at_the_bottom() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        let without_spaces = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        for palette in [Palette::LATTE, Palette::MOCHA] {
            for zh in [true, false] {
                let mut app = App::new(catalog()?, zh)?;
                app.palette = palette;
                let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
                terminal.draw(|frame| app.draw(frame))?;
                let buffer = terminal.backend().buffer();
                assert_eq!(buffer[(0, 0)].bg, palette.base);
                assert!((0..100).any(|x| buffer[(x, 29)].symbol() != " "));
                let area = app.table.table_area;
                assert_eq!(area.bottom(), 28);
                let footer: String = (1..99).map(|x| buffer[(x, 29)].symbol()).collect();
                assert_eq!(
                    without_spaces(&footer),
                    without_spaces(app.catalog[0].purpose(zh))
                );
                assert_eq!(
                    buffer[(area.x + 14, app.table.header_area.y)].bg,
                    palette.base
                );
                assert_eq!(buffer[(area.x + 14, area.y)].bg, palette.surface);
                assert_eq!(buffer[(area.x + 14, area.y + 1)].bg, palette.base);
                app.table.move_to(app.visible.len() - 1);
                terminal.draw(|frame| app.draw(frame))?;
                app.table.move_to(0);
                terminal.draw(|frame| app.draw(frame))?;
                assert_eq!(app.table.row_offset(), 0);
                let bar = app.table.vscroll.area;
                assert_eq!(
                    terminal.backend().buffer()[(bar.x, bar.y + 1)].symbol(),
                    " "
                );
                assert_eq!(
                    terminal.backend().buffer()[(bar.x, bar.y + 1)].bg,
                    palette.blue
                );
                app.focus(app.catalog.iter().position(|f| f.id == "power").unwrap());
                terminal.backend_mut().resize(55, 24);
                terminal.draw(|frame| app.draw(frame))?;
                let buffer = terminal.backend().buffer();
                let footer: String = (app.table.table_area.bottom() + 1..24)
                    .flat_map(|y| (1..54).map(move |x| buffer[(x, y)].symbol()))
                    .collect();
                assert_eq!(
                    without_spaces(&footer),
                    without_spaces(app.catalog[app.selected().unwrap()].purpose(zh))
                );
            }
        }
        Ok(())
    }
    #[test]
    fn parallel_execution_keeps_selection_and_order_when_an_item_finishes() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.command(&Command::All);
        app.receive(Message::State(0, Check::new(Status::Running), 0), 0);
        app.receive(Message::State(1, Check::new(Status::Running), 0), 0);
        app.focus(1);
        let order = app.visible.clone();
        app.receive(Message::State(1, Check::new(Status::Done), 0), 0);
        assert_eq!(app.selected(), Some(1));
        assert_eq!(app.visible, order);
        assert_eq!(app.states[0].state, Status::Running);
        Ok(())
    }
    #[test]
    fn manual_actions_have_no_countdown_but_boot_continuation_does() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.receive(
            Message::Finished {
                processed: 1,
                restart: true,
                auto: false,
                refresh: vec![],
            },
            0,
        );
        assert!(app.countdown.is_none());
        assert!(app.message.contains("待手动重启"));
        app.receive(
            Message::Finished {
                processed: 1,
                restart: true,
                auto: true,
                refresh: vec![],
            },
            0,
        );
        assert!(app.countdown.is_some());
        Ok(())
    }
    #[test]
    fn stale_detection_cannot_replace_queued_or_running_status() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.command(&Command::One(0));
        app.receive(Message::State(0, Check::new(Status::Checking), 0), 1);
        assert_eq!(app.states[0].state, Status::Queued);
        app.receive(Message::State(0, Check::new(Status::Running), 1), 1);
        assert_eq!(app.states[0].state, Status::Running);
        app.receive(Message::State(0, Check::new(Status::Ready), 0), 1);
        assert_eq!(app.states[0].state, Status::Running);
        Ok(())
    }
    #[test]
    fn retry_keeps_the_selected_row_and_viewport_through_completion() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                auto: false,
                refresh: vec![],
            },
            0,
        );
        let i = *app.visible.last().unwrap();
        app.states[i] = Check::new(Status::Failed);
        terminal.draw(|frame| app.draw(frame))?;
        app.focus(i);
        terminal.draw(|frame| app.draw(frame))?;
        let order = app.visible.clone();
        let row = app.table.selected();
        let offset = app.table.row_offset();
        assert!(offset > 0);
        app.command(&Command::One(i));
        for state in [Status::Running, Status::Done] {
            app.receive(Message::State(i, Check::new(state), 0), 0);
            terminal.draw(|frame| app.draw(frame))?;
            assert_eq!(app.visible, order);
            assert_eq!(app.table.selected(), row);
            assert_eq!(app.table.row_offset(), offset);
        }
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                auto: false,
                refresh: vec![],
            },
            0,
        );
        terminal.draw(|frame| app.draw(frame))?;
        assert_eq!(app.visible, order);
        assert_eq!(app.table.selected(), row);
        assert_eq!(app.table.row_offset(), offset);
        Ok(())
    }
}
