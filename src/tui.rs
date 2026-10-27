use crate::{
    controller::{self, Command, Message},
    engine::Engine,
    model::*,
    native,
};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
};
use rat_widget::{
    button::{self, Button, ButtonState},
    event::ButtonOutcome,
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
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs, Wrap},
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
    pub countdown: Option<Instant>,
    pub restart: bool,
    batch_remaining: Vec<usize>,
    batch_total: usize,
    pending_batches: usize,
    active_indices: Vec<usize>,
    active_epoch: usize,
    running_since: Vec<Option<Instant>>,
    stages: Vec<String>,
    restoring: Vec<usize>,
    categories: Vec<usize>,
    category: usize,
    tables: Vec<TableState<RowSelection>>,
    tabs: Vec<Rect>,
    buttons: [ButtonState; 6],
    follow_top: bool,
    palette: Palette,
}
impl App {
    pub fn new(catalog: Vec<Feature>, zh: bool) -> Result<Self> {
        let mut categories: Vec<usize> = vec![];
        for (i, f) in catalog.iter().enumerate().filter(|(_, f)| !f.manual) {
            if !categories
                .iter()
                .any(|n| catalog[*n].group_zh == f.group_zh)
            {
                categories.push(i);
            }
        }
        const ORDER: &[&str] = &[
            "安全",
            "Defender",
            "核心隔离",
            "漏洞防护",
            "隐私",
            "系统",
            "桌面",
            "更新与还原",
            "卸载",
            "安装",
        ];
        categories.sort_by_key(|i| {
            ORDER
                .iter()
                .position(|name| *name == catalog[*i].group_zh)
                .unwrap_or(ORDER.len())
        });
        let mut app = Self {
            states: vec![Check::new(Status::PendingCheck); catalog.len()],
            running_since: vec![None; catalog.len()],
            stages: vec![String::new(); catalog.len()],
            catalog,
            visible: vec![],
            table: TableState::default(),
            zh,
            busy: false,
            message: String::new(),
            engine: None,
            countdown: None,
            restart: false,
            batch_remaining: vec![],
            batch_total: 0,
            pending_batches: 0,
            active_indices: vec![],
            active_epoch: 0,
            restoring: vec![],
            tables: (0..categories.len())
                .map(|_| {
                    let mut table = TableState::<RowSelection>::default();
                    table.selection.set_scroll_selected(true);
                    table.select(Some(0));
                    table
                })
                .collect(),
            tabs: vec![Rect::default(); categories.len()],
            categories,
            category: 0,
            buttons: std::array::from_fn(|_| ButtonState::default()),
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
        self.visible = self.category_items();
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
    fn category_items(&self) -> Vec<usize> {
        self.catalog
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                self.states[*i].visible(f)
                    && self
                        .categories
                        .get(self.category)
                        .is_some_and(|n| self.catalog[*n].group_zh == f.group_zh)
            })
            .map(|(i, _)| i)
            .collect()
    }
    #[cfg(test)]
    pub fn focus(&mut self, i: usize) {
        if let Some(category) = self
            .categories
            .iter()
            .position(|n| self.catalog[*n].group_zh == self.catalog[i].group_zh)
        {
            self.switch_category(category);
        }
        if let Some(row) = self.visible.iter().position(|v| *v == i) {
            self.follow_top = false;
            self.table.select(Some(row));
            self.table.scroll_to_selected();
        }
    }
    fn switch_category(&mut self, category: usize) -> bool {
        if category == self.category {
            return false;
        }
        std::mem::swap(&mut self.table, &mut self.tables[self.category]);
        self.category = category;
        self.follow_top = false;
        self.visible = self.category_items();
        std::mem::swap(&mut self.table, &mut self.tables[category]);
        true
    }
    fn category_command(&self) -> Command {
        Command::Items(
            self.visible
                .iter()
                .copied()
                .filter(|i| !self.catalog[*i].default_off)
                .collect(),
        )
    }
    fn checks_remaining(&self) -> usize {
        self.states
            .iter()
            .filter(|s| matches!(s.state, Status::PendingCheck | Status::Checking))
            .count()
    }
    fn actions_blocked(&self) -> bool {
        self.busy || self.checks_remaining() != 0
    }
    fn queue_checks(&mut self, indices: &[usize]) {
        for &i in indices {
            self.states[i] = Check::new(Status::PendingCheck);
        }
    }
    fn key_command(&self, key: KeyCode) -> Option<Command> {
        if !self.busy && self.checks_remaining() != 0 {
            return None;
        }
        match key {
            KeyCode::Char(' ') => self.space_command(),
            KeyCode::Char('r' | 'R') => self
                .selected()
                .filter(|i| {
                    !self.batch_remaining.contains(i) && !self.catalog[*i].restore_ops.is_empty()
                })
                .map(Command::Restore),
            _ if self.busy => None,
            KeyCode::Char('a' | 'A')
                if self
                    .states
                    .iter()
                    .enumerate()
                    .any(|(i, s)| !self.catalog[i].default_off && s.actionable()) =>
            {
                Some(Command::All)
            }
            KeyCode::Char('c' | 'C')
                if self
                    .visible
                    .iter()
                    .any(|i| !self.catalog[*i].default_off && self.states[*i].actionable()) =>
            {
                Some(self.category_command())
            }
            KeyCode::F(10) if self.restart => Some(Command::Restart),
            _ => None,
        }
    }
    pub fn command(&mut self, command: &Command) -> bool {
        if self.busy
            && !matches!(command, Command::One(i) | Command::Restore(i) if !self.batch_remaining.contains(i))
            || !self.busy && self.checks_remaining() != 0
        {
            return false;
        }
        self.follow_top = false;
        self.countdown = None;
        if let Command::Scan(indices) = command {
            self.queue_checks(indices);
            return true;
        }
        self.busy = true;
        self.pending_batches += 1;
        for i in 0..self.catalog.len() {
            let chosen = match command {
                Command::All => !self.catalog[i].default_off,
                Command::One(n) | Command::Restore(n) => i == *n,
                Command::Items(indices) => !self.catalog[i].default_off && indices.contains(&i),
                Command::RestoreItems(indices) => indices.contains(&i),
                _ => false,
            };
            if chosen {
                self.batch_remaining.push(i);
                self.batch_total += 1;
                if matches!(command, Command::Restore(_) | Command::RestoreItems(_)) {
                    if !self.restoring.contains(&i) {
                        self.restoring.push(i);
                    }
                } else {
                    self.restoring.retain(|index| *index != i);
                }
            }
            if chosen
                && (self.states[i].actionable()
                    || matches!(command, Command::Restore(_) | Command::RestoreItems(_)))
            {
                self.states[i] = Check::new(Status::Queued)
            }
        }
        self.message.clear();
        if matches!(command, Command::Restart) {
            self.message = choose(self.zh, "正在重启", "Restarting").into();
        }
        self.reorder();
        true
    }
    fn space_command(&self) -> Option<Command> {
        if !self.busy && self.checks_remaining() != 0 {
            return None;
        }
        let i = self.selected()?;
        if self.batch_remaining.contains(&i) {
            return None;
        }
        if self.states[i].state == Status::Failed && self.restoring.contains(&i) {
            return Some(Command::Restore(i));
        }
        if self.states[i].actionable() || self.catalog[i].toggle() {
            Some(Command::One(i))
        } else {
            None
        }
    }
    fn navigate(&mut self, event: &Event) -> bool {
        let target = match event {
            Event::Key(key) => match key.code {
                KeyCode::Left | KeyCode::BackTab => {
                    Some((self.category + self.categories.len() - 1) % self.categories.len())
                }
                KeyCode::Right | KeyCode::Tab => Some((self.category + 1) % self.categories.len()),
                _ => None,
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => self
                .tabs
                .iter()
                .position(|area| area.contains((mouse.column, mouse.row).into())),
            _ => None,
        };
        if let Some(category) = target {
            return self.switch_category(category);
        }
        let before = self.table.selected();
        let outcome = rowselection::handle_events(&mut self.table, true, event);
        let changed =
            before != self.table.selected() || outcome != rat_widget::event::TableOutcome::Continue;
        if changed {
            self.follow_top = false;
        }
        changed
    }
    fn description(&self) -> &str {
        self.selected()
            .map_or("", |i| self.catalog[i].purpose(self.zh))
    }
    fn activity(&self) -> String {
        let running: Vec<_> = self
            .states
            .iter()
            .enumerate()
            .filter(|(_, state)| state.state == Status::Running)
            .map(|(i, _)| i)
            .collect();
        let Some(i) = self
            .selected()
            .filter(|i| running.contains(i))
            .or_else(|| running.first().copied())
        else {
            return String::new();
        };
        let seconds = self.running_since[i].map_or(0, |started| started.elapsed().as_secs());
        let mut activity = format!(
            "{} · {} · {seconds}s",
            self.catalog[i].name(self.zh),
            self.stages[i]
        );
        if running.len() > 1 {
            activity.push_str(&format!(
                " · +{} {}",
                running.len() - 1,
                choose(self.zh, "项", "more")
            ));
        }
        activity
    }
    fn footer_message(&self) -> String {
        let mut message = self.message.clone();
        if self.busy && self.batch_total != 0 {
            let completed = self.batch_total - self.batch_remaining.len();
            message = format!(
                "{} {completed}/{}",
                choose(self.zh, "执行中", "Running"),
                self.batch_total,
            );
            let activity = self.activity();
            if !activity.is_empty() {
                message.push_str(" · ");
                message.push_str(&activity);
            }
        } else if self.checks_remaining() != 0 && (self.engine.is_some() || self.message.is_empty())
        {
            message = format!(
                "{} · {} {}",
                choose(self.zh, "检测中", "Checking"),
                self.checks_remaining(),
                choose(self.zh, "项待完成", "remaining")
            );
        } else if !self.busy && self.restart && message.is_empty() {
            message = choose(self.zh, "需要重启", "Restart required").into();
        }
        if let Some(deadline) = self.countdown {
            let seconds = deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
            message = format!(
                "{seconds}{}",
                choose(
                    self.zh,
                    " 秒后重启，任意键停止",
                    "s to restart; any key stops"
                )
            );
        }
        message
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
        let message = self.footer_message();
        let color = if self.actions_blocked() {
            p.blue
        } else if self.states.iter().any(|s| s.state == Status::Failed) {
            p.red
        } else if self.restart {
            p.blue
        } else {
            p.text
        };
        let top = 2;
        let description = self.description().to_owned();
        let bottom = if description.is_empty() {
            0
        } else {
            1 + Paragraph::new(description.as_str())
                .wrap(Wrap { trim: true })
                .line_count(area.width) as u16
        };
        let bottom = bottom.min(area.height.saturating_sub(top + 6));
        let layout = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(bottom),
            Constraint::Length(1),
        ])
        .split(area);
        let buttons = self.toolbar(layout[4]);
        let message_width = buttons.first().map_or(area.width, |(_, _, rect, _)| {
            rect.x.saturating_sub(area.x + 2)
        });
        frame.render_widget(
            Paragraph::new(message).style(style.fg(color)),
            Rect::new(area.x, layout[4].y, message_width, 1),
        );
        for (index, label, rect, enabled) in buttons {
            let foreground = if enabled { p.blue } else { p.muted };
            frame.render_stateful_widget(
                Button::new(label)
                    .style(style.fg(foreground))
                    .hover_style(
                        style
                            .fg(foreground)
                            .bg(if enabled { p.surface } else { p.base }),
                    )
                    .armed_style(if enabled {
                        style.fg(p.base).bg(p.blue)
                    } else {
                        style.fg(p.muted)
                    }),
                rect,
                &mut self.buttons[index],
            );
        }
        self.draw_tabs(frame, layout[0], style);
        let table_area = layout[2];
        let rows = self.visible.iter().map(|i| {
            let f = &self.catalog[*i];
            Row::new([
                Cell::from(f.name(self.zh).to_owned()),
                Cell::from(if let Some(started) = self.running_since[*i] {
                    format!(
                        "{} {}s",
                        self.states[*i].label(f, self.zh),
                        started.elapsed().as_secs()
                    )
                } else {
                    self.states[*i].label(f, self.zh)
                })
                .style(Some(Style::default().fg(p.status(self.states[*i].state)))),
            ])
        });
        let status = if self.zh { 16 } else { 20 };
        let table = Table::default()
            .style(style)
            .rows(rows)
            .header(
                Row::new([
                    choose(self.zh, "项目", "Item"),
                    choose(self.zh, "状态", "Status"),
                ])
                .style(Some(Style::default().fg(p.muted).bg(p.base))),
            )
            .widths([Constraint::Fill(1), Constraint::Length(status)])
            .column_spacing(2)
            .select_row_style(Some(Style::default().bg(p.surface)))
            .show_row_focus(false)
            .vscroll(
                Scroll::vertical()
                    .policy(ScrollbarPolicy::Collapse)
                    .style(style.fg(p.border))
                    .thumb_symbol(" ")
                    .thumb_style(style.bg(p.blue)),
            );
        frame.render_stateful_widget(&table, table_area, &mut self.table);
        if self.table.scroll_to_selected() {
            frame.render_stateful_widget(&table, table_area, &mut self.table);
        }
        if bottom != 0 {
            frame.render_widget(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(p.border)),
                layout[3],
            );
            frame.render_widget(
                Paragraph::new(description.as_str())
                    .wrap(Wrap { trim: true })
                    .style(style),
                Rect::new(
                    area.x,
                    layout[3].y + 1,
                    area.width,
                    layout[3].height.saturating_sub(1),
                ),
            );
        }
    }

    fn button_specs(&self) -> Vec<(usize, KeyCode, String, bool)> {
        let item = if self.selected().is_some_and(|i| self.catalog[i].toggle()) {
            choose(self.zh, "切换", "Switch")
        } else {
            choose(self.zh, "执行", "Run")
        };
        [
            (
                0,
                KeyCode::Char(' '),
                format!("Space {item}"),
                self.space_command().is_some(),
            ),
            (
                1,
                KeyCode::Char('c'),
                choose(self.zh, "C 本类", "C Category").into(),
                self.key_command(KeyCode::Char('c')).is_some(),
            ),
            (
                2,
                KeyCode::Char('a'),
                choose(self.zh, "A 全部", "A All").into(),
                self.key_command(KeyCode::Char('a')).is_some(),
            ),
            (
                3,
                KeyCode::Char('r'),
                choose(self.zh, "R 恢复", "R Restore").into(),
                self.key_command(KeyCode::Char('r')).is_some(),
            ),
            (
                5,
                KeyCode::F(10),
                choose(self.zh, "F10 重启", "F10 Restart").into(),
                self.restart,
            ),
            (
                4,
                KeyCode::Char('q'),
                choose(self.zh, "Q 退出", "Q Exit").into(),
                true,
            ),
        ]
        .into_iter()
        .filter(|(i, _, _, _)| *i != 5 || self.restart)
        .map(|(i, key, label, enabled)| {
            (
                i,
                key,
                label,
                enabled
                    && (i != 4 && i != 5 || !self.busy)
                    && (i == 4 || self.busy || self.checks_remaining() == 0),
            )
        })
        .collect()
    }
    fn draw_tabs(&mut self, frame: &mut Frame, area: Rect, style: Style) {
        let titles: Vec<_> = self
            .categories
            .iter()
            .map(|i| self.catalog[*i].category(self.zh))
            .collect();
        let widths: Vec<_> = titles
            .iter()
            .map(|title| Line::from(*title).width() + 2)
            .collect();
        let mut first = 0;
        while first < self.category
            && widths[first..=self.category].iter().sum::<usize>() + self.category - first
                > usize::from(area.width)
        {
            first += 1;
        }
        self.tabs.fill(Rect::default());
        let mut x = area.x;
        for (i, width) in widths.iter().enumerate().skip(first) {
            if x >= area.right() {
                break;
            }
            let width = (*width as u16).min(area.right() - x);
            self.tabs[i] = Rect::new(x, area.y, width, 1);
            x += width + 1;
        }
        frame.render_widget(
            Tabs::new(titles.into_iter().skip(first))
                .select(self.category - first)
                .style(style.fg(self.palette.muted))
                .highlight_style(style.fg(self.palette.blue).bg(self.palette.surface)),
            area,
        );
    }
    fn toolbar(&self, area: Rect) -> Vec<(usize, String, Rect, bool)> {
        let mut specs = self.button_specs();
        let width = |specs: &[(usize, KeyCode, String, bool)]| -> u16 {
            specs
                .iter()
                .map(|(_, _, label, _)| Line::from(label.as_str()).width() as u16)
                .sum::<u16>()
                + 2 * specs.len().saturating_sub(1) as u16
        };
        let message = Line::from(self.footer_message()).width() as u16;
        let reserved = if message == 0 { 0 } else { message + 2 };
        if width(&specs) + reserved > area.width {
            for (index, _, label, _) in &mut specs {
                *label = ["Space", "C", "A", "R", "Q", "F10"][*index].into();
            }
        }
        let mut x = area.right().saturating_sub(width(&specs));
        specs
            .into_iter()
            .map(|(i, _, label, enabled)| {
                let width = Line::from(label.as_str()).width() as u16;
                let rect = Rect::new(x, area.y, width, 1);
                x += width + 2;
                (i, label, rect, enabled)
            })
            .collect()
    }
    fn button_event(&mut self, event: &Event) -> (Option<KeyCode>, bool) {
        for (i, key, _, enabled) in self.button_specs() {
            if enabled {
                match button::handle_mouse_events(&mut self.buttons[i], event) {
                    ButtonOutcome::Pressed => return (Some(key), true),
                    ButtonOutcome::Changed => return (None, true),
                    _ => {}
                }
            }
        }
        (None, false)
    }
    fn receive(&mut self, msg: Message, epoch: usize) {
        match msg {
            Message::Ready(engine) => self.engine = Some(engine),
            Message::Context {
                restart,
                safe_queued,
                restoring,
            } => {
                self.restart = restart;
                self.restoring = restoring;
                for i in safe_queued {
                    self.states[i] = Check::new(Status::SafeQueued);
                }
            }
            Message::Progress { index, stage } => {
                self.stages[index] = stage;
            }
            Message::State(i, state, version)
                if version == epoch
                    || version == self.active_epoch && self.active_indices.contains(&i) =>
            {
                if self.states[i].state == Status::Queued
                    && !self.active_indices.contains(&i)
                    && matches!(state.state, Status::PendingCheck | Status::Checking)
                {
                    return;
                }
                if state.state == Status::Running {
                    self.running_since[i].get_or_insert_with(Instant::now);
                } else {
                    self.running_since[i] = None;
                    self.stages[i].clear();
                }
                if self.busy
                    && !matches!(
                        state.state,
                        Status::PendingCheck | Status::Checking | Status::Queued | Status::Running
                    )
                {
                    self.batch_remaining.retain(|index| *index != i);
                }
                self.states[i] = state;
            }
            Message::State(..) => {}
            Message::Queued(indices, version) => {
                self.busy = true;
                self.countdown = None;
                self.active_indices = indices.clone();
                self.active_epoch = version;
                if self.pending_batches == 0 {
                    self.pending_batches = 1;
                    self.batch_total = indices.len();
                    self.batch_remaining = indices.clone();
                }
                for i in indices {
                    if self.states[i].actionable()
                        || matches!(
                            self.states[i].state,
                            Status::PendingCheck | Status::Checking
                        )
                    {
                        self.states[i] = Check::new(Status::Queued);
                    }
                }
            }
            Message::Finished { restart, .. } => {
                self.pending_batches = self.pending_batches.saturating_sub(1);
                self.busy = self.pending_batches != 0;
                self.active_indices.clear();
                if !self.busy {
                    self.batch_remaining.clear();
                    self.batch_total = 0;
                }
                self.countdown = None;
                self.restart = restart;
                self.message.clear();
                if !self.busy && restart && self.engine.as_ref().is_some_and(|engine| engine.safe) {
                    self.countdown = Some(Instant::now() + Duration::from_secs(10))
                }
                self.reorder();
            }
            Message::Error(error) => {
                self.pending_batches = self.pending_batches.saturating_sub(1);
                self.busy = self.pending_batches != 0;
                for i in self.active_indices.drain(..) {
                    self.batch_remaining.retain(|index| *index != i);
                    if matches!(self.states[i].state, Status::Queued | Status::Running) {
                        self.states[i] = Check {
                            state: Status::Failed,
                            detail: error.clone(),
                        };
                        self.running_since[i] = None;
                    }
                }
                if !self.busy {
                    self.batch_remaining.clear();
                    self.batch_total = 0;
                }
                self.countdown = None;
                self.message = if self.engine.is_some() {
                    choose(self.zh, "执行失败", "Execution failed").into()
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
    let zh = native::chinese();
    let _font = native::ConsoleFont::new();
    let _guard = TerminalGuard;
    let mut terminal = ratatui::try_init()?;
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let mut app = App::new(catalog()?, zh)?;
    let Color::Rgb(r, g, b) = app.palette.base else {
        unreachable!()
    };
    let background = native::ConsoleBackground::new(r, g, b)?;
    terminal.draw(|frame| app.draw(frame))?;
    native::hide_console_scrollbars();
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
    let mut pending_refresh = Vec::new();
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
        for msg in rx.try_iter().take(64) {
            let finished = matches!(&msg, Message::Finished { .. } | Message::Error(_));
            if let Message::Finished { refresh, .. } = &msg {
                pending_refresh.extend(refresh);
            }
            app.receive(msg, epoch.load(Ordering::SeqCst));
            if finished && app.engine.is_some() && !app.busy {
                let mut refresh = std::mem::take(&mut pending_refresh);
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
                if !refresh.is_empty() {
                    app.queue_checks(&refresh);
                    commands.send(Command::Scan(refresh))?;
                }
            }
            dirty = true
        }
        if app.running_since.iter().any(Option::is_some) {
            let seconds = app
                .running_since
                .iter()
                .flatten()
                .map(|started| started.elapsed().as_secs())
                .max();
            if last_second != seconds {
                last_second = seconds;
                dirty = true;
            }
        }
        if let Some(deadline) = app.countdown {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                commands.send(Command::Restart)?;
                app.countdown = None;
                app.message = choose(app.zh, "正在重启", "Restarting").into()
            }
            if last_second != Some(remaining.as_secs()) {
                last_second = Some(remaining.as_secs());
                dirty = true
            }
        }
        if dirty {
            terminal.draw(|frame| app.draw(frame))?;
            native::hide_console_scrollbars();
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
                app.message.clear();
                dirty = true;
                continue;
            }
            if matches!(key.code, KeyCode::Char('q' | 'Q') | KeyCode::Esc) && !app.busy {
                return Ok(());
            }
            let command = app.key_command(key.code);
            if let Some(command) = command {
                if app.command(&command) {
                    commands.send(command)?;
                }
                dirty = true;
                continue;
            }
        }
        let (button_key, button_changed) = app.button_event(&event);
        dirty |= button_changed;
        if let Some(key) = button_key {
            if key == KeyCode::Char('q') {
                return Ok(());
            }
            if let Some(command) = app.key_command(key)
                && app.command(&command)
            {
                commands.send(command)?;
            }
            continue;
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
    #[test]
    fn detection_blocks_actions_across_categories_and_does_not_queue_them() -> Result<()> {
        use crossterm::event::{KeyEvent, MouseEvent};
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        app.restart = true;
        let i = app.selected().unwrap();
        for state in [Status::PendingCheck, Status::Checking] {
            let check = Check::new(state);
            assert!(!check.actionable());
        }
        assert!(!app.command(&Command::All));
        app.states.fill(Check::new(Status::Done));
        app.states[i] = Check::new(Status::Ready);
        let last = app.catalog.len() - 1;
        assert!(!app.visible.contains(&last));
        let commands = [
            Command::One(i),
            app.category_command(),
            Command::All,
            Command::Restore(i),
            Command::Restart,
        ];
        let keys = [
            KeyCode::Char(' '),
            KeyCode::Char('c'),
            KeyCode::Char('C'),
            KeyCode::Char('a'),
            KeyCode::Char('A'),
            KeyCode::Char('r'),
            KeyCode::Char('R'),
            KeyCode::F(10),
        ];
        let mut terminal = Terminal::new(TestBackend::new(140, 24))?;
        for state in [Status::PendingCheck, Status::Checking] {
            app.states[last] = Check::new(state);
            let before: Vec<_> = app.states.iter().map(|s| s.state).collect();
            for key in keys {
                assert!(app.key_command(key).is_none());
            }
            for command in &commands {
                assert!(!app.command(command));
            }
            assert!(!app.busy);
            assert_eq!(
                before,
                app.states.iter().map(|s| s.state).collect::<Vec<_>>()
            );
            terminal.draw(|f| app.draw(f))?;
            for (index, _, _, enabled) in app.button_specs() {
                assert_eq!(enabled, index == 4);
                let rect = app.buttons[index].area;
                for kind in [
                    MouseEventKind::Down(MouseButton::Left),
                    MouseEventKind::Up(MouseButton::Left),
                ] {
                    if index == 4 {
                        continue;
                    }
                    assert_eq!(
                        app.button_event(&Event::Mouse(MouseEvent {
                            kind,
                            column: rect.x + 1,
                            row: rect.y,
                            modifiers: KeyModifiers::NONE
                        })),
                        (None, false)
                    );
                }
            }
        }
        assert!(app.navigate(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))));
        assert!(app.navigate(&Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT
        ))));
        app.receive(Message::State(last, Check::new(Status::Ready), 0), 0);
        for key in keys {
            assert!(app.key_command(key).is_some());
        }
        app.queue_checks(&[last]);
        assert!(app.key_command(KeyCode::Char('a')).is_none());
        app.receive(Message::State(last, Check::new(Status::Done), 0), 0);
        assert!(app.command(&Command::One(i)));
        assert_eq!(app.states[i].state, Status::Queued);
        Ok(())
    }
    #[test]
    fn idle_footer_is_empty_and_detection_reports_remaining_items() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        for zh in [true, false] {
            let mut app = App::new(catalog()?, zh)?;
            app.states.fill(Check::new(Status::Done));
            let mut terminal = Terminal::new(TestBackend::new(140, 24))?;
            terminal.draw(|f| app.draw(f))?;
            let first_button = app.toolbar(Rect::new(1, 23, 138, 1))[0].2.x;
            let footer = |terminal: &Terminal<TestBackend>| {
                (1..first_button)
                    .map(|x| terminal.backend().buffer()[(x, 23)].symbol())
                    .collect::<String>()
            };
            assert!(footer(&terminal).trim().is_empty());
            app.queue_checks(&[0]);
            terminal.draw(|f| app.draw(f))?;
            let text: String = footer(&terminal)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            assert!(text.contains(choose(zh, "检测中", "Checking")), "{text:?}");
            assert!(footer(&terminal).contains('1'));
            app.receive(Message::State(0, Check::new(Status::Done), 0), 0);
            terminal.draw(|f| app.draw(f))?;
            assert!(footer(&terminal).trim().is_empty());
        }
        Ok(())
    }
    #[test]
    fn restore_and_restart_have_separate_keys_and_fit_a_narrow_toolbar() -> anyhow::Result<()> {
        use super::*;
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        app.states.fill(Check::new(Status::Done));
        let i = app.selected().unwrap();
        app.states[i] = Check::new(Status::Done);
        assert!(matches!(app.key_command(KeyCode::Char('R')), Some(Command::Restore(n)) if n == i));
        assert!(app.key_command(KeyCode::F(10)).is_none());
        app.restart = true;
        assert!(matches!(app.key_command(KeyCode::Char('r')), Some(Command::Restore(n)) if n == i));
        assert!(matches!(
            app.key_command(KeyCode::F(10)),
            Some(Command::Restart)
        ));
        let mut terminal = Terminal::new(TestBackend::new(60, 24))?;
        terminal.draw(|f| app.draw(f))?;
        app.busy = true;
        assert!(matches!(app.key_command(KeyCode::Char('r')), Some(Command::Restore(n)) if n == i));
        app.busy = false;
        app.states.fill(Check::new(Status::Done));
        let child = app.catalog.iter().position(|f| f.default_off).unwrap();
        app.states[child] = Check::new(Status::Ready);
        app.focus(child);
        assert!(app.key_command(KeyCode::Char('a')).is_none());
        assert!(app.key_command(KeyCode::Char('c')).is_none());
        assert!(matches!(app.space_command(), Some(Command::One(i)) if i == child));
        Ok(())
    }
    use super::*;
    #[test]
    fn category_execution_is_scoped_and_all_execution_crosses_tabs() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.states.fill(Check::new(Status::Ready));
        let mut seen = vec![];
        for category in 0..app.categories.len() {
            app.switch_category(category);
            let group = &app.catalog[app.categories[category]].group_zh;
            assert!(
                app.visible
                    .iter()
                    .all(|i| app.catalog[*i].group_zh == *group)
            );
            assert!(app.visible.windows(2).all(|pair| pair[0] < pair[1]));
            seen.extend_from_slice(&app.visible);
            let expected: Vec<_> = app
                .visible
                .iter()
                .copied()
                .filter(|i| !app.catalog[*i].default_off)
                .collect();
            for key in ['c', 'C'] {
                assert!(
                    matches!(app.key_command(KeyCode::Char(key)), Some(Command::Items(indices)) if indices == expected)
                );
            }
        }
        seen.sort();
        assert_eq!(
            seen,
            app.catalog
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.manual)
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        );
        let indices = app.visible.clone();
        app.states[indices[0]] = Check::new(Status::Done);
        app.states[indices[1]] = Check::new(Status::Failed);
        app.command(&app.key_command(KeyCode::Char('c')).unwrap());
        for (i, state) in app.states.iter().enumerate() {
            assert_eq!(
                state.state,
                if i == indices[0] {
                    Status::Done
                } else if indices.contains(&i) {
                    Status::Queued
                } else {
                    Status::Ready
                }
            );
        }
        app.busy = false;
        for key in ['a', 'A'] {
            assert!(matches!(
                app.key_command(KeyCode::Char(key)),
                Some(Command::All)
            ));
        }
        app.command(&Command::All);
        for (state, feature) in app.states.iter().zip(&app.catalog) {
            if feature.default_off {
                assert_eq!(state.state, Status::Ready);
            } else {
                assert!(matches!(state.state, Status::Queued | Status::Done));
            }
        }
        Ok(())
    }
    #[test]
    fn tab_navigation_keeps_each_selection_and_scroll_during_updates() -> Result<()> {
        use crossterm::event::{KeyEvent, MouseEvent};
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        let mut terminal = Terminal::new(TestBackend::new(80, 20))?;
        app.focus(
            app.catalog
                .iter()
                .position(|f| f.id == "search-web")
                .unwrap(),
        );
        terminal.draw(|frame| app.draw(frame))?;
        let (category, selected, offset) = (app.category, app.selected(), app.table.row_offset());
        assert!(offset > 0);
        let next = Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.navigate(&next));
        terminal.draw(|frame| app.draw(frame))?;
        let other_selected = app.selected();
        app.busy = true;
        app.receive(
            Message::State(selected.unwrap(), Check::new(Status::Running), 0),
            0,
        );
        assert_eq!(app.selected(), other_selected);
        assert!(app.navigate(&Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT
        ))));
        app.busy = false;
        terminal.draw(|frame| app.draw(frame))?;
        assert_eq!(
            (app.category, app.selected(), app.table.row_offset()),
            (category, selected, offset)
        );
        let target = if category == 0 { 1 } else { 0 };
        let rect = app.tabs[target];
        assert!(app.navigate(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        })));
        assert_eq!(app.category, target);
        Ok(())
    }
    #[test]
    fn every_tab_is_reachable_in_a_narrow_terminal() -> Result<()> {
        use crossterm::event::KeyEvent;
        use ratatui::{Terminal, backend::TestBackend};
        for zh in [true, false] {
            let mut app = App::new(catalog()?, zh)?;
            let mut terminal = Terminal::new(TestBackend::new(55, 24))?;
            for category in 0..app.categories.len() {
                assert_eq!(app.category, category);
                terminal.draw(|frame| app.draw(frame))?;
                assert!(app.tabs[category].width > 0);
                let tab = app.tabs[category];
                let title = app.catalog[app.categories[category]].category(zh);
                assert_eq!(
                    terminal.backend().buffer()[(tab.x + 1, tab.y)].symbol(),
                    title.chars().next().unwrap().to_string()
                );
                assert_eq!(
                    terminal.backend().buffer()[(tab.x + 1, tab.y)].bg,
                    app.palette.surface
                );
                assert!(app.navigate(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))));
            }
            assert_eq!(app.category, 0);
        }
        Ok(())
    }
    #[test]
    fn execution_info_and_actions_share_the_bottom_row() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        for zh in [true, false] {
            for width in [55, 120, 160] {
                let mut app = App::new(catalog()?, zh)?;
                app.states.fill(Check::new(Status::Ready));
                let node = app
                    .catalog
                    .iter()
                    .position(|f| f.id == "install-openjs.nodejs")
                    .unwrap();
                let git = app
                    .catalog
                    .iter()
                    .position(|f| f.id == "install-git.git")
                    .unwrap();
                app.focus(node);
                app.receive(Message::Queued(vec![node, git, 0], 0), 0);
                for i in [node, git] {
                    app.receive(Message::State(i, Check::new(Status::Running), 0), 0);
                    app.receive(
                        Message::Progress {
                            index: i,
                            stage: choose(zh, "下载并安装软件", "Downloading and installing")
                                .into(),
                        },
                        0,
                    );
                }
                let mut terminal = Terminal::new(TestBackend::new(width, 24))?;
                terminal.draw(|frame| app.draw(frame))?;
                let buffer = terminal.backend().buffer();
                assert_eq!(buffer[(1, 23)].symbol(), if zh { "执" } else { "R" });
                assert_eq!(buffer[(1, 0)].symbol(), " ");
                assert!((1..width - 1).all(|x| buffer[(x, 1)].symbol() == " "));
                let specs = app.toolbar(Rect::new(1, 23, width - 2, 1));
                let footer: String = (1..specs.first().unwrap().2.x)
                    .map(|x| buffer[(x, 23)].symbol())
                    .collect();
                assert!(footer.contains("Node.js"));
                if width >= 120 {
                    let compact = |text: &str| {
                        text.chars()
                            .filter(|c| !c.is_whitespace())
                            .collect::<String>()
                    };
                    assert_eq!(compact(&footer), compact(&app.footer_message()));
                }
                for (index, label, rect, _) in specs {
                    assert_eq!(app.buttons[index].area, rect);
                    assert_eq!(rect.y, 23);
                    assert_eq!(
                        buffer[(rect.x, rect.y)].symbol(),
                        label.chars().next().unwrap().to_string()
                    );
                }
                let last = app.buttons[4].area;
                assert_eq!(last.right(), width - 1);
                app.focus(git);
                terminal.draw(|frame| app.draw(frame))?;
                let footer: String = (1..app.buttons[0].area.x)
                    .map(|x| terminal.backend().buffer()[(x, 23)].symbol())
                    .collect();
                assert!(footer.contains("Git"));
            }
        }
        Ok(())
    }
    #[test]
    fn category_button_click_runs_only_its_items_and_is_disabled_while_busy() -> Result<()> {
        use crossterm::event::MouseEvent;
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        app.states.fill(Check::new(Status::Ready));
        let mut terminal = Terminal::new(TestBackend::new(120, 24))?;
        terminal.draw(|frame| app.draw(frame))?;
        let area = app.buttons[1].area;
        let mouse = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                column: area.x + 1,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            })
        };
        assert_eq!(
            app.button_event(&mouse(MouseEventKind::Down(MouseButton::Left))),
            (None, true)
        );
        let (key, changed) = app.button_event(&mouse(MouseEventKind::Up(MouseButton::Left)));
        assert_eq!(key, Some(KeyCode::Char('c')));
        assert!(changed);
        let command = app.key_command(key.unwrap()).unwrap();
        let Command::Items(ref items) = command else {
            panic!("Expected category items")
        };
        assert_eq!(items, &app.visible);
        let items = items.clone();
        app.command(&command);
        for (i, state) in app.states.iter().enumerate() {
            assert_eq!(
                state.state,
                if items.contains(&i) {
                    Status::Queued
                } else {
                    Status::Ready
                }
            );
        }
        terminal.draw(|frame| app.draw(frame))?;
        assert_eq!(
            app.button_event(&mouse(MouseEventKind::Down(MouseButton::Left))),
            (None, false)
        );
        assert_eq!(
            app.button_event(&mouse(MouseEventKind::Up(MouseButton::Left))),
            (None, false)
        );
        app.busy = false;
        app.states.fill(Check::new(Status::Done));
        assert!(app.key_command(KeyCode::Char('c')).is_none());
        assert!(app.key_command(KeyCode::Char('a')).is_none());
        Ok(())
    }
    #[test]
    fn unknown_state_executes_with_one_space_and_keeps_selection() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.states.fill(Check::new(Status::Done));
        let i = app
            .catalog
            .iter()
            .position(|f| f.id == "phishing-protection")
            .unwrap();
        app.focus(i);
        app.states[i] = Check::new(Status::Unknown);
        let command = app.space_command().unwrap();
        assert!(matches!(&command, Command::One(n) if *n == i));
        app.command(&command);
        assert!(app.busy);
        assert_eq!(app.selected(), Some(i));
        assert_eq!(app.states[i].state, Status::Queued);
        app.receive(Message::State(i, Check::new(Status::Ready), 0), 0);
        assert!(matches!(app.space_command(), Some(Command::One(n)) if n == i));
        app.states[i] = Check::new(Status::Failed);
        assert!(matches!(app.space_command(), Some(Command::One(n)) if n == i));
        for state in [
            Status::Absent,
            Status::Deferred,
            Status::Done,
            Status::Restricted,
        ] {
            app.states[i] = Check::new(state);
            assert!(app.space_command().is_none());
        }
        Ok(())
    }
    #[test]
    fn another_item_can_be_queued_while_running_and_finishes_in_order() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.states.fill(Check::new(Status::Ready));
        let first = app
            .catalog
            .iter()
            .position(|f| f.id == "install-9plm9xgg6vks")
            .unwrap();
        let second = app
            .catalog
            .iter()
            .position(|f| f.id == "install-git.git")
            .unwrap();
        app.focus(first);
        assert!(app.command(&app.space_command().unwrap()));
        app.receive(Message::Queued(vec![first], 1), 1);
        app.receive(Message::State(first, Check::new(Status::Running), 1), 1);
        app.receive(
            Message::Progress {
                index: first,
                stage: "下载并安装软件".into(),
            },
            1,
        );
        assert!(app.activity().contains("ChatGPT · 下载并安装软件 · 0s"));
        app.focus(second);
        assert!(app.command(&app.space_command().unwrap()));
        assert!(app.space_command().is_none());
        assert!(!app.command(&Command::One(second)));
        app.receive(Message::State(first, Check::new(Status::Failed), 1), 2);
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                refresh: vec![],
            },
            1,
        );
        assert!(app.busy);
        assert_eq!(app.states[second].state, Status::Queued);
        app.receive(Message::Queued(vec![second], 2), 2);
        app.receive(Message::State(second, Check::new(Status::Running), 2), 2);
        app.receive(Message::State(second, Check::new(Status::Done), 2), 2);
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                refresh: vec![],
            },
            2,
        );
        assert!(!app.busy);
        assert!(app.activity().is_empty());
        assert_eq!(app.selected(), Some(second));
        Ok(())
    }

    #[test]
    fn changing_the_palette_keeps_selection_and_scroll_position() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        let mut terminal = Terminal::new(TestBackend::new(100, 24))?;
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
    fn finished_actions_show_the_restart_requirement_beside_labeled_buttons() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        for zh in [true, false] {
            for state in [
                Status::Done,
                Status::Restricted,
                Status::Unknown,
                Status::Failed,
            ] {
                let mut app = App::new(catalog()?, zh)?;
                app.states.fill(Check::new(Status::Done));
                app.states[0] = Check::new(state);
                app.message = "Previous action".into();
                app.receive(
                    Message::Finished {
                        processed: 1,
                        restart: true,
                        refresh: vec![],
                    },
                    0,
                );
                let mut terminal = Terminal::new(TestBackend::new(100, 24))?;
                terminal.draw(|frame| app.draw(frame))?;
                let buttons = app.toolbar(Rect::new(1, 23, 98, 1));
                let end = buttons.first().unwrap().2.x;
                let buffer = terminal.backend().buffer();
                let message: String = (1..end).map(|x| buffer[(x, 23)].symbol()).collect();
                assert_eq!(
                    message
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>(),
                    choose(zh, "需要重启", "Restartrequired")
                );
                assert_eq!(app.states[0].state, state);
                assert!(buttons.iter().any(|(i, _, _, enabled)| *i == 5 && *enabled));
            }
        }
        Ok(())
    }
    #[test]
    fn execution_progress_counts_the_batch_across_categories() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.states.fill(Check::new(Status::Done));
        let last = app.catalog.len() - 1;
        app.states[0] = Check::new(Status::Ready);
        app.states[last] = Check::new(Status::Ready);
        assert!(app.command(&Command::Items(vec![0, last])));
        app.receive(Message::Queued(vec![0, last], 2), 2);
        assert_eq!(app.footer_message(), "执行中 0/2");
        app.receive(Message::State(last, Check::new(Status::Running), 2), 2);
        app.receive(Message::State(1, Check::new(Status::Done), 2), 2);
        app.receive(Message::State(0, Check::new(Status::Done), 1), 2);
        assert!(app.footer_message().starts_with("执行中 0/2"));
        app.receive(Message::State(0, Check::new(Status::Done), 2), 2);
        assert!(app.footer_message().starts_with("执行中 1/2"));
        app.receive(Message::State(0, Check::new(Status::Done), 2), 2);
        assert!(app.footer_message().starts_with("执行中 1/2"));
        app.receive(Message::State(last, Check::new(Status::Failed), 2), 2);
        assert_eq!(app.footer_message(), "执行中 2/2");
        app.receive(
            Message::Finished {
                processed: 2,
                restart: false,
                refresh: vec![],
            },
            2,
        );
        assert!(app.footer_message().is_empty());
        app.states[last] = Check::new(Status::Done);
        assert!(app.command(&Command::Restore(last)));
        app.receive(Message::State(last, Check::new(Status::Ready), 3), 3);
        assert_eq!(app.footer_message(), "执行中 1/1");
        Ok(())
    }
    #[test]
    fn normal_window_keeps_action_labels_and_restart_hint_visible() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        for zh in [true, false] {
            let mut app = App::new(catalog()?, zh)?;
            app.states.fill(Check::new(Status::Done));
            app.restart = true;
            for width in [92, 100, 120] {
                let mut terminal = Terminal::new(TestBackend::new(width, 24))?;
                terminal.draw(|frame| app.draw(frame))?;
                let buttons = app.toolbar(Rect::new(1, 23, width - 2, 1));
                assert!(
                    buttons
                        .iter()
                        .all(|(_, label, rect, _)| { label.contains(' ') && rect.right() < width })
                );
                let first = buttons[0].2.x;
                let message: String = (1..first)
                    .map(|x| terminal.backend().buffer()[(x, 23)].symbol())
                    .collect();
                assert_eq!(
                    message
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>(),
                    choose(zh, "需要重启", "Restartrequired")
                );
            }
        }
        Ok(())
    }
    #[test]
    fn selected_description_contains_only_the_purpose_in_every_state() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        let compact = |text: &str| {
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        };
        for zh in [true, false] {
            let mut app = App::new(catalog()?, zh)?;
            app.states.fill(Check::new(Status::Done));
            let i = app
                .catalog
                .iter()
                .position(|f| f.id == "open-file-warning")
                .unwrap();
            app.focus(i);
            let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
            for state in [
                Status::Ready,
                Status::Done,
                Status::Failed,
                Status::Restart,
                Status::Restricted,
            ] {
                app.states[i] = Check {
                    state,
                    detail: "Internal diagnostic output".into(),
                };
                terminal.draw(|frame| app.draw(frame))?;
                let buffer = terminal.backend().buffer();
                assert!((1..79).all(|x| buffer[(x, 1)].symbol() == " "));
                let description: String = (app.table.table_area.bottom() + 1..23)
                    .flat_map(|y| (1..79).map(move |x| buffer[(x, y)].symbol()))
                    .collect();
                assert_eq!(compact(&description), compact(app.catalog[i].purpose(zh)));
                if state == Status::Done {
                    assert_eq!(
                        app.states[i].label(&app.catalog[i], zh),
                        choose(zh, "已完成", "Done")
                    );
                }
            }
        }
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
                let footer: String = (area.bottom() + 1..30)
                    .flat_map(|y| (1..99).map(move |x| buffer[(x, y)].symbol()))
                    .collect();
                assert!(
                    without_spaces(&footer).contains(&without_spaces(app.catalog[0].purpose(zh)))
                );
                assert_eq!(
                    buffer[(area.x + 14, app.table.header_area.y)].bg,
                    palette.base
                );
                assert_eq!(buffer[(area.x + 14, area.y)].bg, palette.surface);
                assert_eq!(buffer[(area.x + 14, area.y + 1)].bg, palette.base);
                app.focus(app.catalog.iter().position(|f| f.id == "mouse").unwrap());
                terminal.backend_mut().resize(100, 24);
                terminal.draw(|frame| app.draw(frame))?;
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
                assert!(without_spaces(&footer).contains(&without_spaces(
                    app.catalog[app.selected().unwrap()].purpose(zh)
                )));
            }
        }
        Ok(())
    }
    #[test]
    fn parallel_execution_keeps_selection_and_order_when_an_item_finishes() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.states.fill(Check::new(Status::Ready));
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
    fn only_safe_mode_has_an_automatic_restart_countdown() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        for state in [Status::Ready, Status::Done, Status::Unknown, Status::Failed] {
            app.states[0] = Check::new(state);
            app.receive(
                Message::Finished {
                    processed: 1,
                    restart: true,
                    refresh: vec![],
                },
                0,
            );
            assert!(app.countdown.is_none());
            assert!(app.restart);
        }
        let mut engine = Engine::new()?;
        engine.safe = true;
        app.engine = Some(Arc::new(engine));
        app.receive(
            Message::Finished {
                processed: 1,
                restart: true,
                refresh: vec![],
            },
            0,
        );
        assert!(app.countdown.is_some());
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                refresh: vec![],
            },
            0,
        );
        assert!(app.countdown.is_none());
        Ok(())
    }
    #[test]
    fn stale_detection_cannot_replace_queued_or_running_status() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
        app.states.fill(Check::new(Status::Ready));
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
        app.states.fill(Check::new(Status::Done));
        let mut terminal = Terminal::new(TestBackend::new(100, 24))?;
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                refresh: vec![],
            },
            0,
        );
        let i = app
            .catalog
            .iter()
            .position(|f| f.id == "search-web")
            .unwrap();
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
