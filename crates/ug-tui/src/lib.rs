//! `ug tui`: Unix Guardian in the terminal.
//!
//! The approved design (docs/tui-design.md): the same pages as the window, in
//! the same order; three fixed zones (menu, page, and a key bar that always
//! says what can be pressed now); every action by mouse and by keyboard;
//! dangerous actions confirmed twice; no animation, and nothing refreshed
//! that is not on screen. English by default (terminals render Persian
//! unevenly); Persian is a later option for terminals known to handle it.

mod data;
mod pages;
mod theme;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::{execute, terminal};
use data::{Bus, Cached};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use ug_proto::{
    AppTraffic, BatteryCare, FpsSession, OverlayState, DayTraffic, ExtraSample, FpsReading, HistoryPoint, NetControlStatus, NetRule,
    PingComparison, PingReport, InstalledGameInfo, Inventory, PowerCapability, PowerJob, PowerWatch,
    ProfileInfo, RunningGameInfo, Snapshot,
};

use theme::Theme;

/// Menu entries, in the window's order: (name, short name for narrow screens, built).
pub const PAGES: [(&str, &str, bool); 12] = [
    ("Dashboard", "Dash", true),
    ("Game mode", "Game", true),
    ("System control", "System", true),
    ("Monitoring", "Monitor", true),
    ("Ping stability", "Ping", true),
    ("Overlay", "Overlay", true),
    ("Playtime", "Play", true),
    ("Recorder", "Rec", false),
    ("Drivers", "Drivers", true),
    ("Bug report", "Bugs", false),
    ("Updates", "Updates", false),
    ("Settings", "Settings", true),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Menu,
    Page,
}

/// Something the mouse can hit, recorded while drawing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Menu(usize),
    Tab(usize),
    Row(usize),
    /// A secondary choice on the page (profile, range, "when"): its index.
    Choice(usize),
}

pub struct App {
    pub bus: Bus,
    pub theme: Theme,
    pub page: usize,
    pub focus: Focus,
    /// Tab within the page.
    pub tab: usize,
    /// Selected row within the page.
    pub row: usize,
    /// A page-specific choice (profile index, history range, "when").
    pub choice: usize,
    pub inventory: Cached<Inventory>,
    pub snapshot: Cached<Snapshot>,
    pub gamemode: Cached<bool>,
    /// (seconds, CPU °C, working GPU °C) for the live temperature chart.
    pub temps: VecDeque<(f64, f64, f64)>,
    pub running: Cached<Vec<RunningGameInfo>>,
    pub profiles: Cached<Vec<ProfileInfo>>,
    pub installed: Cached<Vec<InstalledGameInfo>>,
    pub caps: Cached<Vec<PowerCapability>>,
    pub jobs: Cached<(Vec<PowerJob>, Vec<PowerWatch>)>,
    pub battery: Cached<BatteryCare>,
    pub history: Cached<Vec<HistoryPoint>>,
    pub traffic: Cached<Vec<DayTraffic>>,
    /// The live monitor's last two minutes: one snapshot and one extra
    /// reading a second, only while it is on screen.
    pub live: VecDeque<(Snapshot, ExtraSample)>,
    pub live_due: Cached<()>,
    pub fps: Cached<FpsReading>,
    pub ping: Cached<PingReport>,
    pub ping_compare: Option<(u64, PingComparison)>,
    pub net: Cached<(Vec<AppTraffic>, Vec<NetRule>, NetControlStatus)>,
    pub overlay: Cached<OverlayState>,
    pub shortcuts: Cached<ug_proto::ShortcutsState>,
    /// Recording new keys for a global shortcut: its id, and Meta added by
    /// hand (most terminals never see the Meta key itself).
    pub recording: Option<(String, bool)>,
    pub settings: Cached<ug_proto::SettingsState>,
    /// The driver check (files only, run here) and the service's changes.
    pub driver: Cached<ug_proto::DriverReport>,
    pub driver_ops: Cached<Vec<ug_proto::DriverOp>>,
    /// The fix being reviewed: its id and plan.
    pub driver_review: Option<(String, Result<ug_proto::DriverPlan, String>)>,
    pub tray: Cached<bool>,
    pub daemon_version: Cached<String>,
    pub cooling: Cached<(ug_proto::CoolingState, String)>,
    /// The dashboard's card switch: the GPUs (boot one first) and the GPU
    /// programs start on. Per-user files and sysfs only.
    pub gpu_switch: Cached<(Vec<ug_hw::gpu_use::Gpu>, ug_core::gpu_default::State)>,
    /// The Playtime page's list for the chosen period (games and apps).
    pub playtime: Cached<Vec<ug_proto::PlaytimeEntry>>,
    /// Tracked apps and what Steam imports added, read with `playtime`.
    pub playtime_extra: Cached<(Vec<ug_proto::TrackedApp>, Vec<ug_proto::PlaytimeEntry>)>,
    /// Programs open now that could be tracked, while the Apps tab is open.
    pub programs: Cached<Vec<ug_proto::TrackedApp>>,
    /// The Steam game whose launch options are being edited, and the choice.
    pub launch: Option<pages::LaunchEdit>,
    /// Re-reads the launch options (is Steam still running?) while shown.
    pub launch_due: Cached<()>,
    /// GPU tuning on the game mode page: the card, its state (read only on
    /// request: it wakes the card), the draft, and the trial's deadline.
    pub tune: Option<(String, ug_proto::GpuTuneState)>,
    pub tune_draft: ug_proto::GpuTune,
    pub tune_until: Option<Instant>,
    pub sessions: Cached<Vec<FpsSession>>,
    /// A dangerous action armed by the first press; confirmed by a second
    /// within five seconds.
    pub armed: Option<(String, Instant)>,
    /// A one-line result or error shown above the key bar.
    pub notice: Option<(String, bool, Instant)>,
    pub hits: Vec<(Rect, Hit)>,
    /// Row count of the current page's list, set while drawing.
    pub rows: usize,
    pub help: bool,
    quit: bool,
    open_window: bool,
}

const LIVE: Duration = Duration::from_secs(1);
const SLOW: Duration = Duration::from_secs(5);
const RARE: Duration = Duration::from_secs(60);

impl App {
    fn new(bus: Bus) -> App {
        App {
            bus,
            theme: Theme::detect(),
            page: 0,
            focus: Focus::Menu,
            tab: 0,
            row: 0,
            choice: 0,
            inventory: Cached::new(Duration::from_secs(600)),
            snapshot: Cached::new(LIVE),
            gamemode: Cached::new(SLOW),
            temps: VecDeque::new(),
            running: Cached::new(SLOW),
            profiles: Cached::new(SLOW),
            installed: Cached::new(RARE),
            caps: Cached::new(RARE),
            jobs: Cached::new(SLOW),
            battery: Cached::new(Duration::from_secs(15)),
            history: Cached::new(RARE),
            traffic: Cached::new(RARE),
            live: VecDeque::new(),
            live_due: Cached::new(LIVE),
            fps: Cached::new(LIVE),
            ping: Cached::new(LIVE),
            ping_compare: None,
            net: Cached::new(Duration::from_secs(2)),
            overlay: Cached::new(SLOW),
            // Fast enough to follow the desktop's confirmation dialog.
            shortcuts: Cached::new(Duration::from_secs(1)),
            recording: None,
            settings: Cached::new(SLOW),
            driver: Cached::new(Duration::from_secs(3600)),
            driver_ops: Cached::new(Duration::from_secs(1)),
            driver_review: None,
            tray: Cached::new(SLOW),
            daemon_version: Cached::new(Duration::from_secs(3600)),
            gpu_switch: Cached::new(SLOW),
            // A running game's time moves by the agent's minute heartbeat.
            playtime: Cached::new(RARE),
            playtime_extra: Cached::new(RARE),
            programs: Cached::new(Duration::from_secs(10)),
            launch: None,
            launch_due: Cached::new(SLOW),
            cooling: Cached::new(Duration::from_secs(3)),
            tune: None,
            tune_draft: Default::default(),
            tune_until: None,
            sessions: Cached::new(Duration::from_secs(30)),
            armed: None,
            notice: None,
            hits: Vec::new(),
            rows: 0,
            help: false,
            quit: false,
            open_window: false,
        }
    }

    pub fn say(&mut self, text: impl Into<String>, ok: bool) {
        self.notice = Some((text.into(), ok, Instant::now()));
    }

    /// Refreshes what the current page shows, each piece at its own rate.
    fn refresh(&mut self) {
        if self.inventory.due() {
            let r = self.bus.inventory();
            self.inventory.store(r);
        }
        match self.page {
            0 => {
                if self.snapshot.due() {
                    let r = self.bus.snapshot();
                    if let Ok(s) = &r {
                        let t = s.ts_unix_ms as f64 / 1000.0;
                        let gpu = s.gpus.iter().find(|g| !g.asleep && !g.temp_c.is_nan()).map(|g| g.temp_c).unwrap_or(f64::NAN);
                        self.temps.push_back((t, s.cpu.temp_c, gpu));
                        while self.temps.len() > 120 {
                            self.temps.pop_front();
                        }
                    }
                    self.snapshot.store(r);
                }
                if self.gpu_switch.due() {
                    let gpus = ug_hw::gpu_use::gpus(&ug_hw::SysRoot::real());
                    let st = ug_core::gpu_default::state(&ug_core::gpu_default::config_home(), &gpus);
                    self.gpu_switch.store(Ok((gpus, st)));
                }
                self.refresh_games();
            }
            1 => {
                self.refresh_games();
                if self.installed.due() {
                    let r = self.bus.installed();
                    self.installed.store(r);
                }
                if self.tab == 2 {
                    pages::refresh_launch(self);
                }
            }
            2 => {
                if self.caps.due() {
                    let r = self.bus.power_caps();
                    self.caps.store(r);
                }
                if self.jobs.due() {
                    let r = self.bus.power_jobs();
                    self.jobs.store(r);
                }
                if self.battery.due() {
                    let r = self.bus.battery();
                    self.battery.store(r);
                }
                if self.tab == 3 && self.cooling.due() {
                    let r = self.bus.cooling().map(|c| (c, self.bus.power_profile()));
                    self.cooling.store(r);
                }
            }
            3 => {
                if self.tab == 0 && self.choice == 0 {
                    if self.live_due.due() {
                        match (self.bus.snapshot(), self.bus.extra()) {
                            (Ok(s), Ok(e)) => {
                                self.live.push_back((s, e));
                                while self.live.len() > 120 {
                                    self.live.pop_front();
                                }
                                self.live_due.store(Ok(()));
                            }
                            (Err(e), _) | (_, Err(e)) => self.live_due.store(Err(e)),
                        }
                    }
                } else if self.tab == 0 && self.history.due() {
                    let now = data::now_ms();
                    let since = match self.choice {
                        1 => now.saturating_sub(3_600_000),
                        3 => now.saturating_sub(7 * 86_400_000),
                        _ => {
                            // Today: since local midnight, like the window.
                            let secs_today = unsafe {
                                let t = (now / 1000) as libc::time_t;
                                let mut tm: libc::tm = std::mem::zeroed();
                                libc::localtime_r(&t, &mut tm);
                                (tm.tm_hour * 3600 + tm.tm_min * 60 + tm.tm_sec) as u64
                            };
                            now.saturating_sub(secs_today * 1000)
                        }
                    };
                    // About one point per 5 minutes for a day, per 30 for a week.
                    let points = [60, 60, 288, 336][self.choice.min(3)];
                    let r = self.bus.history(since, now, points);
                    self.history.store(r);
                }
                if self.tab == 2 && self.fps.due() {
                    let r = self.bus.fps();
                    self.fps.store(r);
                }
                if self.tab == 2 && self.sessions.due() {
                    let r = self.bus.fps_sessions(data::now_ms().saturating_sub(7 * 86_400_000));
                    self.sessions.store(r);
                }
                if self.tab == 1 && self.snapshot.due() {
                    let r = self.bus.snapshot();
                    self.snapshot.store(r);
                }
                if self.tab == 1 && self.traffic.due() {
                    let r = self.bus.traffic(data::now_ms().saturating_sub(7 * 86_400_000));
                    self.traffic.store(r);
                }
            }
            5 => {
                if self.tab == 0 && self.overlay.due() {
                    let r = self.bus.overlay();
                    self.overlay.store(r);
                }
                if self.tab == 1 && self.shortcuts.due() {
                    let r = self.bus.shortcuts();
                    self.shortcuts.store(r);
                }
            }
            8 => {
                if self.driver.due() {
                    let mut env = ug_driver::Env::from_process();
                    if env.kernel_log.is_empty() {
                        env.kernel_log = self.bus.firmware_log();
                    }
                    let r = ug_driver::diagnose(&ug_hw::SysRoot::real(), &env);
                    self.driver.store(Ok(r));
                }
                if self.driver_ops.due() {
                    let r = self.bus.driver_ops();
                    // A change just finished: check again.
                    let was_running = self.driver_ops.value.as_ref().is_some_and(|v| v.iter().any(|o| o.state == "running" || o.state == "undoing"));
                    let running = r.as_ref().is_ok_and(|v| v.iter().any(|o| o.state == "running" || o.state == "undoing"));
                    self.driver_ops.store(r);
                    if was_running && !running {
                        self.driver.invalidate();
                    }
                }
            }
            11 => {
                if self.settings.due() {
                    let r = self.bus.settings();
                    self.settings.store(r);
                }
                if self.tray.due() {
                    let r = self.bus.tray_visible();
                    self.tray.store(r);
                }
                if self.shortcuts.due() {
                    let r = self.bus.shortcuts();
                    self.shortcuts.store(r);
                }
                if self.daemon_version.due() {
                    let r = self.bus.daemon_version();
                    self.daemon_version.store(r);
                }
            }
            4 => {
                if self.tab == 0 && self.ping.due() {
                    let r = self.bus.ping(data::now_ms().saturating_sub(5 * 60_000));
                    if let Ok(rep) = &r {
                        // Compare around the last time game mode went on, once per mark.
                        let mark = rep.marks.iter().rev().find(|m| m.kind == "gamemode-on").map(|m| m.ts_unix_ms);
                        let far = rep.targets.iter().find(|t| t.id == "game" && !t.addr.is_empty()).or_else(|| rep.targets.iter().find(|t| t.id == "internet"));
                        if let (Some(m), Some(t)) = (mark, far) {
                            if data::now_ms() > m + 10_000 {
                                if let Ok(c) = self.bus.ping_compare(&t.id, m) {
                                    self.ping_compare = Some((m, c));
                                }
                            }
                        }
                    }
                    self.ping.store(r);
                }
                if self.net.due() {
                    let r = self.bus.net();
                    self.net.store(r);
                }
            }
            6 => {
                if self.playtime.due() {
                    let since = ug_core::playtime::since(ug_core::playtime::RANGES[self.choice.min(2)], data::now_ms());
                    let r = self.bus.playtime(since);
                    self.playtime.store(r);
                }
                if self.playtime_extra.due() {
                    let r = self.bus.tracked_apps().and_then(|a| Ok((a, self.bus.playtime_imported()?)));
                    self.playtime_extra.store(r);
                }
                if self.tab == 1 && self.programs.due() {
                    let r = self.bus.running_programs();
                    self.programs.store(r);
                }
            }
            _ => {}
        }
        // A GPU trial ends by itself in the service; show that here too.
        if self.tune_until.is_some_and(|t| t <= Instant::now()) {
            self.tune_until = None;
            if let Some((pci, _)) = self.tune.clone() {
                if let Ok(st) = self.bus.gpu_tune_state(&pci) {
                    self.tune = Some((pci, st));
                }
            }
        }
        if self.armed.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(5)) {
            self.armed = None;
        }
        if self.notice.as_ref().is_some_and(|(_, _, t)| t.elapsed() > Duration::from_secs(6)) {
            self.notice = None;
        }
    }

    fn refresh_games(&mut self) {
        if self.running.due() {
            let r = self.bus.running();
            self.running.store(r);
        }
        if self.profiles.due() {
            let r = self.bus.profiles();
            self.profiles.store(r);
        }
        if self.gamemode.due() {
            let r = self.bus.gamemode_active();
            self.gamemode.store(r);
        }
    }

    fn go(&mut self, page: usize) {
        if page < PAGES.len() && page != self.page {
            self.page = page;
            self.tab = 0;
            self.row = 0;
            // Monitoring and Playtime open on their middle range.
            self.choice = if page == 3 || page == 6 { 1 } else { 0 };
            self.armed = None;
            self.history.invalidate();
        }
    }

    fn tabs(&self) -> usize {
        match self.page {
            1 => 3,
            2 => 4,
            3 => 3,
            4 => 2,
            5 => 2,
            8 => 2,
            6 => 2,
            _ => 1,
        }
    }

    fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        if self.help {
            self.help = false;
            return;
        }
        // The next press is the shortcut's new keys, whatever it is.
        if self.recording.is_some() {
            pages::record_keys(self, code, mods);
            return;
        }
        match code {
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('g') => {
                self.open_window = true;
                self.quit = true;
            }
            KeyCode::Char(c @ '1'..='9') => {
                self.go(c as usize - '1' as usize);
                self.focus = Focus::Page;
            }
            KeyCode::Esc => {
                if self.armed.take().is_none() {
                    if self.focus == Focus::Page {
                        self.focus = Focus::Menu;
                    } else {
                        self.quit = true;
                    }
                }
            }
            _ if self.focus == Focus::Menu => match code {
                KeyCode::Up | KeyCode::Char('k') => self.go(self.page.saturating_sub(1)),
                KeyCode::Down | KeyCode::Char('j') => self.go((self.page + 1).min(PAGES.len() - 1)),
                KeyCode::Enter | KeyCode::Right | KeyCode::Tab => self.focus = Focus::Page,
                _ => {}
            },
            _ => match code {
                KeyCode::Up | KeyCode::Char('k') => self.row = self.row.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.row = (self.row + 1).min(self.rows.saturating_sub(1)),
                KeyCode::Tab => self.switch_tab((self.tab + 1) % self.tabs()),
                KeyCode::BackTab => self.switch_tab((self.tab + self.tabs() - 1) % self.tabs()),
                KeyCode::Left | KeyCode::Char('h') => pages::step_choice(self, -1),
                KeyCode::Right | KeyCode::Char('l') => pages::step_choice(self, 1),
                KeyCode::Enter | KeyCode::Char(' ') => pages::activate(self, self.row),
                KeyCode::Char('x') | KeyCode::Delete => pages::remove(self, self.row),
                KeyCode::Char('b') => pages::block(self, self.row),
                KeyCode::Char('c') if (self.page, self.tab) == (5, 1) => pages::record_shortcut(self, self.row),
                KeyCode::Char('e') if self.page == 6 => pages::export_playtime(self),
                KeyCode::Char('y') if (self.page, self.tab) == (8, 1) => pages::keep_driver_op(self, self.row),
                KeyCode::Char('r') if self.page == 8 => {
                    self.driver.invalidate();
                    self.driver_review = None;
                }
                KeyCode::Char('o') if (self.page, self.tab) == (1, 2) => pages::toggle_launch_overlay(self),
                KeyCode::Char('u') if (self.page, self.tab) == (1, 2) => pages::undo_launch(self),
                KeyCode::Char('n') if self.page == 6 => pages::not_a_game(self, self.row),
                KeyCode::Char('i') if self.page == 6 => pages::import_steam(self, false),
                KeyCode::Char('u') if self.page == 6 => pages::import_steam(self, true),
                _ => {}
            },
        }
    }

    fn switch_tab(&mut self, tab: usize) {
        self.tab = tab;
        self.row = 0;
        self.armed = None;
    }

    fn click(&mut self, x: u16, y: u16) {
        if self.help {
            self.help = false;
            return;
        }
        let pos = Position { x, y };
        let Some(&(_, hit)) = self.hits.iter().rev().find(|(r, _)| r.contains(pos)) else { return };
        match hit {
            Hit::Menu(i) => {
                self.go(i);
                self.focus = Focus::Page;
            }
            Hit::Tab(i) => {
                self.focus = Focus::Page;
                self.switch_tab(i);
            }
            Hit::Row(i) => {
                self.focus = Focus::Page;
                // First click selects, a click on the selected row acts.
                if self.row == i {
                    pages::activate(self, i);
                } else {
                    self.row = i;
                }
            }
            Hit::Choice(i) => {
                self.focus = Focus::Page;
                pages::set_choice(self, i);
            }
        }
    }

    fn scroll(&mut self, down: bool) {
        if self.focus == Focus::Menu {
            self.go(if down { (self.page + 1).min(PAGES.len() - 1) } else { self.page.saturating_sub(1) });
        } else if down {
            self.row = (self.row + 1).min(self.rows.saturating_sub(1));
        } else {
            self.row = self.row.saturating_sub(1);
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        self.hits.clear();
        let area = f.area();
        let t = &self.theme;
        f.render_widget(Block::default().style(Style::default().fg(t.text)), area);
        if area.width < 60 || area.height < 20 {
            f.render_widget(
                Paragraph::new("Unix Guardian needs a terminal of at least 60×20. Make the window bigger, or press q.")
                    .wrap(ratatui::widgets::Wrap { trim: true }),
                area,
            );
            return;
        }
        let wide = area.width >= 100 && area.height >= 26;
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(5), Constraint::Length(1), Constraint::Length(1)])
            .split(area);
        self.draw_header(f, outer[0]);
        let body = outer[1];
        let content = if wide {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(20), Constraint::Min(40)])
                .split(body);
            self.draw_menu(f, cols[0]);
            cols[1]
        } else {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(3)])
                .split(body);
            self.draw_menu_row(f, rows[0]);
            rows[1]
        };
        pages::draw(self, f, content, wide);
        self.draw_notice(f, outer[2]);
        self.draw_keys(f, outer[3]);
        if self.help {
            self.draw_help(f, area);
        }
    }

    fn draw_header(&self, f: &mut Frame, r: Rect) {
        let t = &self.theme;
        let dot = |ok: bool| Span::styled("● ", Style::default().fg(if ok { t.ok } else { t.danger }));
        let service = self.inventory.value.is_some() && self.inventory.error.is_none();
        let agent = self.profiles.error.is_none();
        let left = Line::from(vec![Span::styled(" Unix Guardian", t.accent())]);
        let right = Line::from(vec![dot(service), Span::styled("service  ", t.muted()), dot(agent), Span::styled("agent ", t.muted())])
            .right_aligned();
        f.render_widget(Paragraph::new(left), r);
        f.render_widget(Paragraph::new(right), r);
    }

    fn draw_menu(&mut self, f: &mut Frame, r: Rect) {
        let t = &self.theme;
        let block = Block::default().borders(Borders::RIGHT).border_style(t.border());
        let inner = block.inner(r);
        f.render_widget(block, r);
        for (i, (name, _, built)) in PAGES.iter().enumerate() {
            let y = inner.y + 1 + i as u16;
            if y >= inner.bottom() {
                break;
            }
            let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
            let active = i == self.page;
            let style = if active {
                if self.focus == Focus::Menu { t.selected() } else { t.accent() }
            } else if *built {
                t.muted()
            } else {
                t.faint()
            };
            let marker = if active { "▌" } else { " " };
            f.render_widget(Paragraph::new(Line::from(vec![
                Span::styled(marker, t.accent()),
                Span::styled(format!("{name:<18}"), style),
            ])), row);
            self.hits.push((row, Hit::Menu(i)));
        }
    }

    fn draw_menu_row(&mut self, f: &mut Frame, r: Rect) {
        let t = &self.theme;
        let mut x = r.x + 1;
        for (i, (_, short, built)) in PAGES.iter().enumerate() {
            let w = short.len() as u16 + 2;
            if x + w > r.right() {
                f.render_widget(Paragraph::new(Span::styled("…", t.faint())), Rect { x, y: r.y, width: 1, height: 1 });
                break;
            }
            let rect = Rect { x, y: r.y, width: w, height: 1 };
            let style = if i == self.page { t.selected() } else if *built { t.muted() } else { t.faint() };
            f.render_widget(Paragraph::new(Span::styled(format!(" {short} "), style)), rect);
            self.hits.push((rect, Hit::Menu(i)));
            x += w + 1;
        }
    }

    fn draw_notice(&self, f: &mut Frame, r: Rect) {
        let t = &self.theme;
        if let Some((action, _)) = &self.armed {
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!(" Press Enter again within 5 seconds to {action}. Esc cancels."),
                    Style::default().fg(t.danger),
                )),
                r,
            );
        } else if let Some((text, ok, _)) = &self.notice {
            let mut spans = vec![Span::styled(format!(" {text}"), Style::default().fg(if *ok { t.ok } else { t.danger }))];
            let hint = format!("   A bug? {}", ug_core::BUG_REPORT_EMAIL);
            if !ok && (text.chars().count() + hint.chars().count() + 1) < r.width as usize {
                spans.push(Span::styled(hint, t.faint()));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), r);
        }
    }

    fn draw_keys(&self, f: &mut Frame, r: Rect) {
        let t = &self.theme;
        let keys: Vec<(&str, &str)> = if self.help {
            vec![("any key", "close help")]
        } else if self.recording.is_some() {
            pages::keys(self)
        } else if self.focus == Focus::Menu {
            vec![("↑↓", "menu"), ("Enter", "open"), ("1-9", "jump"), ("?", "help"), ("g", "window"), ("q", "quit")]
        } else {
            let mut k = vec![("↑↓", "move")];
            k.extend(pages::keys(self));
            k.extend([("Esc", "menu"), ("?", "help"), ("q", "quit")]);
            k
        };
        let mut spans = vec![Span::raw(" ")];
        for (key, what) in keys {
            spans.push(Span::styled(key, t.accent()));
            spans.push(Span::styled(format!(" {what}  "), t.muted()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(t.selected_bg)), r);
    }

    fn draw_help(&self, f: &mut Frame, area: Rect) {
        let t = &self.theme;
        let w = 64.min(area.width - 4);
        let h = 20.min(area.height - 2);
        let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
        f.render_widget(ratatui::widgets::Clear, r);
        let lines = vec![
            Line::from(Span::styled("Keys", t.title())),
            Line::from(""),
            Line::from("  ↑ ↓  j k        move in the menu or the page"),
            Line::from("  Enter  → Tab    open the page / next section"),
            Line::from("  ← →  h l        change the choice on the page"),
            Line::from("  Enter  Space    press, toggle, run"),
            Line::from("  x  Delete       remove a scheduled item or a rule"),
            Line::from("  b               block the selected app (Ping page)"),
            Line::from("  1 … 9           jump to a page"),
            Line::from("  Esc             back to the menu, then quit"),
            Line::from("  g               open the graphical window"),
            Line::from("  q  Ctrl-C       quit"),
            Line::from(""),
            Line::from(Span::styled("Mouse: click anything; the wheel scrolls.", t.muted())),
            Line::from(Span::styled("Shut down and restart ask twice.", t.muted())),
            Line::from(Span::styled(format!("Found a bug? {}", ug_core::BUG_REPORT_EMAIL), t.accent())),
        ];
        f.render_widget(
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).border_style(t.accent()).title(" Help ")),
            r,
        );
    }
}

/// What the TUI asks the caller to do when it ends.
pub enum Exit {
    Quit,
    /// `g`: open the graphical window instead.
    OpenWindow,
}

/// Set by SIGTERM (the tray's "Quit Unix Guardian"): the loop then leaves
/// normally, so the terminal is restored. SIGHUP keeps its default (end at
/// once): the terminal is gone, there is nothing to restore, and crossterm
/// spins reading a hung-up tty instead of returning to the loop.
static TERMINATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_terminate(_: libc::c_int) {
    TERMINATED.store(true, std::sync::atomic::Ordering::Relaxed);
    // If the loop cannot get back to the flag (the same spin, if the
    // terminal went away too), SIGALRM's default action ends the process.
    // SAFETY: alarm(2) is async-signal-safe.
    unsafe { libc::alarm(3) };
}

/// Runs the terminal interface until the user quits.
pub fn run(daemon_session: bool) -> anyhow::Result<Exit> {
    let bus = Bus::connect(daemon_session)?;
    let mut app = App::new(bus);
    // SAFETY: the handler only stores to an atomic and calls alarm(2).
    unsafe { libc::signal(libc::SIGTERM, on_terminate as *const () as libc::sighandler_t) };

    terminal::enable_raw_mode()?;
    let mut out = std::io::stdout();
    execute!(out, terminal::EnterAlternateScreen, event::EnableMouseCapture)?;
    // Restore the terminal even if something panics mid-draw.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        hook(info);
    }));
    let backend = ratatui::backend::CrosstermBackend::new(out);
    let mut term = ratatui::Terminal::new(backend)?;

    let result = (|| -> anyhow::Result<()> {
        let mut dirty = true;
        let mut last_tick = Instant::now() - LIVE;
        while !app.quit && !TERMINATED.load(std::sync::atomic::Ordering::Relaxed) {
            if last_tick.elapsed() >= Duration::from_millis(500) {
                app.refresh();
                last_tick = Instant::now();
                dirty = true;
            }
            if dirty {
                term.draw(|f| app.draw(f))?;
                dirty = false;
            }
            // Wait for input, at most until the next refresh is due.
            if event::poll(Duration::from_millis(500))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => app.key(k.code, k.modifiers),
                    Event::Mouse(m) => match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => app.click(m.column, m.row),
                        MouseEventKind::ScrollDown => app.scroll(true),
                        MouseEventKind::ScrollUp => app.scroll(false),
                        _ => continue,
                    },
                    Event::Resize(..) => {}
                    _ => continue,
                }
                dirty = true;
            }
        }
        Ok(())
    })();

    restore()?;
    result?;
    Ok(if app.open_window { Exit::OpenWindow } else { Exit::Quit })
}

fn restore() -> std::io::Result<()> {
    terminal::disable_raw_mode()?;
    execute!(std::io::stdout(), event::DisableMouseCapture, terminal::LeaveAlternateScreen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect()
    }

    #[test]
    fn every_page_draws_without_services_at_both_sizes_and_says_what_is_wrong() {
        let mut app = App::new(Bus::offline());
        app.refresh();
        for page in 0..PAGES.len() {
            app.go(page);
            app.refresh();
            for (w, h) in [(120, 34), (80, 24)] {
                let screen = render(&mut app, w, h);
                assert!(screen.contains("Unix Guardian"), "page {page} at {w}x{h}");
                assert!(screen.contains("quit"), "the key bar is always there");
            }
        }
        app.go(0);
        app.refresh();
        assert!(render(&mut app, 120, 34).contains("Cannot reach"), "an unreachable service is said, not hidden");
    }

    fn snap(i: u64) -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "ts_unix_ms": 1_000_000 + i * 1000,
            "cpu": {"load_pct": 20.0 + i as f64, "temp_c": 52.0, "power_w": 11.0, "freq_avg_mhz": 1800, "freq_max_mhz": 4100,
                    "per_core_load": [10.0, 90.0, 50.0, 0.0], "governor": "powersave", "epp": "", "driver": "intel_pstate"},
            "gpus": [{"pci": "0000:01:00.0", "temp_c": 0.0, "temp_hotspot_c": 0.0, "temp_mem_c": 0.0, "load_pct": 0.0,
                      "power_w": 0.0, "power_limit_w": 0.0, "core_clock_mhz": 0, "mem_clock_mhz": 0, "vram_used_mb": 0,
                      "fan_pct": 0.0, "fan_rpm": 0, "asleep": true}],
            "mem": {"total_kb": 16_000_000, "available_kb": 6_000_000, "swap_total_kb": 0, "swap_used_kb": 0, "zram_used_kb": 0},
            "net": {"iface": "wlan0", "rx_bps": 250_000, "tx_bps": 20_000, "rx_total_bytes": 0, "tx_total_bytes": 0}
        }))
        .map(|mut s: Snapshot| {
            // As the daemon sends a sleeping card: nothing read.
            s.gpus[0].temp_c = f64::NAN;
            s
        })
        .expect("snapshot json")
    }

    #[test]
    fn the_live_monitor_draws_every_reading() {
        let mut app = App::new(Bus::offline());
        app.go(3);
        app.choice = 0;
        for i in 0..10 {
            let e = ExtraSample { battery_pct: 81.0, battery_w: 9.5, battery_status: "Discharging".into(), disk_read_bps: 2_500_000, load_avg_1: 1.25, processes: 612, ..Default::default() };
            app.live.push_back((snap(i), e));
        }
        let screen = render(&mut app, 140, 44);
        for want in ["Live", "Load avg", "1.25", "612", "81 % 9.5 W", "Temperature", "Clock", "Memory", "Disk", "Internet", "Battery power", "▂▇▅▁", "Battery power  9.5 W"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        // A sleeping graphics card is never charted (it was not read).
        assert!(!screen.contains("Graphics card"));
    }

    #[test]
    fn the_fps_tab_says_what_to_do_in_each_state() {
        let mut app = App::new(Bus::offline());
        app.go(3);
        app.tab = 2;
        let mut r = FpsReading { state: "not-installed".into(), install_hint: "sudo apt install mangohud".into(), ..Default::default() };
        app.fps.store(Ok(r.clone()));
        assert!(render(&mut app, 120, 34).contains("sudo apt install mangohud"));
        r.state = "not-set-up".into();
        app.fps.store(Ok(r.clone()));
        assert!(render(&mut app, 120, 34).contains("Set up"));
        r.state = "live".into();
        r.game = "cs2".into();
        r.fps = 141.0;
        r.low_1pct_fps = 88.0;
        r.frametime_ms = 7.1;
        r.recent = (0..20).map(|i| (1_000_000 + i * 200, 140.0)).collect();
        app.fps.store(Ok(r));
        let screen = render(&mut app, 120, 34);
        for want in ["cs2", "141", "88", "7.1 ms", "last ten seconds"] {
            assert!(screen.contains(want), "missing {want:?}");
        }
    }

    #[test]
    fn the_ping_page_shows_targets_background_tasks_and_rules() {
        use ug_proto::{AppTraffic, NetRule, PingSample, PingStats, PingTarget};
        let mut app = App::new(Bus::offline());
        app.go(4);
        let samples: Vec<PingSample> = (0..30).map(|i| PingSample { ts_unix_ms: data::now_ms() - 30_000 + i * 1000, rtt_ms: 20.0 + (i % 3) as f64 }).collect();
        let stats = PingStats { sent: 30, received: 30, median_ms: 21.0, jitter_ms: 1.5, p95_ms: 22.0, loss_pct: 0.0 };
        app.ping.store(Ok(PingReport {
            targets: vec![
                PingTarget { id: "gateway".into(), label: "Router".into(), addr: "192.168.1.1".into(), samples: samples.clone(), stats: stats.clone(), problem: String::new(), via: "wlan0".into(), through_vpn: false },
                PingTarget { id: "internet".into(), label: "DNS server".into(), addr: "1.1.1.1".into(), samples, stats, problem: String::new(), via: "tun0".into(), through_vpn: true },
            ],
            marks: vec![],
        }));
        let steam = AppTraffic {
            id: "a/steam".into(), name: "steam".into(), kind: "app".into(), uid: 1000, exe: "steam".into(), pids: vec![1],
            rx_bps: 4_200_000, tx_bps: 20_000, rx_bytes: 9e8 as u64, tx_bytes: 1, state: String::new(), category: "launcher".into(), has_game: false,
        };
        let firefox = AppTraffic { id: "a/ff".into(), name: "firefox-esr".into(), category: String::new(), rx_bps: 300_000, ..steam.clone() };
        let rule = NetRule { app: "qbittorrent".into(), block: false, up_kbps: 1000, down_kbps: 8000, when: "gaming".into(), active: false };
        app.net.store(Ok((vec![steam, firefox], vec![rule], NetControlStatus { available: true, ..Default::default() })));
        let screen = render(&mut app, 130, 40);
        for want in ["Router", "21 ms", "1.1.1.1", "through the VPN (tun0)", "direct (wlan0)", "Ping, last five minutes", "Background tasks", "steam", "launcher", "4.2 MB/s"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        assert!(!screen.contains("firefox-esr"), "a browser download is not a background task");
        app.tab = 1;
        let screen = render(&mut app, 130, 40);
        for want in ["firefox-esr", "Rules", "qbittorrent", "up 1.0 Mbit/s", "waits for a game"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    #[test]
    fn the_drivers_page_explains_findings_reviews_a_fix_and_lists_changes() {
        let mut app = App::new(Bus::offline());
        app.go(8);
        let finding = |id: &str, sev: &str, auto: bool| ug_proto::DriverFinding {
            id: id.into(),
            severity: sev.into(),
            title_en: format!("Title of {id}"),
            what_en: "What it means".into(),
            why_en: "Why it matters".into(),
            how_en: "How it is fixed".into(),
            fix_id: if auto { "nvidia-suspend".into() } else { "reboot".into() },
            fix_auto: auto,
            commands: vec!["sudo do-the-thing".into()],
            ..Default::default()
        };
        app.driver.store(Ok(ug_proto::DriverReport {
            distro: "Debian".into(),
            findings: vec![finding("vram", "important", true), finding("mismatch", "critical", false)],
            ..Default::default()
        }));
        let screen = render(&mut app, 130, 40);
        for want in ["Check", "Changes made", "IMPORTANT", "Title of vram", "fixable here", "Why it matters", "Enter: review the fix", "sudo do-the-thing"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        app.driver_review = Some(("nvidia-suspend".into(), Ok(ug_proto::DriverPlan {
            fix_id: "nvidia-suspend".into(),
            steps: vec![ug_proto::DriverStep { kind: "write-file".into(), target: "/etc/modprobe.d/x.conf".into(), detail: "options nvidia X=1".into(), undo: "put back".into() }],
            needs_reboot: true,
            ..Default::default()
        })));
        let screen = render(&mut app, 130, 40);
        for want in ["Exactly this will happen", "write-file /etc/modprobe.d/x.conf", "| options nvidia X=1", "Enter again applies it"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        app.tab = 1;
        app.driver_ops.store(Ok(vec![ug_proto::DriverOp { id: "d1".into(), title: "Keep video memory".into(), state: "done".into(), log: vec!["Wrote /etc/modprobe.d/x.conf".into()], needs_reboot: true, ..Default::default() }]));
        let screen = render(&mut app, 130, 40);
        for want in ["done", "Keep video memory", "Wrote /etc/modprobe.d/x.conf", "Enter twice undoes"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    #[test]
    fn the_settings_page_shows_the_agent_settings_and_this_installation() {
        let mut app = App::new(Bus::offline());
        app.go(11);
        app.refresh();
        assert!(render(&mut app, 120, 34).contains("Cannot reach the session bus"), "says why when there is no agent");
        app.settings.store(Ok(ug_proto::SettingsState {
            settings: ug_proto::AgentSettings { notifications: false, scan_interval_s: 5, close_window_on_game: true },
            agent_version: "0.1.0".into(),
            config_dir: "/home/u/.config/unix-guardian".into(),
            data_dir: "/home/u/.local/share/unix-guardian".into(),
            scan_interval_fixed: false,
        }));
        app.tray.store(Ok(false));
        let screen = render(&mut app, 130, 34);
        for want in ["Settings", "Notifications", "‹ off ›", "Close the window when a game starts", "‹ on ›", "‹ 5 s ›", "‹ hidden ›", "0.1.0", ".config/unix-guardian", "GNU GPL"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        assert!(!screen.contains("Not built yet"));
    }

    #[test]
    fn a_terminal_key_press_reads_as_a_shortcut() {
        use pages::combo;
        assert_eq!(combo(KeyCode::Char('g'), KeyModifiers::CONTROL | KeyModifiers::ALT, false).as_deref(), Some("Ctrl+Alt+G"));
        assert_eq!(combo(KeyCode::Char('G'), KeyModifiers::ALT, true).as_deref(), Some("Meta+Alt+Shift+G"));
        assert_eq!(combo(KeyCode::Char('g'), KeyModifiers::SUPER, false).as_deref(), Some("Meta+G"));
        assert_eq!(combo(KeyCode::F(5), KeyModifiers::NONE, false).as_deref(), Some("F5"));
        assert_eq!(combo(KeyCode::Char(' '), KeyModifiers::CONTROL, false).as_deref(), Some("Ctrl+Space"));
        assert_eq!(combo(KeyCode::PageUp, KeyModifiers::ALT, false).as_deref(), Some("Alt+Page_Up"));
        assert_eq!(combo(KeyCode::Char(','), KeyModifiers::ALT, false).as_deref(), Some("Alt+,"));
        assert_eq!(combo(KeyCode::CapsLock, KeyModifiers::ALT, false), None);
    }

    #[test]
    fn the_shortcuts_tab_shows_the_keys_or_the_commands_to_bind() {
        let mut app = App::new(Bus::offline());
        app.go(5);
        app.tab = 1;
        let keys = |trigger: &str| {
            ["open-window", "toggle-overlay", "toggle-gamemode"]
                .iter()
                .map(|id| ug_proto::ShortcutInfo {
                    id: (*id).into(),
                    description: format!("Do {id}"),
                    trigger: trigger.into(),
                    keys: "Meta+Alt+K".into(),
                    chosen: true,
                })
                .collect()
        };
        app.shortcuts.store(Ok(ug_proto::ShortcutsState {
            enabled: true,
            status: "active".into(),
            shortcuts: keys("Meta+Shift+G"),
            gamemode_by_hand: "on".into(),
            ..Default::default()
        }));
        let screen = render(&mut app, 120, 34);
        for want in ["Global shortcuts", "Use global shortcuts", "‹ on ›", "Meta+Shift+G", "Do toggle-gamemode", "on by hand"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        assert!(!screen.contains("ug shortcut"), "no commands while the desktop binds the keys");
        app.shortcuts.store(Ok(ug_proto::ShortcutsState { enabled: true, status: "unavailable".into(), shortcuts: keys(""), ..Default::default() }));
        let screen = render(&mut app, 120, 34);
        for want in ["not offered by this desktop", "ug shortcut toggle-overlay", "Meta+Alt+K (yours)"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }

        // c records new keys for the selected shortcut; Tab adds Meta, which
        // most terminals can't see; Esc cancels without leaving the page.
        app.focus = Focus::Page;
        app.row = 3;
        app.key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(app.recording, Some(("toggle-gamemode".to_string(), false)));
        assert!(render(&mut app, 120, 34).contains("press the keys…"));
        app.key(KeyCode::Tab, KeyModifiers::NONE);
        assert!(render(&mut app, 120, 34).contains("Meta+press the keys…"));
        app.key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!app.quit, "while recording, q is a key to record, not quit");
        assert_eq!(app.recording, None);
        app.key(KeyCode::Char('c'), KeyModifiers::NONE);
        app.key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!((app.recording.clone(), app.focus, app.quit), (None, Focus::Page, false));
    }

    #[test]
    fn the_overlay_page_and_fps_sessions_draw() {
        let mut app = App::new(Bus::offline());
        app.go(5);
        app.overlay.store(Ok(OverlayState { installed: true, set_up: true, settings: Default::default(), ..Default::default() }));
        let screen = render(&mut app, 120, 34);
        for want in ["Overlay", "only while gaming", "top-left", "50 %", "Alt_L+o", "Remove Unix Guardian's setup"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        app.overlay.store(Ok(OverlayState { installed: false, install_hint: "sudo apt install mangohud".into(), ..Default::default() }));
        assert!(render(&mut app, 120, 34).contains("sudo apt install mangohud"));

        app.go(3);
        app.tab = 2;
        app.fps.store(Ok(FpsReading { state: "idle".into(), ..Default::default() }));
        app.sessions.store(Ok(vec![FpsSession {
            game: "cs2".into(), started_unix_ms: data::now_ms(), duration_s: 3120, frames: 1, avg_fps: 141.0,
            low_1pct_fps: 96.0, low_01pct_fps: 71.0, p99_frametime_ms: 10.4, stutters: 2,
        }]));
        let screen = render(&mut app, 120, 34);
        for want in ["Game sessions this week", "cs2", "141", "96", "71", "smooth"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    #[test]
    fn the_gpu_tuner_waits_to_be_asked_then_shows_limits() {
        let mut app = App::new(Bus::offline());
        app.go(1);
        app.tab = 1;
        let screen = render(&mut app, 120, 34);
        assert!(screen.contains("Show tuning") && screen.contains("wakes the graphics card"));
        let st = ug_proto::GpuTuneState {
            pci: "0000:01:00.0".into(), power_limit: true, power_limit_w: 60.0, power_default_w: 60.0, power_min_w: 35.0, power_max_w: 95.0,
            core_offset: true, core_min_mhz: -200, core_max_mhz: 300, ..Default::default()
        };
        app.tune_draft = ug_proto::GpuTune { power_limit_w: 60.0, ..Default::default() };
        app.tune = Some(("0000:01:00.0".into(), st));
        app.row = 0;
        pages::step_choice(&mut app, -1);
        let screen = render(&mut app, 120, 34);
        for want in ["55 W", "default 60, 35–95 W", "Core clock offset", "Try for 30 seconds", "88 °C"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    #[test]
    fn a_tiny_terminal_gets_a_request_instead_of_a_broken_layout() {
        let mut app = App::new(Bus::offline());
        assert!(render(&mut app, 50, 15).contains("at least 60"));
    }

    #[test]
    fn clicking_a_menu_entry_opens_that_page_in_both_layouts() {
        for (w, h) in [(120, 34), (80, 24)] {
            let mut app = App::new(Bus::offline());
            render(&mut app, w, h);
            let (rect, _) = *app.hits.iter().find(|(_, hit)| *hit == Hit::Menu(2)).expect("menu entry 3 is clickable");
            app.click(rect.x, rect.y);
            assert_eq!(app.page, 2);
            assert_eq!(app.focus, Focus::Page);
        }
    }

    #[test]
    fn keys_move_through_the_menu_and_q_quits() {
        let mut app = App::new(Bus::offline());
        app.key(KeyCode::Down, KeyModifiers::NONE);
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.page, 2);
        app.key(KeyCode::Char('4'), KeyModifiers::NONE);
        assert_eq!(app.page, 3);
        app.key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(app.quit);
    }

    #[test]
    fn playtime_lists_games_by_time_and_deleting_asks_twice() {
        let mut app = App::new(Bus::offline());
        app.go(6);
        app.focus = Focus::Page;
        let e = |key: &str, name: &str, seconds: u64| ug_proto::PlaytimeEntry {
            key: key.into(),
            name: name.into(),
            seconds,
            sessions: 3,
            last_played_unix_ms: 0,
        };
        app.playtime.store(Ok(vec![e("steam:730", "Counter-Strike 2", 3 * 3600 + 7 * 60), e("steam:1245620", "Elden Ring", 25 * 60)]));
        let screen = render(&mut app, 120, 34);
        for want in ["This week", "Played this week", "3 h 32 min", "2 games", "most played", "3 h 07 min", "88 %", "25 min"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        app.key(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(app.armed.as_ref().is_some_and(|(a, _)| a.contains("Counter-Strike 2")), "first press only arms");
        assert!(render(&mut app, 120, 34).contains("press again: delete"));
    }

    #[test]
    fn a_guessed_game_can_be_moved_to_apps_and_apps_have_their_own_tab() {
        let mut app = App::new(Bus::offline());
        app.go(6);
        app.focus = Focus::Page;
        let e = |key: &str, name: &str, seconds: u64, sessions: u32| ug_proto::PlaytimeEntry {
            key: key.into(),
            name: name.into(),
            seconds,
            sessions,
            last_played_unix_ms: 0,
        };
        app.playtime.store(Ok(vec![
            e("steam:730", "Counter-Strike 2", 7200, 2),
            e("exe:wondershare filmora", "Wondershare Filmora", 900, 1),
            e("app:dolphin", "Dolphin", 600, 3),
        ]));
        let firefox = ug_proto::TrackedApp { exe: "firefox-esr".into(), name: "Firefox ESR".into() };
        let dolphin = ug_proto::TrackedApp { exe: "dolphin".into(), name: "Dolphin".into() };
        app.playtime_extra.store(Ok((vec![dolphin.clone()], vec![])));
        app.programs.store(Ok(vec![dolphin, firefox]));

        let games = render(&mut app, 120, 34);
        assert!(games.contains("1×  guessed") && games.contains("2 games"), "{games}");
        assert!(!games.contains("Dolphin"), "apps are not games");

        app.row = 0;
        app.key(KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(app.armed.is_none(), "a Steam game is never moved");
        app.row = 1;
        app.key(KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(app.armed.as_ref().is_some_and(|(a, _)| a == "move Wondershare Filmora to Apps"), "first press only arms");
        assert!(render(&mut app, 120, 34).contains("n again: not a game"));

        app.key(KeyCode::Tab, KeyModifiers::NONE);
        let apps = render(&mut app, 120, 34);
        for want in ["Apps this week", "1 counted", "Dolphin", "counting", "Open now", "+ Firefox ESR"] {
            assert!(apps.contains(want), "missing {want:?} in\n{apps}");
        }
        assert!(!apps.contains("+ Dolphin"), "a tracked app is not offered again");
        assert_eq!(app.rows, 2, "Dolphin, then Firefox to add");
    }

    #[test]
    fn steam_launch_options_show_now_and_after_and_wait_for_steam_to_close() {
        let mut app = App::new(Bus::offline());
        app.go(1);
        app.focus = Focus::Page;
        app.key(KeyCode::Tab, KeyModifiers::NONE);
        app.key(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.tab, 2);
        let game = |id: &str, name: &str, launcher: &str| ug_proto::InstalledGameInfo { id: id.into(), name: name.into(), launcher: launcher.into() };
        app.installed.store(Ok(vec![game("steam:730", "Counter-Strike 2", "steam"), game("lutris:3", "osu!", "lutris")]));
        let info = ug_proto::LaunchInfo {
            appid: 730,
            name: "Counter-Strike 2".into(),
            current: "-novid".into(),
            own: "-novid".into(),
            steam_running: true,
            overlay_problem: "MangoHud is not installed.".into(),
            ..Default::default()
        };
        app.launch = Some(pages::LaunchEdit { appid: 730, info: Ok(info), gpu: String::new(), overlay: false, after: Ok("__NV_PRIME_RENDER_OFFLOAD=1 %command% -novid".into()) });
        let screen = render(&mut app, 120, 34);
        for want in ["Steam launch options", "Counter-Strike 2", "System's choice", "[–]", "MangoHud is not installed.", "Now    -novid", "After  __NV_PRIME_RENDER_OFFLOAD=1 %command% -novid", "Steam is running"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        assert!(!screen.contains("osu!"), "only Steam games have Steam launch options");
        app.key(KeyCode::Char('o'), KeyModifiers::NONE);
        assert!(!app.launch.as_ref().unwrap().overlay, "the overlay cannot be chosen without MangoHud");
    }

    #[test]
    fn a_power_action_needs_a_second_press_and_esc_cancels_it() {
        let mut app = App::new(Bus::offline());
        app.go(2);
        app.focus = Focus::Page;
        app.key(KeyCode::Enter, KeyModifiers::NONE); // "Shut down", now
        assert!(app.armed.is_some(), "first press only arms");
        app.key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.armed.is_none());
        assert_eq!(app.focus, Focus::Page, "Esc cancelled the action, it did not leave the page");
    }
}
