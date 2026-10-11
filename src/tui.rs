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
    button::{self, Button, ButtonState},
    event::{ButtonOutcome, TabbedOutcome},
    scrolled::{Scroll, ScrollbarPolicy},
    tabbed::{self, Tabbed, TabbedState},
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
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

const THEME_KEY: &str = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
#[cfg(test)]
use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
#[cfg(test)]
use ratatui::text::Span;

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
    categories: Vec<usize>,
    category: usize,
    tables: Vec<TableState<RowSelection>>,
    tabs: TabbedState,
    buttons: [ButtonState; 5],
    follow_top: bool,
    palette: Palette,
}
struct DetailSection {
    label: &'static str,
    text: String,
    color: Color,
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
            catalog,
            visible: vec![],
            table: TableState::default(),
            zh,
            busy: false,
            message: String::new(),
            engine: None,
            countdown: None,
            restart: false,
            tables: (0..categories.len())
                .map(|_| {
                    let mut table = TableState::<RowSelection>::default();
                    table.selection.set_scroll_selected(true);
                    table.select(Some(0));
                    table
                })
                .collect(),
            categories,
            category: 0,
            tabs: TabbedState::default(),
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
        Command::Items(self.visible.clone())
    }
    fn key_command(&self, key: KeyCode) -> Option<Command> {
        if self.busy {
            return None;
        }
        match key {
            KeyCode::Char('a' | 'A') if self.states.iter().any(Check::actionable) => {
                Some(Command::All)
            }
            KeyCode::Char('c' | 'C')
                if self.visible.iter().any(|i| self.states[*i].actionable()) =>
            {
                Some(self.category_command())
            }
            KeyCode::Char(' ') => self.space_command(),
            KeyCode::Char('r' | 'R') if self.restart => Some(Command::Restart),
            _ => None,
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
                Command::Items(indices) => indices.contains(&i),
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
        let target = match event {
            Event::Key(key) => match key.code {
                KeyCode::Left | KeyCode::BackTab => {
                    Some((self.category + self.categories.len() - 1) % self.categories.len())
                }
                KeyCode::Right | KeyCode::Tab => Some((self.category + 1) % self.categories.len()),
                _ => None,
            },
            Event::Mouse(_) => match tabbed::handle_mouse_events(&mut self.tabs, event) {
                TabbedOutcome::Select(category) => Some(category),
                TabbedOutcome::Changed => return true,
                _ => None,
            },
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
    fn detail_sections(&self) -> Vec<DetailSection> {
        let Some(i) = self.selected() else {
            return vec![];
        };
        let f = &self.catalog[i];
        let state = &self.states[i];
        let mut sections = vec![DetailSection {
            label: choose(self.zh, "功能", "Purpose"),
            text: f.purpose(self.zh).into(),
            color: self.palette.text,
        }];
        if !f.impact(self.zh).is_empty() {
            sections.push(DetailSection {
                label: choose(self.zh, "影响", "Effect"),
                text: f.impact(self.zh).into(),
                color: self.palette.muted,
            });
        }
        let mut summary = state.summary(f, self.zh).to_owned();
        if !state.detail.is_empty()
            && !matches!(
                state.state,
                Status::Unknown | Status::Restricted | Status::Failed
            )
        {
            if !summary.is_empty() {
                summary.push('\n');
            }
            if state.state == Status::SignIn && f.toggle() {
                summary.push_str(choose(self.zh, "目标菜单：", "Target menu: "));
            }
            summary.push_str(&state.detail);
        }
        sections.push(DetailSection {
            label: choose(self.zh, "状态", "Status"),
            text: summary,
            color: self.palette.status(state.state),
        });
        if state.state == Status::Failed && !state.detail.is_empty() {
            sections.push(DetailSection {
                label: choose(self.zh, "错误", "Error"),
                text: state.detail.clone(),
                color: self.palette.red,
            });
        }
        if !self.busy
            && let Some(instruction) = state.instruction(f, self.zh)
        {
            sections.push(DetailSection {
                label: choose(self.zh, "操作", "Action"),
                text: instruction.into(),
                color: self.palette.blue,
            });
        }
        if matches!(
            state.state,
            Status::Unknown | Status::Restricted | Status::Failed
        ) {
            sections.push(DetailSection {
                label: choose(self.zh, "日志", "Log"),
                text: store::root()
                    .join(if state.state != Status::Failed {
                        "checks.jsonl"
                    } else {
                        "operations.jsonl"
                    })
                    .display()
                    .to_string(),
                color: self.palette.muted,
            });
        }
        sections
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
        let buttons = self.toolbar(area);
        let toolbar_height = buttons
            .iter()
            .map(|(_, _, rect, _)| rect.bottom() - area.y)
            .max()
            .unwrap_or(0);
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
                choose(self.zh, "排队中", "Queued")
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
        let mut lines = vec![];
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
        let header_height = header.line_count(area.width) as u16;
        let top = toolbar_height + header_height;
        let sections = self.detail_sections();
        let label_width = if self.zh { 6 } else { 8 };
        let detail_width = area.width.saturating_sub(label_width);
        let mut heights: Vec<_> = sections
            .iter()
            .map(|section| {
                Paragraph::new(section.text.as_str())
                    .wrap(Wrap { trim: true })
                    .line_count(detail_width) as u16
            })
            .collect();
        let available = area.height.saturating_sub(top + 6);
        let mut shortened_error = None;
        if let Some(error) = sections
            .iter()
            .position(|s| s.label == choose(self.zh, "错误", "Error"))
        {
            let others = heights.iter().sum::<u16>() - heights[error];
            let room = available.saturating_sub(others).max(1);
            if heights[error] > room {
                heights[error] = room;
                shortened_error = Some(error);
            }
        }
        let bottom = if sections.is_empty() {
            0
        } else {
            1 + heights.iter().sum::<u16>()
        };
        let bottom = bottom.min(area.height.saturating_sub(top + 5));
        let layout = Layout::vertical([
            Constraint::Length(top),
            Constraint::Min(4),
            Constraint::Length(bottom),
        ])
        .split(area);
        for (index, label, rect, enabled) in buttons {
            let foreground = if enabled { p.blue } else { p.muted };
            frame.render_stateful_widget(
                Button::new(label)
                    .style(style.fg(foreground))
                    .hover_style(style.fg(p.blue).bg(p.surface))
                    .armed_style(style.fg(p.base).bg(p.blue))
                    .block(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .border_style(style.fg(if enabled { p.blue } else { p.border })),
                    ),
                rect,
                &mut self.buttons[index],
            );
        }
        frame.render_widget(
            header,
            Rect::new(area.x, area.y + toolbar_height, area.width, header_height),
        );
        self.tabs.select(Some(self.category));
        frame.render_stateful_widget(
            Tabbed::new()
                .tabs(
                    self.categories
                        .iter()
                        .map(|i| self.catalog[*i].category(self.zh)),
                )
                .style(style.fg(p.border))
                .tab_style(style.fg(p.muted))
                .hover_style(style.fg(p.blue))
                .select_style(style.fg(p.blue).bg(p.surface).add_modifier(Modifier::BOLD)),
            layout[1],
            &mut self.tabs,
        );
        let table_area = self.tabs.widget_area;
        let rows = self.visible.iter().map(|i| {
            let f = &self.catalog[*i];
            Row::new([
                Cell::from(f.name(self.zh).to_owned()),
                Cell::from(self.states[*i].label(f, self.zh))
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
                .style(Some(
                    Style::default()
                        .fg(p.muted)
                        .bg(p.base)
                        .add_modifier(Modifier::BOLD),
                )),
            )
            .widths([Constraint::Fill(1), Constraint::Length(status)])
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
        frame.render_stateful_widget(&table, table_area, &mut self.table);
        if self.table.scroll_to_selected() {
            frame.render_stateful_widget(&table, table_area, &mut self.table);
        }
        if bottom != 0 {
            frame.render_widget(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(p.border)),
                layout[2],
            );
            let mut y = layout[2].y + 1;
            for (index, (section, height)) in sections.iter().zip(heights).enumerate() {
                let height = height.min(layout[2].bottom().saturating_sub(y));
                frame.render_widget(
                    Paragraph::new(section.label)
                        .style(Style::default().fg(p.muted).add_modifier(Modifier::BOLD)),
                    Rect::new(area.x, y, label_width, height.min(1)),
                );
                frame.render_widget(
                    Paragraph::new(section.text.as_str())
                        .wrap(Wrap { trim: true })
                        .style(Style::default().fg(section.color)),
                    Rect::new(area.x + label_width, y, detail_width, height),
                );
                if shortened_error == Some(index) && height != 0 {
                    let line = Rect::new(area.x + label_width, y + height - 1, detail_width, 1);
                    frame.render_widget(Clear, line);
                    frame.render_widget(Block::default().style(style), line);
                    frame.render_widget(
                        Paragraph::new(choose(
                            self.zh,
                            "…完整错误见日志。",
                            "…Full error details are in the log.",
                        ))
                        .style(Style::default().fg(p.muted)),
                        line,
                    );
                }
                y += height;
            }
        }
    }
    fn button_specs(&self) -> Vec<(usize, KeyCode, String, bool)> {
        let item = if self
            .selected()
            .is_some_and(|i| self.states[i].recheckable())
        {
            choose(self.zh, "重新检测", "Check again")
        } else if self.selected().is_some_and(|i| self.catalog[i].toggle()) {
            choose(self.zh, "切换菜单样式", "Switch menu")
        } else {
            choose(self.zh, "执行当前项", "Run item")
        };
        [
            (
                0,
                KeyCode::Char(' '),
                format!("{item}  Space"),
                self.space_command().is_some(),
            ),
            (
                1,
                KeyCode::Char('c'),
                choose(self.zh, "执行本类别  C", "Run category  C").into(),
                self.key_command(KeyCode::Char('c')).is_some(),
            ),
            (
                2,
                KeyCode::Char('a'),
                choose(self.zh, "执行全部类别  A", "Run all categories  A").into(),
                self.key_command(KeyCode::Char('a')).is_some(),
            ),
            (
                3,
                KeyCode::Char('r'),
                choose(self.zh, "重启  R", "Restart  R").into(),
                self.restart,
            ),
            (
                4,
                KeyCode::Char('q'),
                choose(self.zh, "退出  Q", "Exit  Q").into(),
                true,
            ),
        ]
        .into_iter()
        .filter(|(i, _, _, _)| *i != 3 || self.restart)
        .map(|(i, key, label, enabled)| (i, key, label, enabled && !self.busy))
        .collect()
    }
    fn toolbar(&self, area: Rect) -> Vec<(usize, String, Rect, bool)> {
        let mut x = area.x;
        let mut y = area.y;
        self.button_specs()
            .into_iter()
            .map(|(i, _, label, enabled)| {
                let width = (Line::from(label.as_str()).width() as u16 + 4).min(area.width);
                if x > area.x && x + width > area.right() {
                    x = area.x;
                    y += 3;
                }
                let rect = Rect::new(x, y, width, 3);
                x += width + 1;
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
                self.countdown = None;
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
                    .states
                    .iter()
                    .filter(|s| s.state == Status::Failed)
                    .count();
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
                        " · {failures} {}",
                        choose(self.zh, "项执行未完成", "incomplete")
                    ))
                }
                let unconfirmed = self.states.iter().filter(|s| s.recheckable()).count();
                let skipped = self
                    .states
                    .iter()
                    .filter(|s| s.state == Status::Restricted)
                    .count();
                if skipped > 0 {
                    self.message.push_str(&format!(
                        " · {skipped} {}",
                        choose(
                            self.zh,
                            "项受系统限制，已跳过",
                            "skipped due to system restrictions"
                        )
                    ));
                }
                if unconfirmed > 0 {
                    self.message.push_str(&format!(
                        " · {unconfirmed} {}",
                        choose(self.zh, "项状态待确认", "unconfirmed")
                    ));
                }
                if auto && (failures == 0 && unconfirmed == 0 || native::safe_mode()) {
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
            if matches!(key.code, KeyCode::Char('q' | 'Q') | KeyCode::Esc) && !app.busy {
                return Ok(());
            }
            let command = app.key_command(key.code);
            if let Some(command) = command {
                app.command(&command);
                commands.send(command)?;
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
            if let Some(command) = app.key_command(key) {
                app.command(&command);
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
    use super::*;
    #[test]
    fn category_execution_is_scoped_and_all_execution_crosses_tabs() -> Result<()> {
        let mut app = App::new(catalog()?, true)?;
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
            for key in ['c', 'C'] {
                assert!(
                    matches!(app.key_command(KeyCode::Char(key)), Some(Command::Items(indices)) if indices == app.visible)
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
                    Status::PendingCheck
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
        assert!(
            app.states
                .iter()
                .all(|s| matches!(s.state, Status::Queued | Status::Done))
        );
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
        let rect = app.tabs.tab_title_areas[target];
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
                assert_eq!(app.tabs.selected(), Some(category));
                assert!(app.tabs.tab_title_areas[category].width > 0);
                assert!(app.navigate(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))));
            }
            assert_eq!(app.category, 0);
        }
        Ok(())
    }
    #[test]
    fn category_button_click_runs_only_its_items_and_is_disabled_while_busy() -> Result<()> {
        use crossterm::event::MouseEvent;
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = App::new(catalog()?, true)?;
        let mut terminal = Terminal::new(TestBackend::new(120, 24))?;
        terminal.draw(|frame| app.draw(frame))?;
        let area = app.buttons[1].area;
        let mouse = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                column: area.x + 1,
                row: area.y + 1,
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
                    Status::PendingCheck
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
    fn selected_item_details_are_separate_and_fit_a_small_terminal() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let mut previews = vec![];
        for zh in [true, false] {
            for (id, state, detail) in [
                (
                    "process-mitigations",
                    Status::Done,
                    choose(
                        zh,
                        "启动配置已关闭。仍启用的防护（数量为已读取的 Windows 进程数）：\n数据执行保护 DEP：170；地址随机化 ASLR：8\n动态代码限制：3；严格句柄检查：71\n系统调用限制：1；扩展点限制：2\n控制流保护 CFG：1；代码签名限制：5\n字体加载限制：3；映像加载限制：4\n子进程限制：3；堆栈保护 CET：53\n重定向信任检查：12；异常处理链保护：170",
                        "Startup settings are off. Retained protections (counts are inspected Windows processes):\nDEP: 170; ASLR: 8\nDynamic code: 3; Strict handle checks: 71\nWin32k restrictions: 1; Extension point restrictions: 2\nControl flow guard: 1; Code signing restrictions: 5\nFont restrictions: 3; Image loading restrictions: 4\nChild process restrictions: 3; Stack protection CET: 53\nRedirection trust checks: 12; Exception chain protection: 170",
                    ),
                ),
                (
                    "dep",
                    Status::Done,
                    choose(
                        zh,
                        "启动策略已关闭；当前 64 位进程的 DEP 仍开启。",
                        "The boot policy is off; DEP remains enabled for the current 64-bit process.",
                    ),
                ),
                (
                    "windows-update",
                    Status::Done,
                    choose(
                        zh,
                        "更新暂停至 2045-12-06",
                        "Updates paused until 2045-12-06",
                    ),
                ),
                (
                    "svc-sysmain",
                    Status::Done,
                    choose(
                        zh,
                        "当前内存压缩：开启；内存页合并：开启。",
                        "Memory compression: on; page combining: on.",
                    ),
                ),
                ("classic-menu", Status::SignIn, "Windows 10"),
                ("install-git.git", Status::Queued, ""),
            ] {
                for width in [80, 120] {
                    let mut app = App::new(catalog()?, zh)?;
                    let i = app.catalog.iter().position(|f| f.id == id).unwrap();
                    let mut terminal = Terminal::new(TestBackend::new(width, 24))?;
                    terminal.draw(|frame| app.draw(frame))?;
                    app.focus(i);
                    app.states[i] = Check {
                        state,
                        detail: detail.into(),
                    };
                    if state == Status::Done && app.catalog[i].security() {
                        assert_eq!(
                            app.states[i].label(&app.catalog[i], zh),
                            choose(zh, "已优化", "Optimized")
                        );
                        assert!(app.space_command().is_none());
                    }
                    let sections = app.detail_sections();
                    assert_eq!(sections[0].label, choose(zh, "功能", "Purpose"));
                    assert!(
                        sections
                            .iter()
                            .any(|s| s.label == choose(zh, "状态", "Status"))
                    );
                    assert!(
                        !app.states[i].label(&app.catalog[i], zh).contains(detail)
                            || detail.is_empty()
                    );
                    terminal.draw(|frame| app.draw(frame))?;
                    let row = app.table.selected().unwrap();
                    assert!(
                        row >= app.table.row_offset()
                            && row < app.table.row_offset() + app.table.vscroll.page_len()
                    );
                    let buffer = terminal.backend().buffer();
                    let label_width: u16 = if zh { 6 } else { 8 };
                    let rows: Vec<_> = (0..24).map(|y| {
                        let mut remaining = 0;
                        (0..width).map(|x| {
                            let cell = &buffer[(x,y)];
                            let text = if remaining != 0 { remaining -= 1; "" } else {
                                remaining = Span::raw(cell.symbol()).width().saturating_sub(1);
                                cell.symbol()
                            };
                            serde_json::json!({"text":text,"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg)})
                        }).collect::<Vec<_>>()
                    }).collect();
                    let footer: String = rows[usize::from(app.table.table_area.bottom() + 1)..]
                        .iter()
                        .flat_map(|row| {
                            row[usize::from(1 + label_width)..usize::from(width - 1)]
                                .iter()
                                .map(|cell| cell["text"].as_str().unwrap())
                        })
                        .collect();
                    for section in sections {
                        assert!(
                            compact(&footer).contains(&compact(&section.text)),
                            "{id}, width {width}, {}: {:?}",
                            section.label,
                            footer
                        );
                    }
                    if width == 80 {
                        previews.push(serde_json::json!({"id":id,"zh":zh,"width":width,"height":24,"rows": rows}));
                    }
                }
            }
        }
        if let Ok(path) = std::env::var("ZSW_UI_PREVIEWS") {
            std::fs::write(path, serde_json::to_string(&previews)?)?;
        }
        Ok(())
    }
    #[test]
    fn long_errors_keep_the_retry_action_and_log_visible() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        for zh in [true, false] {
            let mut app = App::new(catalog()?, zh)?;
            let i = app
                .catalog
                .iter()
                .position(|f| f.id == "install-git.git")
                .unwrap();
            let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
            terminal.draw(|frame| app.draw(frame))?;
            app.focus(i);
            app.states[i] = Check {
                state: Status::Failed,
                detail: "Installer returned an error with detailed output.\n".repeat(30),
            };
            terminal.draw(|frame| app.draw(frame))?;
            let buffer = terminal.backend().buffer();
            let label_width = if zh { 6 } else { 8 };
            let mut footer = String::new();
            for y in app.table.table_area.bottom() + 1..24 {
                let mut x = 1 + label_width;
                while x < 79 {
                    let symbol = buffer[(x, y)].symbol();
                    footer.push_str(symbol);
                    x += Span::raw(symbol).width().max(1) as u16;
                }
            }
            let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            assert!(compact(&footer).contains(&compact(
                app.states[i].instruction(&app.catalog[i], zh).unwrap()
            )));
            assert!(compact(&footer).contains("operations.jsonl"));
            assert!(compact(&footer).contains(&compact(choose(
                zh,
                "完整错误见日志",
                "Full error details are in the log"
            ))));
        }
        Ok(())
    }
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
                (Status::Ready, choose(zh, "可清理", "Can clean up")),
                (Status::Done, choose(zh, "无固定项", "No pins")),
                (Status::SignIn, choose(zh, "待重新登录", "Needs sign-in")),
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
        app.states[i] = Check::new(Status::Unknown);
        let command = app.space_command().unwrap();
        assert!(matches!(&command, Command::Scan(indices) if indices == &[i]));
        app.command(&command);
        assert!(!app.busy);
        assert_eq!(app.selected(), Some(i));
        assert_eq!(app.states[i].state, Status::PendingCheck);
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
                assert!(
                    (app.table.table_area.bottom() + 1..24)
                        .any(|y| (1..89).any(|x| buffer[(x, y)].fg == palette.red))
                );
            }
        }
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
                let label_width = if zh { 6 } else { 8 };
                let footer: String = (area.bottom() + 1..30)
                    .flat_map(|y| (1 + label_width..99).map(move |x| buffer[(x, y)].symbol()))
                    .collect();
                assert!(
                    without_spaces(&footer).contains(&without_spaces(app.catalog[0].purpose(zh)))
                );
                assert!(
                    without_spaces(&footer)
                        .contains(&without_spaces(app.states[0].summary(&app.catalog[0], zh)))
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
                    .flat_map(|y| (1 + label_width..54).map(move |x| buffer[(x, y)].symbol()))
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
        app.states[0] = Check::new(Status::Unknown);
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                auto: true,
                refresh: vec![],
            },
            0,
        );
        assert!(app.countdown.is_none());
        assert!(app.message.contains("1 项状态待确认"));
        app.states[0] = Check::new(Status::Restricted);
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                auto: true,
                refresh: vec![],
            },
            0,
        );
        assert!(app.countdown.is_some());
        assert!(app.message.contains("1 项受系统限制，已跳过"));
        app.states[0] = Check::new(Status::Failed);
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                auto: true,
                refresh: vec![],
            },
            0,
        );
        assert!(app.countdown.is_none());
        assert!(app.message.contains("1 项执行未完成"));
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
        let mut terminal = Terminal::new(TestBackend::new(100, 24))?;
        app.receive(
            Message::Finished {
                processed: 1,
                restart: false,
                auto: false,
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
