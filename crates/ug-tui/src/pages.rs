//! The pages: drawing, and what their keys and clicks do.

use crate::{data, App, Hit, PAGES};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, BorderType, Borders, Chart, Dataset, GraphType, Paragraph, Wrap,
};
use ratatui::Frame;
use std::time::{Duration, Instant};

const OPTIONS: [(&str, &str, &str); 10] = [
    (
        "cpu_performance",
        "CPU at full speed",
        "Governor and energy preference set to performance.",
    ),
    (
        "cpu_no_idle_states",
        "No clock drops between frames",
        "Keeps the CPU out of deep sleep states between frames.",
    ),
    (
        "cpu_preferred_cores",
        "Game on the fastest cores",
        "Performance cores (Intel) or V-Cache cores (AMD X3D).",
    ),
    (
        "cpu_priority",
        "Game gets priority",
        "Higher scheduling priority than other apps.",
    ),
    (
        "memory_tuning",
        "Memory tuned for games",
        "Less swapping, larger memory-map limit, no background compaction.",
    ),
    (
        "pause_services",
        "Background updates held",
        "Package updates and indexing wait until you stop playing.",
    ),
    (
        "network_tuning",
        "Network tuned for low ping",
        "Wi-Fi power saving off, TCP Fast Open, smaller send queue, BBR.",
    ),
    (
        "power_profile",
        "Performance power profile",
        "The system's performance profile (and the laptop's performance fan mode) while playing.",
    ),
    (
        "keep_screen_on",
        "Screen kept on",
        "No screen lock, dimming or sleep while playing.",
    ),
    (
        "do_not_disturb",
        "Do not disturb",
        "Desktop notifications held while playing (KDE Plasma), Unix Guardian's too.",
    ),
];
const WHEN: [(&str, u64); 6] = [
    ("Now", 0),
    ("in 15 min", 15),
    ("in 30 min", 30),
    ("in 1 h", 60),
    ("in 2 h", 120),
    ("in 4 h", 240),
];
const ACTIONS: [(&str, &str); 5] = [
    ("power-off", "Shut down"),
    ("reboot", "Restart"),
    ("suspend", "Sleep"),
    ("hibernate", "Hibernate"),
    ("reboot-firmware", "Restart into BIOS/UEFI setup"),
];
const RANGES: [&str; 4] = ["Live", "Last hour", "Today", "This week"];

fn card<'a>(app: &App, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(app.theme.border())
        .title(Span::styled(format!(" {title} "), app.theme.title()))
        .padding(ratatui::widgets::Padding::horizontal(1))
}

/// A card in its category's colour (border and title), as in the window.
fn cat_card<'a>(title: &'a str, color: Color) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .padding(ratatui::widgets::Padding::horizontal(1))
}

fn opt(v: f64, unit: &str, digits: usize) -> String {
    if v.is_nan() {
        "—".into()
    } else {
        format!("{v:.digits$}{unit}")
    }
}

fn rate(bps: u64) -> String {
    match bps {
        b if b >= 1_000_000 => format!("{:.1} MB/s", b as f64 / 1e6),
        b if b >= 1_000 => format!("{:.0} kB/s", b as f64 / 1e3),
        b => format!("{b} B/s"),
    }
}

fn bar(width: u16, frac: f64) -> String {
    let w = width as usize;
    let filled = ((frac.clamp(0.0, 1.0)) * w as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(w - filled))
}

fn heat(c: f64) -> (&'static str, usize) {
    if c.is_nan() {
        ("", 0)
    } else if c >= 85.0 {
        ("hot", 2)
    } else if c >= 70.0 {
        ("warm", 1)
    } else {
        ("cool", 0)
    }
}

/// Draws the current page into `r`.
pub fn draw(app: &mut App, f: &mut Frame, r: Rect, wide: bool) {
    let inner = Rect {
        x: r.x + 1,
        y: r.y,
        width: r.width.saturating_sub(2),
        height: r.height,
    };
    app.rows = 0;
    match app.page {
        0 => dashboard(app, f, inner, wide),
        1 => {
            tabs(
                app,
                f,
                Rect { height: 1, ..inner },
                &["Game mode", "Graphics card", "Steam launch options"],
            );
            let body = Rect {
                y: inner.y + 2,
                height: inner.height.saturating_sub(2),
                ..inner
            };
            match app.tab {
                1 => gpu_tune_page(app, f, body),
                2 => launch_page(app, f, body),
                _ => game_mode(app, f, body),
            }
        }
        2 => system(app, f, inner),
        3 => monitoring(app, f, inner),
        4 => ping_page(app, f, inner),
        5 => {
            tabs(
                app,
                f,
                Rect { height: 1, ..inner },
                &["Overlay", "Global shortcuts"],
            );
            let body = Rect {
                y: inner.y + 2,
                height: inner.height.saturating_sub(2),
                ..inner
            };
            if app.tab == 1 {
                shortcuts_page(app, f, body);
            } else {
                overlay_page(app, f, body);
            }
        }
        6 => playtime_page(app, f, inner),
        7 => recorder_page(app, f, inner),
        10 => crate::lab::page(app, f, inner),
        11 => updates_page(app, f, inner),
        12 => settings_page(app, f, inner),
        8 => {
            tabs(
                app,
                f,
                Rect { height: 1, ..inner },
                &["Check", "Changes made"],
            );
            let body = Rect {
                y: inner.y + 2,
                height: inner.height.saturating_sub(2),
                ..inner
            };
            if app.tab == 1 {
                driver_ops_page(app, f, body);
            } else {
                driver_page(app, f, body);
            }
        }
        p => {
            let text = vec![
                Line::from(Span::styled(PAGES[p].0, app.theme.title())),
                Line::from(""),
                Line::from(Span::styled(
                    "Not built yet. This part of Unix Guardian is on the roadmap but not in this version; nothing here pretends to work.",
                    app.theme.muted(),
                )),
            ];
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
        }
    }
}

/// `3 h 07 min`, `12 min`, `<1 min`.
fn hm(secs: u64) -> String {
    match (secs / 3600, (secs % 3600) / 60) {
        (0, 0) => "<1 min".into(),
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} h {m:02} min"),
    }
}

/// One row of the Playtime page's Apps tab: a tracked app, or an app no
/// longer counted that still has time in the period.
struct AppRow {
    key: String,
    exe: String,
    name: String,
    seconds: u64,
    sessions: u32,
    counting: bool,
}

fn game_rows(app: &App) -> Vec<ug_proto::PlaytimeEntry> {
    app.playtime
        .value
        .iter()
        .flatten()
        .filter(|e| !e.key.starts_with("app:"))
        .cloned()
        .collect()
}

fn app_rows(app: &App) -> Vec<AppRow> {
    let tracked = app
        .playtime_extra
        .value
        .as_ref()
        .map(|v| v.0.clone())
        .unwrap_or_default();
    let mut rows: Vec<AppRow> = app
        .playtime
        .value
        .iter()
        .flatten()
        .filter_map(|e| {
            let exe = e.key.strip_prefix("app:")?.to_string();
            let counting = tracked.iter().any(|a| a.exe == exe);
            Some(AppRow {
                key: e.key.clone(),
                exe,
                name: e.name.clone(),
                seconds: e.seconds,
                sessions: e.sessions,
                counting,
            })
        })
        .collect();
    for a in tracked {
        if !rows.iter().any(|r| r.exe == a.exe) {
            rows.push(AppRow {
                key: format!("app:{}", a.exe),
                exe: a.exe,
                name: a.name,
                seconds: 0,
                sessions: 0,
                counting: true,
            });
        }
    }
    rows
}

/// Open programs not yet tracked, below the apps on the Apps tab.
fn program_rows(app: &App) -> Vec<ug_proto::TrackedApp> {
    let rows = app_rows(app);
    app.programs
        .value
        .iter()
        .flatten()
        .filter(|p| !rows.iter().any(|r| r.exe == p.exe))
        .cloned()
        .collect()
}

fn fit(name: &str, w: usize) -> String {
    let mut n: String = name.chars().take(w).collect();
    if name.chars().count() > w {
        n.pop();
        n.push('…');
    }
    format!("{n:<w$}")
}

/// Playtime: the period, then games or apps by time, with what can be done
/// to them.
fn playtime_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    tabs(app, f, Rect { height: 1, ..r }, &["Games", "Apps", "Days"]);
    let r = Rect {
        y: r.y + 2,
        height: r.height.saturating_sub(2),
        ..r
    };
    if app.tab == 2 {
        playtime_days(app, f, r);
        return;
    }
    let names: Vec<String> = ["Today", "This week", "All time"]
        .map(String::from)
        .to_vec();
    let chosen = app.choice.min(2);
    choices(app, f, Rect { height: 1, ..r }, "Period", &names, chosen);
    let mut y = r.y + 2;
    let line = |y: u16| Rect {
        x: r.x,
        y,
        width: r.width,
        height: 1,
    };
    // Tracked apps and imports come from newer agents only; without them the
    // page still lists what was recorded.
    if let Some(e) = app.playtime.error.clone() {
        f.render_widget(
            Paragraph::new(Span::styled(e, Style::default().fg(t.danger))),
            line(y),
        );
        return;
    }
    if app.playtime.value.is_none() {
        return;
    }
    let name_w = (r.width as usize / 3).clamp(12, 34);
    // name, time, share, then room for "  press again: delete".
    let bar_w = (r.width as usize).saturating_sub(name_w + 46).clamp(6, 30);
    let bottom = r.bottom().saturating_sub(3);
    let armed = |app: &App, what: &str| app.armed.as_ref().is_some_and(|(a, _)| a == what);

    let hint = if app.tab == 0 {
        let games = game_rows(app);
        let imported: Vec<ug_proto::PlaytimeEntry> = app
            .playtime_extra
            .value
            .as_ref()
            .map(|v| v.1.clone())
            .unwrap_or_default();
        let total: u64 = games.iter().map(|e| e.seconds).sum();
        let mut summary = vec![
            Span::styled(
                ["Played today ", "Played this week ", "Played in total "][chosen],
                t.muted(),
            ),
            Span::styled(hm(total), t.title()),
            Span::styled(
                format!(
                    "   {} {}",
                    games.len(),
                    if games.len() == 1 { "game" } else { "games" }
                ),
                t.muted(),
            ),
        ];
        if let Some(top) = games.first() {
            summary.push(Span::styled("   most played ", t.muted()));
            summary.push(Span::styled(top.name.clone(), t.text()));
        }
        f.render_widget(Paragraph::new(Line::from(summary)), line(y));
        y += 2;
        if games.is_empty() {
            let msg = [
                "No games played today.",
                "No games played this week.",
                "No games recorded yet.",
            ][chosen];
            f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), line(y));
            y += 2;
        }
        app.rows = games.len();
        for (i, e) in games.iter().enumerate() {
            if y >= bottom {
                break;
            }
            let frac = if total > 0 {
                e.seconds as f64 / total as f64
            } else {
                0.0
            };
            let steam = if chosen == 2 {
                imported.iter().find(|m| m.key == e.key).map(|m| m.seconds)
            } else {
                None
            };
            let note = if armed(app, &format!("delete {}'s playtime", e.name)) {
                (
                    "  press again: delete".to_string(),
                    Style::default().fg(t.danger),
                )
            } else if armed(app, &format!("move {} to Apps", e.name)) {
                (
                    "  n again: not a game".to_string(),
                    Style::default().fg(t.danger),
                )
            } else {
                let mut n = if e.sessions == 0 {
                    "  from Steam".to_string()
                } else {
                    format!("  {}×", e.sessions)
                };
                if e.sessions > 0 && steam.is_some() {
                    n.push_str(" +Steam");
                }
                if e.key.starts_with("exe:") {
                    n.push_str("  guessed");
                }
                (n, t.faint())
            };
            let l = Line::from(vec![
                Span::styled(format!("{}  ", fit(&e.name, name_w)), t.text()),
                Span::styled(format!("{:>13}  ", hm(e.seconds)), t.title()),
                Span::styled(format!("{:>4.0} %  ", frac * 100.0), t.muted()),
                Span::styled(bar(bar_w as u16, frac), Style::default().fg(t.accent)),
                Span::styled(note.0, note.1),
            ]);
            row(app, f, line(y), i, l);
            y += 1;
        }
        format!(
            "Counted automatically while a game runs. x deletes a game's history · n moves a guessed game to Apps · i imports Steam's own playtime into All time{} · e saves the period as CSV in Downloads.",
            if imported.is_empty() { "" } else { ", u removes it" }
        )
    } else {
        let apps = app_rows(app);
        let programs = program_rows(app);
        let total: u64 = apps.iter().map(|a| a.seconds).sum();
        let counting = apps.iter().filter(|a| a.counting).count();
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    ["Apps today ", "Apps this week ", "Apps in total "][chosen],
                    t.muted(),
                ),
                Span::styled(hm(total), t.title()),
                Span::styled(format!("   {counting} counted"), t.muted()),
            ])),
            line(y),
        );
        y += 2;
        if apps.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled("No apps yet: pick an open program below to see how much of your time it takes.", t.muted())),
                line(y),
            );
            y += 2;
        }
        app.rows = apps.len() + programs.len();
        for (i, a) in apps.iter().enumerate() {
            if y >= bottom {
                break;
            }
            let frac = if total > 0 {
                a.seconds as f64 / total as f64
            } else {
                0.0
            };
            let note = if armed(app, &format!("delete {}'s playtime", a.name)) {
                (
                    "  press again: delete".to_string(),
                    Style::default().fg(t.danger),
                )
            } else if a.counting {
                ("  counting".to_string(), Style::default().fg(t.accent))
            } else {
                ("  not counted".to_string(), t.faint())
            };
            let l = Line::from(vec![
                Span::styled(format!("{}  ", fit(&a.name, name_w)), t.text()),
                Span::styled(format!("{:>13}  ", hm(a.seconds)), t.title()),
                Span::styled(format!("{:>4.0} %  ", frac * 100.0), t.muted()),
                Span::styled(bar(bar_w as u16, frac), Style::default().fg(t.accent)),
                Span::styled(note.0, note.1),
            ]);
            row(app, f, line(y), i, l);
            y += 1;
        }
        if !programs.is_empty() && y + 2 < bottom {
            y += 1;
            f.render_widget(
                Paragraph::new(Span::styled("Open now (Enter starts counting)", t.muted())),
                line(y),
            );
            y += 1;
            for (j, p) in programs.iter().enumerate() {
                if y >= bottom {
                    break;
                }
                let l = Line::from(vec![
                    Span::styled(format!("+ {}", fit(&p.name, name_w)), t.text()),
                    Span::styled(format!("  {}", p.exe), t.faint()),
                ]);
                row(app, f, line(y), apps.len() + j, l);
                y += 1;
            }
        }
        "Apps are only timed, never tuned. Enter stops or restarts counting · x deletes an app's history · any program: ug playtime track <name>.".to_string()
    };
    y += 1;
    if y < r.bottom() {
        f.render_widget(
            Paragraph::new(Span::styled(hint, t.faint())).wrap(Wrap { trim: true }),
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: r.bottom() - y,
            },
        );
    }
}

/// Deletes a game's or an app's playtime on the second press.
fn forget_game(app: &mut App, i: usize) {
    let target = if app.tab == 0 {
        game_rows(app)
            .get(i)
            .map(|e| (e.key.clone(), e.name.clone()))
    } else {
        app_rows(app)
            .get(i)
            .filter(|a| a.sessions > 0)
            .map(|a| (a.key.clone(), a.name.clone()))
    };
    let Some((key, name)) = target else { return };
    let confirm = format!("delete {name}'s playtime");
    if app.armed.as_ref().is_some_and(|(a, _)| *a == confirm) {
        app.armed = None;
        let r = app.bus.playtime_forget(&key);
        report(app, r.map(|n| format!("Deleted {n} sessions of {name}.")));
        app.playtime.invalidate();
        app.row = app.row.saturating_sub(1);
    } else {
        app.armed = Some((confirm, std::time::Instant::now()));
    }
}

/// Ping stability, s: "slow down background apps" on or off, keeping its caps.
pub fn toggle_background(app: &mut App) {
    let Some((on, up, down, when)) = app.net_bg.clone() else {
        report(
            app,
            Err("The installed service does not offer this yet; reinstall Unix Guardian.".into()),
        );
        return;
    };
    let r = app.bus.net_set_background(!on, up, down, &when).map(|_| {
        if on {
            "Background apps are no longer slowed.".to_string()
        } else {
            "Background apps are slowed now; the game, VPNs and DNS managers are left alone."
                .to_string()
        }
    });
    report(app, r);
    app.net_bg = app.bus.net_background().ok();
    app.net.invalidate();
}

/// Monitoring → Internet traffic, r twice: the counts start from now.
pub fn reset_traffic(app: &mut App) {
    let confirm = "reset the internet-use counts".to_string();
    if app.armed.as_ref().is_some_and(|(a, _)| *a == confirm) {
        app.armed = None;
        let r = app.bus.reset_traffic().and_then(|ok| {
            if ok {
                Ok("The internet-use counts start from now.".to_string())
            } else {
                Err("The agent could not reset the counts.".into())
            }
        });
        report(app, r);
        app.traffic.invalidate();
    } else {
        app.armed = Some((confirm, std::time::Instant::now()));
    }
}

/// "Not a game" for a guessed game, on the second press.
pub fn not_a_game(app: &mut App, i: usize) {
    if app.tab != 0 {
        return;
    }
    let Some(e) = game_rows(app).get(i).cloned() else {
        return;
    };
    if !e.key.starts_with("exe:") {
        report(app, Err(format!("{} was recognised by its launcher or the game database, not guessed; it stays a game.", e.name)));
        return;
    }
    let confirm = format!("move {} to Apps", e.name);
    if app.armed.as_ref().is_some_and(|(a, _)| *a == confirm) {
        app.armed = None;
        let r = app.bus.move_to_apps(&e.key);
        report(app, r.map(|n| format!("{} is an app now: no game mode for it, its time ({n} sessions) counts under Apps.", e.name)));
        app.playtime.invalidate();
        app.playtime_extra.invalidate();
    } else {
        app.armed = Some((confirm, std::time::Instant::now()));
    }
}

/// Enter on the Playtime page: confirms a move armed with n, deletes on
/// Games, starts or stops counting on Apps.
fn activate_playtime(app: &mut App, i: usize) {
    if app.tab == 0 {
        let moving = game_rows(app).get(i).is_some_and(|e| {
            app.armed
                .as_ref()
                .is_some_and(|(a, _)| *a == format!("move {} to Apps", e.name))
        });
        if moving {
            not_a_game(app, i);
        } else {
            forget_game(app, i);
        }
        return;
    }
    let apps = app_rows(app);
    let r = if let Some(a) = apps.get(i) {
        if a.counting {
            app.bus.untrack_app(&a.exe).map(|_| {
                format!(
                    "No longer counting {}. The time counted so far stays.",
                    a.name
                )
            })
        } else {
            app.bus
                .track_app(&a.exe, &a.name)
                .map(|_| format!("Counting {} again.", a.name))
        }
    } else if let Some(p) = program_rows(app).get(i - apps.len()) {
        app.bus
            .track_app(&p.exe, &p.name)
            .map(|_| format!("Counting {} from now on.", p.name))
    } else {
        return;
    };
    report(app, r);
    app.playtime.invalidate();
    app.playtime_extra.invalidate();
    app.programs.invalidate();
}

/// Imports Steam's own playtime, or (`undo`) removes the import.
pub fn import_steam(app: &mut App, undo: bool) {
    if app.tab != 0 {
        return;
    }
    let r = if undo {
        app.bus
            .forget_imported()
            .map(|n| format!("Removed the Steam playtime of {n} games."))
    } else {
        app.bus.import_steam().map(|(n, secs)| {
            if n == 0 {
                "Nothing to add: Steam has no playtime that Unix Guardian has not counted already."
                    .to_string()
            } else {
                format!(
                    "Added {} of Steam playtime across {n} games to All time.",
                    hm(secs)
                )
            }
        })
    };
    report(app, r);
    app.playtime.invalidate();
    app.playtime_extra.invalidate();
}

/// Saves the chosen period's sessions as CSV in the Downloads folder.
pub fn export_playtime(app: &mut App) {
    let since =
        ug_core::playtime::since(ug_core::playtime::RANGES[app.choice.min(2)], data::now_ms());
    let r = app.bus.playtime_sessions(since).and_then(|s| {
        let stamp = ug_core::playtime::local_time(data::now_ms()).replace([' ', ':'], "-");
        let path =
            ug_core::playtime::downloads_dir().join(format!("unix-guardian-playtime-{stamp}.csv"));
        std::fs::write(&path, ug_core::playtime::csv(&s))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!("Saved {} sessions to {}", s.len(), path.display()))
    });
    report(app, r);
}

fn tabs(app: &mut App, f: &mut Frame, r: Rect, names: &[&str]) {
    let mut x = r.x;
    for (i, n) in names.iter().enumerate() {
        let w = n.chars().count() as u16 + 2;
        let rect = Rect {
            x,
            y: r.y,
            width: w.min(r.right().saturating_sub(x)),
            height: 1,
        };
        let style = if i == app.tab {
            app.theme.accent().add_modifier(Modifier::UNDERLINED)
        } else {
            app.theme.muted()
        };
        f.render_widget(Paragraph::new(Span::styled(format!(" {n} "), style)), rect);
        app.hits.push((rect, Hit::Tab(i)));
        x += w + 1;
    }
}

/// A row of choices; the chosen one highlighted, each clickable.
/// A one-line choice. When the names do not fit, the line keeps its size and
/// scrolls: the chosen name is always shown, `‹` / `›` mark hidden names on
/// that side (clicking one picks the next hidden name), and a name wider than
/// the whole line is cut with `…`.
fn choices(app: &mut App, f: &mut Frame, r: Rect, label: &str, names: &[String], chosen: usize) {
    let mut x = r.x;
    f.render_widget(
        Paragraph::new(Span::styled(label, app.theme.muted())),
        Rect {
            x,
            y: r.y,
            width: label.len() as u16,
            height: 1,
        },
    );
    x += label.len() as u16 + 1;
    let room = r.right().saturating_sub(x) as usize;
    let (first, last, shown) = choice_window(names, chosen, room);
    if first > 0 {
        let rect = Rect {
            x,
            y: r.y,
            width: 2,
            height: 1,
        };
        f.render_widget(Paragraph::new(Span::styled("‹ ", app.theme.accent())), rect);
        app.hits.push((rect, Hit::Choice(first - 1)));
        x += 2;
    }
    for (i, n) in shown.iter().enumerate().map(|(k, n)| (first + k, n)) {
        let w = n.chars().count() as u16 + 2;
        let rect = Rect {
            x,
            y: r.y,
            width: w,
            height: 1,
        };
        let style = if i == chosen {
            app.theme.selected()
        } else {
            app.theme.muted()
        };
        f.render_widget(Paragraph::new(Span::styled(format!(" {n} "), style)), rect);
        app.hits.push((rect, Hit::Choice(i)));
        x += w + 1;
    }
    if last + 1 < names.len() {
        let rect = Rect {
            x: r.right().saturating_sub(2).max(x),
            y: r.y,
            width: 2,
            height: 1,
        };
        f.render_widget(Paragraph::new(Span::styled(" ›", app.theme.accent())), rect);
        app.hits.push((rect, Hit::Choice(last + 1)));
    }
}

/// Which names fit in `room` columns (each takes its length + 2, with one
/// column between), keeping `chosen` in view and leaving two columns for each
/// `‹` / `›` marker that is needed. Returns the first and last index shown and
/// the names to draw (cut with `…` when one alone is too wide).
fn choice_window(names: &[String], chosen: usize, room: usize) -> (usize, usize, Vec<String>) {
    if names.is_empty() {
        return (0, 0, vec![]);
    }
    let chosen = chosen.min(names.len() - 1);
    let width = |i: usize| names[i].chars().count() + 2;
    let span = |a: usize, b: usize| (a..=b).map(width).sum::<usize>() + (b - a);
    let fits = |a: usize, b: usize| {
        let marks = if a > 0 { 2 } else { 0 } + if b + 1 < names.len() { 2 } else { 0 };
        span(a, b) + marks <= room
    };
    // Smallest first index that still shows the chosen name, then as many
    // names after it as fit.
    let Some(first) = (0..=chosen).find(|&a| fits(a, chosen)) else {
        let marks = if chosen > 0 { 2 } else { 0 } + if chosen + 1 < names.len() { 2 } else { 0 };
        let keep = room.saturating_sub(marks + 3).max(1);
        let cut: String = names[chosen].chars().take(keep).collect();
        return (chosen, chosen, vec![format!("{cut}…")]);
    };
    let mut last = chosen;
    while last + 1 < names.len() && fits(first, last + 1) {
        last += 1;
    }
    (first, last, names[first..=last].to_vec())
}

/// A selectable list row; returns the rect so callers can add detail lines.
fn row(app: &mut App, f: &mut Frame, r: Rect, i: usize, line: Line) {
    let selected = app.focus == crate::Focus::Page && app.row == i;
    let style = if selected {
        app.theme.selected()
    } else {
        Style::default()
    };
    f.render_widget(Paragraph::new(line).style(style), r);
    app.hits.push((r, Hit::Row(i)));
}

fn dashboard(app: &mut App, f: &mut Frame, r: Rect, wide: bool) {
    let t = app.theme;
    let Some(s) = app.snapshot.value.clone() else {
        let msg = app
            .snapshot
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    };
    let inv = app.inventory.value.clone();
    let gpu_awake = s.gpus.iter().find(|g| !g.asleep && !g.temp_c.is_nan());
    let dgpu = inv
        .as_ref()
        .and_then(|i| i.gpus.iter().find(|g| !g.boot_vga));
    let igpu = inv
        .as_ref()
        .and_then(|i| i.gpus.iter().find(|g| g.boot_vga));
    let (heat_word, level) = heat(s.cpu.temp_c);
    let heat_color = [t.ok, t.warn, t.danger][level];
    let mem_used = s.mem.total_kb.saturating_sub(s.mem.available_kb) as f64;
    let mem_frac = if s.mem.total_kb > 0 {
        mem_used / s.mem.total_kb as f64
    } else {
        0.0
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if wide {
            vec![
                Constraint::Length(6),
                Constraint::Min(8),
                Constraint::Length(3),
            ]
        } else {
            vec![
                Constraint::Length(5),
                Constraint::Min(6),
                Constraint::Length(3),
            ]
        })
        .split(r);

    // Tiles: the GPU, the processor, memory, network. With two GPUs the
    // GPU tile starts with the card switch and shows the chosen card.
    let switch = gpu_switch(app);
    let gpu_lines: Vec<Line> = if let Some((names, chosen)) = &switch {
        let gpus = app
            .gpu_switch
            .value
            .as_ref()
            .map(|v| v.0.clone())
            .unwrap_or_default();
        let pci = &gpus[*chosen].pci;
        let name = |pci: &str| {
            inv.as_ref()
                .and_then(|i| i.gpus.iter().find(|g| g.pci == pci))
                .map(|g| short_name(&g.name))
                .unwrap_or_else(|| pci.to_string())
        };
        let sample = s.gpus.iter().find(|g| &g.pci == pci);
        let reading = match sample {
            Some(g) if g.asleep => Line::from(Span::styled("asleep", t.title())),
            Some(g) if !g.temp_c.is_nan() => Line::from(vec![
                Span::styled(opt(g.temp_c, "°", 0), t.title()),
                Span::styled(format!("  {}", opt(g.load_pct, " %", 0)), t.muted()),
            ]),
            Some(g) => Line::from(Span::styled(format!("{} MHz", g.core_clock_mhz), t.title())),
            None => Line::from(Span::styled("—", t.title())),
        };
        let other = &gpus[1 - *chosen];
        let note = {
            let o = s.gpus.iter().find(|g| g.pci == other.pci);
            let state = match o {
                Some(g) if g.asleep => "asleep".to_string(),
                Some(g) if !g.temp_c.is_nan() => opt(g.temp_c, "°", 0),
                _ => "idle".to_string(),
            };
            Line::from(Span::styled(
                format!("{}: {state}", name(&other.pci)),
                t.faint(),
            ))
        };
        let mut l = vec![
            reading,
            Line::from(Span::styled(name(pci), t.faint())),
            note,
        ];
        if !wide {
            l[0].spans.push(Span::styled(
                format!("  ⇄ {}", names[*chosen]),
                Style::default().fg(t.accent),
            ));
        }
        l
    } else {
        match (gpu_awake, dgpu) {
            (Some(g), Some(d)) => vec![
                Line::from(vec![
                    Span::styled(opt(g.temp_c, "°", 0), t.title()),
                    Span::styled(format!("  {}", opt(g.load_pct, " %", 0)), t.muted()),
                ]),
                Line::from(Span::styled(d.name.clone(), t.faint())),
            ],
            _ => {
                let clock = igpu
                    .and_then(|i| s.gpus.iter().find(|g| g.pci == i.pci))
                    .map(|g| g.core_clock_mhz)
                    .unwrap_or(0);
                vec![
                    Line::from(Span::styled(format!("{clock} MHz"), t.title())),
                    Line::from(Span::styled(
                        igpu.map(|g| g.name.clone()).unwrap_or_default(),
                        t.faint(),
                    )),
                ]
            }
        }
    };
    let cpu_lines = vec![
        Line::from(vec![
            Span::styled(
                opt(s.cpu.temp_c, "°", 0),
                Style::default().fg(heat_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {heat_word}   {}", opt(s.cpu.load_pct, " %", 0)),
                t.muted(),
            ),
            Span::styled(format!("   {}", opt(s.cpu.power_w, " W", 0)), t.muted()),
        ]),
        Line::from(Span::styled(
            bar(16, s.cpu.load_pct.max(0.0) / 100.0),
            Style::default().fg(t.cpu),
        )),
        Line::from(Span::styled(
            inv.as_ref()
                .map(|i| short_name(&i.cpu.model))
                .unwrap_or_default(),
            t.faint(),
        )),
    ];
    let mem_lines = vec![
        Line::from(vec![
            Span::styled(format!("{:.1}", mem_used / 1_048_576.0), t.title()),
            Span::styled(
                format!(
                    " / {:.1} GB  {:.0} %",
                    s.mem.total_kb as f64 / 1_048_576.0,
                    mem_frac * 100.0
                ),
                t.muted(),
            ),
        ]),
        Line::from(Span::styled(bar(16, mem_frac), Style::default().fg(t.mem))),
    ];
    let net_lines = vec![
        Line::from(vec![
            Span::styled("↓ ", t.muted()),
            Span::styled(rate(s.net.rx_bps), t.title()),
        ]),
        Line::from(vec![
            Span::styled("↑ ", t.muted()),
            Span::styled(rate(s.net.tx_bps), t.text()),
        ]),
        Line::from(Span::styled(s.net.iface.clone(), t.faint())),
    ];
    let tiles = [
        ("Graphics card", gpu_lines, t.gpu),
        ("Processor", cpu_lines, t.cpu),
        ("Memory", mem_lines, t.mem),
        ("Network", net_lines, t.net),
    ];
    if wide {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 4); 4])
            .split(rows[0]);
        for (i, (title, lines, color)) in tiles.into_iter().enumerate() {
            f.render_widget(Paragraph::new(lines).block(cat_card(title, color)), cols[i]);
        }
        // The switch sits in the tile's top border, like the window's
        // icon: "⇄ Intel"; a click (or ←/→) switches to the other card.
        if let Some((names, chosen)) = &switch {
            let c = cols[0];
            let text = format!(" ⇄ {} ", names[*chosen]);
            let w = text.chars().count() as u16;
            let title = "Graphics card".len() as u16 + 4;
            // Too narrow for both: on the tile's free last line instead.
            let rect = if c.width > w + title + 2 {
                Rect {
                    x: c.right() - w - 2,
                    y: c.y,
                    width: w,
                    height: 1,
                }
            } else {
                Rect {
                    x: c.x + 1,
                    y: c.bottom().saturating_sub(2),
                    width: w.min(c.width.saturating_sub(2)),
                    height: 1,
                }
            };
            f.render_widget(
                Paragraph::new(Span::styled(text, app.theme.selected())),
                rect,
            );
            app.hits.push((rect, Hit::Choice(1 - *chosen)));
        }
    } else {
        let mut lines = Vec::new();
        for (title, l, color) in tiles {
            let mut first = vec![Span::styled(
                format!("{title:<14}"),
                Style::default().fg(color),
            )];
            first.extend(l[0].spans.clone());
            lines.push(Line::from(first));
        }
        f.render_widget(Paragraph::new(lines), rows[0]);
    }

    // Game mode and the live temperature chart.
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(rows[1]);
    let game = app.running.value.as_ref().and_then(|v| v.first().cloned());
    let (hand, hand_profile) = app
        .hand
        .value
        .clone()
        .unwrap_or_else(|| (String::new(), "balanced".into()));
    let on =
        hand == "on" || (hand != "off" && (game.is_some() || app.gamemode.value == Some(true)));
    let profile_id = match &game {
        Some(g) if hand != "on" => g.profile.clone(),
        _ => hand_profile,
    };
    let profile = app
        .profiles
        .value
        .as_ref()
        .and_then(|p| p.iter().find(|p| p.id == profile_id).cloned());
    // The state in words, as the window's badge: On, Off, Waiting for a game.
    let (badge, badge_style) = if on {
        (" On ", Style::default().fg(Color::Black).bg(t.ok))
    } else if hand.is_empty() {
        (
            " Waiting for a game ",
            Style::default().fg(Color::White).bg(t.accent),
        )
    } else {
        (" Off ", Style::default().fg(t.text).bg(t.line))
    };
    let mut gm = vec![Line::from(vec![
        Span::styled(badge, badge_style.add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(
            match &game {
                _ if hand == "on" => format!("— on by hand ({profile_id} profile); g turns it off"),
                Some(g) => format!("— {} ({} profile)", g.name, g.profile),
                None => format!("— starts with a game ({profile_id} profile); g turns it on now"),
            },
            t.muted(),
        ),
    ])];
    if let Some(p) = profile {
        let flags = [
            p.cpu_performance,
            p.cpu_no_idle_states,
            p.cpu_preferred_cores,
            p.cpu_priority,
            p.memory_tuning,
            p.pause_services,
            p.network_tuning,
            p.power_profile,
            p.keep_screen_on,
            p.do_not_disturb,
        ];
        for ((_, label, _), on) in OPTIONS.iter().zip(flags) {
            gm.push(Line::from(vec![
                Span::styled(
                    if on { "■ " } else { "□ " },
                    Style::default().fg(if on { t.accent } else { t.faint }),
                ),
                Span::styled(*label, if on { t.text() } else { t.faint() }),
            ]));
        }
    }
    f.render_widget(
        Paragraph::new(gm).wrap(Wrap { trim: true }).block(if on {
            cat_card("Game mode", t.accent)
        } else {
            card(app, "Game mode")
        }),
        mid[0],
    );
    temp_chart(app, f, mid[1]);

    // Per-core load, P and E cores apart.
    let p_cores: Vec<u32> = inv
        .as_ref()
        .map(|i| i.cpu.p_cores.clone())
        .unwrap_or_default();
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let glyph = |l: f64| blocks[((l.clamp(0.0, 100.0) / 100.0) * 7.0).round() as usize];
    let mut p = String::new();
    let mut e = String::new();
    for (i, l) in s.cpu.per_core_load.iter().enumerate() {
        let g = glyph(if l.is_nan() { 0.0 } else { *l });
        if p_cores.is_empty() || p_cores.contains(&(i as u32)) {
            p.push(g)
        } else {
            e.push(g)
        }
    }
    let mut spans = vec![
        Span::styled(
            if e.is_empty() {
                "Cores "
            } else {
                "Performance "
            },
            t.muted(),
        ),
        Span::styled(p, Style::default().fg(t.accent)),
    ];
    if !e.is_empty() {
        spans.push(Span::styled("   Efficiency ", t.muted()));
        spans.push(Span::styled(e, Style::default().fg(t.accent)));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).block(card(app, "Load per core")),
        rows[2],
    );
}

/// The card switch, when there are two GPUs: their short names (boot GPU
/// first) and the chosen one.
fn gpu_switch(app: &App) -> Option<(Vec<String>, usize)> {
    let (gpus, st) = app.gpu_switch.value.as_ref()?;
    if gpus.len() != 2 {
        return None;
    }
    let same = gpus[0].vendor == gpus[1].vendor;
    let names = gpus
        .iter()
        .map(|g| match g.vendor_name() {
            _ if same => if g.boot_vga { "Integrated" } else { "Discrete" }.to_string(),
            "nvidia" => "NVIDIA".into(),
            "amd" => "AMD".into(),
            "intel" => "Intel".into(),
            _ => if g.boot_vga { "Integrated" } else { "Discrete" }.to_string(),
        })
        .collect();
    let chosen = gpus.iter().position(|g| g.pci == st.chosen).unwrap_or(0);
    Some((names, chosen))
}

/// Makes GPU `i` of the switch the one `ug run` starts programs on.
fn choose_gpu(app: &mut App, i: usize) {
    let Some((gpus, st)) = app.gpu_switch.value.clone() else {
        return;
    };
    let Some(g) = gpus.get(i) else { return };
    if g.pci == st.chosen {
        return;
    }
    let r = ug_core::gpu_default::set(&ug_core::paths::user_config(), g)
        .map(|_| {
            "Programs Unix Guardian starts (`ug run`) use this card; the desktop is not changed."
                .to_string()
        })
        .map_err(|e| e.to_string());
    report(app, r);
    app.gpu_switch.invalidate();
    // Shown at once, not at the next refresh.
    let st = ug_core::gpu_default::state(&ug_core::paths::user_config(), &gpus);
    app.gpu_switch.store(Ok((gpus, st)));
}

fn short_name(s: &str) -> String {
    s.replace("12th Gen Intel(R) Core(TM) ", "")
        .replace("NVIDIA GeForce ", "")
        .replace(" Laptop GPU", "")
}

fn temp_chart(app: &App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some(&(t0, _, _)) = app.temps.front() else {
        f.render_widget(
            Paragraph::new("Collecting readings…").block(card(app, "Temperature, last 2 minutes")),
            r,
        );
        return;
    };
    let cpu: Vec<(f64, f64)> = app
        .temps
        .iter()
        .filter(|p| !p.1.is_nan())
        .map(|p| (p.0 - t0, p.1))
        .collect();
    let gpu: Vec<(f64, f64)> = app
        .temps
        .iter()
        .filter(|p| !p.2.is_nan())
        .map(|p| (p.0 - t0, p.2))
        .collect();
    let span = app.temps.back().map(|p| p.0 - t0).unwrap_or(1.0).max(60.0);
    let lo = cpu
        .iter()
        .chain(&gpu)
        .map(|p| p.1)
        .fold(30.0_f64, f64::min)
        .floor();
    let hi = cpu
        .iter()
        .chain(&gpu)
        .map(|p| p.1)
        .fold(70.0_f64, f64::max)
        .ceil();
    let mut sets = vec![Dataset::default()
        .name("Processor")
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(t.cpu))
        .data(&cpu)];
    if !gpu.is_empty() {
        sets.push(
            Dataset::default()
                .name("Graphics card")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(t.gpu))
                .data(&gpu),
        );
    }
    let two = sets.len() > 1;
    let chart = Chart::new(sets)
        .legend_position(two.then_some(ratatui::widgets::LegendPosition::TopRight))
        .block(card(app, "Temperature, last 2 minutes"))
        .x_axis(Axis::default().bounds([0.0, span]))
        .y_axis(Axis::default().style(t.faint()).bounds([lo, hi]).labels([
            Span::raw(format!("{lo:.0}°")),
            Span::raw(format!("{:.0}°", (lo + hi) / 2.0)),
            Span::raw(format!("{hi:.0}°")),
        ]));
    f.render_widget(chart, r);
}

fn game_mode(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let profiles = app.profiles.value.clone().unwrap_or_default();
    if profiles.is_empty() {
        let msg = app
            .profiles
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    }
    app.choice = app.choice.min(profiles.len() - 1);
    let game = app.running.value.as_ref().and_then(|v| v.first().cloned());
    let hand = app.hand.value.clone().unwrap_or_default();
    let on =
        hand.0 == "on" || (hand.0 != "off" && (game.is_some() || app.gamemode.value == Some(true)));
    let head = Line::from(vec![
        Span::styled("Game mode  ", t.title()),
        Span::styled(if on { "On" } else { "Off" }, if on { t.accent() } else { t.muted() }),
        Span::styled(
            match &game {
                Some(g) => {
                    let gpu = app
                        .inventory
                        .value
                        .as_ref()
                        .and_then(|i| i.gpus.iter().find(|x| x.pci == g.gpu).map(|x| x.name.clone()))
                        .unwrap_or_else(|| g.gpu.clone());
                    let on = if gpu.is_empty() { String::new() } else { format!(", on {gpu}") };
                    format!("   {} is running ({} profile{on})", g.name, g.profile)
                }
                _ if hand.0 == "on" => format!("   On by hand with the {} profile; g turns it off", hand.1),
                None => "   Starts by itself when a game starts; g turns it on now with the profile shown".into(),
            },
            t.muted(),
        ),
    ]);
    f.render_widget(Paragraph::new(head), Rect { height: 1, ..r });
    let names: Vec<String> = profiles
        .iter()
        .map(|p| {
            if p.builtin {
                capital(&p.id)
            } else {
                format!("{} (yours)", p.id)
            }
        })
        .collect();
    let chosen = app.choice;
    choices(
        app,
        f,
        Rect {
            y: r.y + 2,
            height: 1,
            ..r
        },
        "Profile",
        &names,
        chosen,
    );
    let t = app.theme;
    let p = profiles[chosen].clone();
    let flags = [
        p.cpu_performance,
        p.cpu_no_idle_states,
        p.cpu_preferred_cores,
        p.cpu_priority,
        p.memory_tuning,
        p.pause_services,
        p.network_tuning,
        p.power_profile,
        p.keep_screen_on,
        p.do_not_disturb,
    ];
    let has_preferred = app
        .inventory
        .value
        .as_ref()
        .is_some_and(|i| i.cpu.has_vcache || !i.cpu.p_cores.is_empty());
    let mut y = r.y + 4;
    for (i, ((key, label, explain), on)) in OPTIONS.iter().zip(flags).enumerate() {
        if y + 1 >= r.bottom() {
            break;
        }
        let blocked = *key == "cpu_preferred_cores" && !has_preferred;
        let mark = if blocked {
            "[–]"
        } else if on {
            "[■]"
        } else {
            "[ ]"
        };
        let line = Line::from(vec![
            Span::styled(
                format!("{mark} "),
                Style::default().fg(if on { t.accent } else { t.faint }),
            ),
            Span::styled(*label, t.text()),
        ]);
        row(
            app,
            f,
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: 1,
            },
            i,
            line,
        );
        let t = app.theme;
        let why = if blocked {
            "This CPU has no faster core group to prefer."
        } else {
            explain
        };
        f.render_widget(
            Paragraph::new(Span::styled(format!("    {why}"), t.faint())),
            Rect {
                x: r.x,
                y: y + 1,
                width: r.width,
                height: 1,
            },
        );
        y += 2;
    }
    let t = app.theme;
    y += 1;
    let games = app.installed.value.clone().unwrap_or_default();
    if y < r.bottom() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Your games (Enter changes the profile)",
                t.title(),
            )),
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: 1,
            },
        );
        y += 1;
    }
    for (gi, g) in games.iter().enumerate() {
        if y >= r.bottom() {
            break;
        }
        let chosen = profiles
            .iter()
            .find(|p| p.games.contains(&g.id))
            .map(|p| p.id.clone())
            .unwrap_or_else(|| "suggested".into());
        let line = Line::from(vec![
            Span::styled(format!("{:<40}", g.name), app.theme.text()),
            Span::styled(format!(" {chosen}"), app.theme.accent()),
        ]);
        row(
            app,
            f,
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: 1,
            },
            OPTIONS.len() + gi,
            line,
        );
        y += 1;
    }
    app.rows = OPTIONS.len() + games.len();
    // How to put a game on the other GPU (hybrid laptops).
    let root = ug_hw::SysRoot::real();
    let gpus = ug_hw::gpu_use::gpus(&root);
    if let Some(d) = gpus.iter().find(|g| !g.boot_vga) {
        if y + 3 < r.bottom() {
            let t = app.theme;
            let lines = vec![
                Line::from(Span::styled(
                    format!("Run a game on the {} card (no restart)", d.vendor_name()),
                    t.title(),
                )),
                Line::from(Span::styled(
                    format!(
                        "  Steam launch options: {}",
                        ug_hw::gpu_use::launch_options(&root, d)
                    ),
                    t.muted(),
                )),
                Line::from(Span::styled(
                    format!("  Any program: ug run --gpu {} -- program", d.vendor_name()),
                    t.muted(),
                )),
            ];
            f.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: false }),
                Rect {
                    x: r.x,
                    y: y + 1,
                    width: r.width,
                    height: r.bottom() - y - 1,
                },
            );
        }
    }
}

/// The launch options of one Steam game being edited on the Game mode
/// page's third tab.
pub struct LaunchEdit {
    pub appid: u32,
    pub info: Result<ug_proto::LaunchInfo, String>,
    /// The choice: a GPU's PCI address ("" for the system's) and the overlay.
    pub gpu: String,
    pub overlay: bool,
    pub after: Result<String, String>,
}

/// Installed Steam games: (App ID, name).
fn steam_games(app: &App) -> Vec<(u32, String)> {
    app.installed
        .value
        .iter()
        .flatten()
        .filter_map(|g| Some((g.id.strip_prefix("steam:")?.parse().ok()?, g.name.clone())))
        .collect()
}

/// Reads the selected game's launch options when the selection changes, and
/// every few seconds while shown (Steam may have been closed meanwhile).
pub fn refresh_launch(app: &mut App) {
    let Some((appid, _)) = steam_games(app).get(app.row).cloned() else {
        app.launch = None;
        return;
    };
    let stale = app.launch.as_ref().is_none_or(|l| l.appid != appid);
    if !stale && !app.launch_due.due() {
        return;
    }
    app.launch_due.store(Ok(()));
    let info = app.bus.launch_options(appid);
    let (gpu, overlay) = match (&app.launch, &info) {
        (Some(l), _) if !stale => (l.gpu.clone(), l.overlay),
        (_, Ok(i)) => (i.gpu.clone(), i.overlay && i.overlay_problem.is_empty()),
        _ => (String::new(), false),
    };
    let after = if info.is_ok() {
        app.bus.preview_launch_options(appid, &gpu, overlay)
    } else {
        Err(String::new())
    };
    app.launch = Some(LaunchEdit {
        appid,
        info,
        gpu,
        overlay,
        after,
    });
}

fn repreview(app: &mut App) {
    if let Some(l) = app.launch.as_mut() {
        l.after = app.bus.preview_launch_options(l.appid, &l.gpu, l.overlay);
    }
}

/// ←/→: the system's choice, then each GPU.
fn step_launch_gpu(app: &mut App, d: i32) {
    let gpus: Vec<String> = std::iter::once(String::new())
        .chain(
            ug_hw::gpu_use::gpus(&ug_hw::SysRoot::real())
                .into_iter()
                .map(|g| g.pci),
        )
        .collect();
    if let Some(l) = app.launch.as_mut() {
        let i = gpus.iter().position(|g| *g == l.gpu).unwrap_or(0) as i32;
        l.gpu = gpus[(i + d).clamp(0, gpus.len() as i32 - 1) as usize].clone();
    }
    repreview(app);
}

pub fn toggle_launch_overlay(app: &mut App) {
    let Some(l) = app.launch.as_mut() else { return };
    if let Ok(i) = &l.info {
        if !i.overlay_problem.is_empty() {
            let why = i.overlay_problem.clone();
            report(app, Err(why));
            return;
        }
    }
    l.overlay = !l.overlay;
    repreview(app);
}

fn write_launch(app: &mut App) {
    let Some(l) = app.launch.as_ref() else { return };
    let r = app.bus.set_launch_options(l.appid, &l.gpu, l.overlay).map(|(_, backup)| {
        format!("Written: Steam uses it the next time the game starts. Backup of Steam's settings: {backup}")
    });
    report(app, r);
    app.launch_due.invalidate();
    app.launch = app.launch.take().map(|l| LaunchEdit {
        appid: u32::MAX,
        ..l
    });
    refresh_launch(app);
}

pub fn undo_launch(app: &mut App) {
    let Some(l) = app.launch.as_ref() else { return };
    let r = app.bus.undo_launch_options(l.appid).map(|(o, _)| {
        format!(
            "Back to the game's own launch options: {}",
            if o.is_empty() { "(none)" } else { &o }
        )
    });
    report(app, r);
    app.launch = app.launch.take().map(|l| LaunchEdit {
        appid: u32::MAX,
        ..l
    });
    refresh_launch(app);
}

/// The Steam launch options tab: games, then the selected game's choice and
/// what it would write.
fn launch_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let games = steam_games(app);
    let line = |y: u16| Rect {
        x: r.x,
        y,
        width: r.width,
        height: 1,
    };
    if games.is_empty() {
        let msg = app
            .installed
            .error
            .clone()
            .unwrap_or_else(|| "No Steam games are installed.".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), line(r.y));
        return;
    }
    f.render_widget(Paragraph::new(Span::styled("Choose a game with ↑↓; the card it renders on and the MangoHud overlay go into its Steam launch options.", t.faint())), line(r.y));
    let mut y = r.y + 2;
    let list_h = (games.len() as u16).min(r.height.saturating_sub(12).max(3));
    app.rows = games.len();
    let first = app.row.saturating_sub(list_h as usize - 1);
    for (i, (_, name)) in games.iter().enumerate().skip(first).take(list_h as usize) {
        row(
            app,
            f,
            line(y),
            i,
            Line::from(Span::styled(format!(" {name}"), t.text())),
        );
        y += 1;
    }
    y += 1;
    let Some(l) = app.launch.as_ref() else { return };
    let mut lines: Vec<Line> = Vec::new();
    match &l.info {
        Err(e) => lines.push(Line::from(Span::styled(
            e.clone(),
            Style::default().fg(t.danger),
        ))),
        Ok(i) => {
            let gpu_name = if l.gpu.is_empty() {
                "System's choice".to_string()
            } else {
                app.inventory
                    .value
                    .as_ref()
                    .and_then(|v| {
                        v.gpus
                            .iter()
                            .find(|g| g.pci == l.gpu)
                            .map(|g| short_name(&g.name))
                    })
                    .unwrap_or_else(|| l.gpu.clone())
            };
            lines.push(Line::from(vec![
                Span::styled("  Graphics card     ", t.muted()),
                Span::styled(format!("‹ {gpu_name} ›"), t.accent()),
            ]));
            let ov = if !i.overlay_problem.is_empty() {
                "[–]"
            } else if l.overlay {
                "[■]"
            } else {
                "[ ]"
            };
            lines.push(Line::from(vec![
                Span::styled("  MangoHud overlay  ", t.muted()),
                Span::styled(ov, Style::default().fg(t.accent)),
                Span::styled("  (o)", t.faint()),
            ]));
            if !i.overlay_problem.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", i.overlay_problem),
                    t.faint(),
                )));
            }
            let none = |s: &str| {
                if s.is_empty() {
                    "(none)".to_string()
                } else {
                    s.to_string()
                }
            };
            lines.push(Line::from(vec![
                Span::styled("  Now    ", t.muted()),
                Span::styled(none(&i.current), t.text()),
            ]));
            match &l.after {
                Ok(a) => lines.push(Line::from(vec![
                    Span::styled("  After  ", t.muted()),
                    Span::styled(
                        none(a),
                        if *a == i.current {
                            t.text()
                        } else {
                            t.accent()
                        },
                    ),
                ])),
                Err(e) => lines.push(Line::from(Span::styled(
                    format!("  {e}"),
                    Style::default().fg(t.danger),
                ))),
            }
            if i.steam_running {
                lines.push(Line::from(Span::styled("  Steam is running: close it first (Steam → Exit); it rewrites its settings when it exits.", t.warn_style())));
            } else {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  Enter writes it (Steam's settings are backed up first){}",
                        if i.gpu.is_empty() && !i.overlay {
                            ""
                        } else {
                            " · u puts the game's own options back"
                        }
                    ),
                    t.faint(),
                )));
            }
            if i.changed_outside {
                lines.push(Line::from(Span::styled(
                    "  Changed in Steam since Unix Guardian set them: kept and built on.",
                    t.faint(),
                )));
            }
            if i.flatpak {
                lines.push(Line::from(Span::styled(
                    "  Flatpak Steam: the overlay comes from MangoHud's Flatpak layer.",
                    t.faint(),
                )));
            }
        }
    }
    if y < r.bottom() {
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: r.bottom() - y,
            },
        );
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

fn system(app: &mut App, f: &mut Frame, r: Rect) {
    tabs(
        app,
        f,
        Rect { height: 1, ..r },
        &["Power command", "Queue", "Battery", "Cooling", "Sleep"],
    );
    let body = Rect {
        y: r.y + 2,
        height: r.height.saturating_sub(2),
        ..r
    };
    match app.tab {
        0 => power(app, f, body),
        1 => queue(app, f, body),
        2 => battery(app, f, body),
        3 => cooling(app, f, body),
        _ => sleep(app, f, body),
    }
}

const SLEEP_ACTIONS: [&str; 5] = ["ignore", "lock", "suspend", "hibernate", "poweroff"];

/// The rows of the Sleep tab: (id, label); ids are the daemon's.
/// One row of the Alerts tab.
#[derive(Clone, Debug, PartialEq)]
enum AlertRow {
    /// An alert: kind, as in `AlertSensor::kind`.
    Rule(&'static str),
    Sensitivity,
    Cooldown,
    Mount(String),
    Test,
    Clear,
}

/// (kind, name, unit, lowest, highest) of each alert.
const ALERT_RULES: [(&str, &str, &str, u32, u32); 6] = [
    ("cpu_temp", "CPU temperature", "°C", 50, 110),
    ("gpu_temp", "GPU temperature", "°C", 50, 110),
    ("gpu_hotspot", "GPU hotspot", "°C", 60, 120),
    ("gpu_vram", "GPU memory temp.", "°C", 60, 120),
    ("memory", "Memory use", "%", 50, 99),
    ("disk", "Disk fill", "%", 50, 99),
];

fn alert_rule<'a>(s: &'a mut ug_proto::AlertSettings, kind: &str) -> (&'a mut bool, &'a mut u32) {
    match kind {
        "cpu_temp" => (&mut s.cpu_temp_on, &mut s.cpu_temp_c),
        "gpu_temp" => (&mut s.gpu_temp_on, &mut s.gpu_temp_c),
        "gpu_hotspot" => (&mut s.gpu_hotspot_on, &mut s.gpu_hotspot_c),
        "gpu_vram" => (&mut s.gpu_vram_on, &mut s.gpu_vram_c),
        "memory" => (&mut s.mem_on, &mut s.mem_pct),
        _ => (&mut s.disk_on, &mut s.disk_pct),
    }
}

/// Every sensor of a kind reads `unsupported`: the alert cannot work here.
fn alert_unsupported(st: &ug_proto::AlertsState, kind: &str) -> bool {
    let mut it = st.sensors.iter().filter(|x| x.kind == kind).peekable();
    it.peek().is_some() && it.all(|x| x.status == "unsupported")
}

/// The rows, in order. Hotspot and memory temperature appear only where a
/// card was seen reporting them (or the alert is on).
fn alert_rows(st: &ug_proto::AlertsState) -> Vec<AlertRow> {
    let mut rows = Vec::new();
    for (kind, ..) in ALERT_RULES {
        if kind == "gpu_hotspot" || kind == "gpu_vram" {
            let seen = st
                .sensors
                .iter()
                .any(|x| x.kind == kind && (x.status == "ok" || x.status == "asleep"));
            let mut s = st.settings.clone();
            if !seen && !*alert_rule(&mut s, kind).0 {
                continue;
            }
        }
        rows.push(AlertRow::Rule(kind));
    }
    rows.push(AlertRow::Sensitivity);
    rows.push(AlertRow::Cooldown);
    let mut mounts: Vec<String> = st.mounts.iter().map(|m| m.path.clone()).collect();
    for m in &st.settings.disk_mounts {
        if !mounts.contains(m) {
            mounts.push(m.clone());
        }
    }
    rows.extend(mounts.into_iter().map(AlertRow::Mount));
    rows.push(AlertRow::Test);
    if !st.recent.is_empty() {
        rows.push(AlertRow::Clear);
    }
    rows
}

/// What a sensor reads now, in words.
fn alert_reading(x: &ug_proto::AlertSensor, unit: &str, on: bool, sustain: u32) -> String {
    let who = if x.subject.is_empty() {
        String::new()
    } else {
        format!("{}: ", x.subject)
    };
    match (x.status.as_str(), x.reading()) {
        ("ok", Some(v)) if on => {
            let filled = x.above_s.min(sustain) as usize;
            format!(
                "{who}{v:.0} {unit}  {}{}",
                "■".repeat(filled),
                "□".repeat(sustain as usize - filled)
            )
        }
        ("ok", Some(v)) => format!("{who}{v:.0} {unit}"),
        ("asleep", _) => format!("{who}asleep; checked when it wakes"),
        ("unsupported", _) => format!("{who}not supported on this hardware"),
        ("missing", _) => format!("{who}not mounted"),
        ("unknown", _) if x.subject.is_empty() => "the service is not reachable".into(),
        _ => format!("{who}no reading yet"),
    }
}

fn alerts_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some(st) = app.alerts.value.clone() else {
        let msg = app
            .alerts
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(
            Paragraph::new(Span::styled(msg, t.muted())).wrap(Wrap { trim: true }),
            r,
        );
        return;
    };
    let s = &st.settings;
    let rows = alert_rows(&st);
    let mut y = r.y;
    if st.held {
        f.render_widget(
            Paragraph::new(Span::styled(
                " Game mode's do not disturb is on: alerts are recorded, not shown.",
                t.warn_style(),
            )),
            Rect { y, height: 1, ..r },
        );
        y += 1;
    }
    let mut mounts_head = false;
    for (i, row_kind) in rows.iter().enumerate() {
        if y >= r.bottom() {
            break;
        }
        let line = match row_kind {
            AlertRow::Rule(kind) => {
                let (_, name, unit, ..) = ALERT_RULES
                    .iter()
                    .find(|x| x.0 == *kind)
                    .copied()
                    .unwrap_or(ALERT_RULES[0]);
                let mut c = s.clone();
                let (on, limit) = alert_rule(&mut c, kind);
                let (on, limit) = (*on, *limit);
                let no = alert_unsupported(&st, kind);
                let readings: Vec<String> = st
                    .sensors
                    .iter()
                    .filter(|x| x.kind == *kind)
                    .map(|x| alert_reading(x, unit, on, s.sustain_s))
                    .collect();
                let mut spans = vec![Span::styled(format!(" {name:<18}"), t.text())];
                if no {
                    spans.push(Span::styled("not supported on this hardware", t.muted()));
                } else {
                    spans.push(Span::styled(
                        format!("{:<4}", if on { "on" } else { "off" }),
                        if on { t.accent() } else { t.muted() },
                    ));
                    spans.push(Span::styled(format!("‹ {limit:>3} {unit} ›  "), t.accent()));
                    spans.push(Span::styled(readings.join("  ·  "), t.muted()));
                }
                Line::from(spans)
            }
            AlertRow::Sensitivity => Line::from(vec![
                Span::styled(format!(" {:<22}", "Sensitivity"), t.text()),
                Span::styled(format!("‹ {:>2} s ›  ", s.sustain_s), t.accent()),
                Span::styled("above the limit this long before an alert", t.faint()),
            ]),
            AlertRow::Cooldown => Line::from(vec![
                Span::styled(format!(" {:<22}", "Quiet after an alert"), t.text()),
                Span::styled(format!("‹ {:>3} min ›", s.cooldown_min), t.accent()),
            ]),
            AlertRow::Mount(m) => {
                if !mounts_head {
                    mounts_head = true;
                    f.render_widget(
                        Paragraph::new(Span::styled(" Disks the disk alert watches:", t.muted())),
                        Rect { y, height: 1, ..r },
                    );
                    y += 1;
                    if y >= r.bottom() {
                        break;
                    }
                }
                let picked = s.disk_mounts.contains(m);
                let info = st
                    .mounts
                    .iter()
                    .find(|x| &x.path == m)
                    .map(|x| {
                        format!(
                            "{} · {}",
                            x.fs,
                            x.used().map_or("—".into(), |p| format!("{p:.0} % full"))
                        )
                    })
                    .unwrap_or_else(|| "not in the list".into());
                Line::from(vec![
                    Span::styled(
                        format!("   [{}] ", if picked { "x" } else { " " }),
                        t.accent(),
                    ),
                    Span::styled(format!("{m:<20} "), t.text()),
                    Span::styled(info, t.faint()),
                ])
            }
            AlertRow::Test => Line::from(Span::styled(" [ Send a test alert ]", t.accent())),
            AlertRow::Clear => Line::from(Span::styled(" [ Clear the recent alerts ]", t.accent())),
        };
        row(app, f, Rect { y, height: 1, ..r }, i, line);
        y += 1;
    }
    app.rows = rows.len();
    if y + 2 >= r.bottom() {
        return;
    }
    y += 1;
    f.render_widget(
        Paragraph::new(Span::styled(" Recent alerts", t.title())),
        Rect { y, height: 1, ..r },
    );
    y += 1;
    if st.recent.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(" No alerts; everything normal.", t.muted())),
            Rect { y, height: 1, ..r },
        );
        return;
    }
    let lines: Vec<Line> = st
        .recent
        .iter()
        .take(r.bottom().saturating_sub(y) as usize)
        .map(|e| {
            let name = ALERT_RULES
                .iter()
                .find(|x| x.0 == e.kind)
                .map_or("Test alert", |x| x.1);
            let unit = if e.kind == "memory" || e.kind == "disk" {
                "%"
            } else {
                "°C"
            };
            let value = if e.kind == "test" {
                String::new()
            } else {
                format!("{:.0} {unit} (limit {} {unit})", e.value, e.threshold)
            };
            Line::from(vec![
                Span::styled(format!(" {}  ", clock_time(e.ts_unix_ms)), t.faint()),
                Span::styled(format!("{name:<18} {:<14} ", e.subject), t.text()),
                Span::styled(value, t.muted()),
                Span::styled(if e.notified { "" } else { "  held" }, t.warn_style()),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            y,
            height: r.bottom() - y,
            ..r
        },
    );
}

/// "Sun 14:05" in local time.
fn clock_time(unix_ms: u64) -> String {
    // SAFETY: localtime_r writes only into the struct we pass.
    let tm = unsafe {
        let t = (unix_ms / 1000) as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    format!(
        "{} {:02}:{:02}",
        DAYS[tm.tm_wday.clamp(0, 6) as usize],
        tm.tm_hour,
        tm.tm_min
    )
}

/// Saves changed alert settings and says what changed.
fn save_alerts(app: &mut App, s: ug_proto::AlertSettings, said: String) {
    let r = app.bus.set_alerts(&s).map(|_| said);
    if r.is_ok() {
        if let Some(st) = app.alerts.value.as_mut() {
            st.settings = s;
        }
    }
    report(app, r);
    app.alerts.invalidate();
}

/// ←→ on an Alerts row: the limit, the sensitivity or the cooldown.
fn step_alert(app: &mut App, d: i32) {
    let Some(st) = app.alerts.value.clone() else {
        return;
    };
    let Some(which) = alert_rows(&st).get(app.row).cloned() else {
        return;
    };
    let mut s = st.settings.clone();
    let said = match which {
        AlertRow::Rule(kind) => {
            if alert_unsupported(&st, kind) {
                app.say("Not supported on this hardware.", false);
                return;
            }
            let (_, name, unit, lo, hi) = ALERT_RULES
                .iter()
                .find(|x| x.0 == kind)
                .copied()
                .unwrap_or(ALERT_RULES[0]);
            let (_, limit) = alert_rule(&mut s, kind);
            *limit = (*limit as i32 + d).clamp(lo as i32, hi as i32) as u32;
            format!("{name}: alert at {} {unit}.", limit)
        }
        AlertRow::Sensitivity => {
            s.sustain_s = (s.sustain_s as i32 + d).clamp(3, 30) as u32;
            format!("Sensitivity: {} s.", s.sustain_s)
        }
        AlertRow::Cooldown => {
            let list = ug_proto::ALERT_COOLDOWNS;
            let i = list.iter().position(|m| *m == s.cooldown_min).unwrap_or(2) as i32;
            s.cooldown_min = list[(i + d).clamp(0, list.len() as i32 - 1) as usize];
            format!("Quiet for {} min after an alert.", s.cooldown_min)
        }
        _ => return,
    };
    save_alerts(app, s, said);
}

/// Enter on an Alerts row.
fn activate_alert(app: &mut App, i: usize) {
    let Some(st) = app.alerts.value.clone() else {
        return;
    };
    let Some(which) = alert_rows(&st).get(i).cloned() else {
        return;
    };
    let mut s = st.settings.clone();
    match which {
        AlertRow::Rule(kind) => {
            if alert_unsupported(&st, kind) {
                app.say("Not supported on this hardware.", false);
                return;
            }
            let name = ALERT_RULES
                .iter()
                .find(|x| x.0 == kind)
                .map_or(kind, |x| x.1);
            let (on, _) = alert_rule(&mut s, kind);
            *on = !*on;
            let said = format!("{name} alert {}.", if *on { "on" } else { "off" });
            save_alerts(app, s, said);
        }
        AlertRow::Mount(m) => {
            if let Some(p) = s.disk_mounts.iter().position(|x| *x == m) {
                s.disk_mounts.remove(p);
            } else {
                s.disk_mounts.push(m.clone());
            }
            let said = if s.disk_mounts.contains(&m) {
                format!("{m} is watched.")
            } else {
                format!("{m} is no longer watched.")
            };
            save_alerts(app, s, said);
        }
        AlertRow::Test => {
            let r = app.bus.test_alert().map(|e| {
                match e.held.as_str() {
                    "" => "Test alert sent. Did it show on the desktop?",
                    "dnd" => "Held: game mode's do not disturb is on (listed under Recent alerts).",
                    "off" => "Held: notifications are off in Settings.",
                    _ => "No notification service answered.",
                }
                .to_string()
            });
            report(app, r);
            app.alerts.invalidate();
        }
        AlertRow::Clear => {
            let r = app
                .bus
                .clear_alerts()
                .map(|n| format!("Forgot {n} recent alerts."));
            report(app, r);
            app.alerts.invalidate();
        }
        AlertRow::Sensitivity | AlertRow::Cooldown => step_alert(app, 1),
    }
}

fn sleep_rows(s: &ug_proto::SleepSettings) -> Vec<(&'static str, &'static str)> {
    let mut rows = Vec::new();
    if s.sleep_modes.len() > 1 {
        rows.push(("mode", "Sleep variant"));
        rows.push(("keep", "Keep after restart"));
    }
    if s.buttons_note.is_empty() {
        if s.has_lid {
            rows.push(("lid-battery", "Lid closed, on battery"));
            rows.push(("lid-ac", "Lid closed, charging"));
            rows.push(("lid-docked", "Lid closed, docked"));
        }
        rows.push(("power-key", "Power button"));
    }
    rows
}

fn sleep_value(s: &ug_proto::SleepSettings, id: &str) -> String {
    match id {
        "mode" => s.sleep_mode.clone(),
        "keep" => (if s.sleep_mode_kept { "yes" } else { "no" }).into(),
        "lid-battery" => s.lid_battery.clone(),
        "lid-ac" => s.lid_ac.clone(),
        "lid-docked" => s.lid_docked.clone(),
        _ => s.power_key.clone(),
    }
}

fn sleep(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some(s) = app.sleep.value.clone() else {
        let msg = app.sleep.error.clone().unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    };
    let rows = sleep_rows(&s);
    let mut y = r.y;
    for (i, (id, label)) in rows.iter().enumerate() {
        if y >= r.bottom() {
            break;
        }
        row(
            app,
            f,
            Rect { y, height: 1, ..r },
            i,
            Line::from(vec![
                Span::styled(format!(" {label:<26}"), t.text()),
                Span::styled(format!("‹ {} ›", sleep_value(&s, id)), t.accent()),
            ]),
        );
        y += 1;
    }
    app.rows = rows.len();
    let mut notes = Vec::new();
    if s.sleep_modes.len() <= 1 {
        notes.push(format!(
            "Sleep variant: not supported by firmware; it offers only {}.",
            if s.sleep_mode.is_empty() {
                "s2idle"
            } else {
                &s.sleep_mode
            }
        ));
    } else {
        notes.push("s2idle wakes at once and drains more; deep (S3) takes a second longer and uses far less battery.".into());
    }
    if !s.buttons_note.is_empty() {
        notes.push(s.buttons_note.clone());
    }
    for h in &s.lid_handled_by {
        notes.push(format!(
            "The lid is handled by {h}; these apply whenever it is not running."
        ));
    }
    for h in &s.power_key_handled_by {
        notes.push(format!("The power button is handled by {h}."));
    }
    notes.push("←→ changes a row. Set for the whole system (logind), so every desktop behaves the same. Wake alarm: ug power wake --at 07:00.".into());
    if y + 1 < r.bottom() {
        let lines: Vec<Line> = notes
            .into_iter()
            .map(|n| Line::from(Span::styled(n, t.faint())))
            .collect();
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }),
            Rect {
                y: y + 1,
                height: r.bottom() - y - 1,
                ..r
            },
        );
    }
}

/// ←→ (or Enter) on a Sleep row.
fn step_sleep(app: &mut App, d: i32) {
    let Some(s) = app.sleep.value.clone() else {
        return;
    };
    let rows = sleep_rows(&s);
    let Some((id, label)) = rows.get(app.row).copied() else {
        return;
    };
    let cycle = |list: &[String], cur: &str| -> String {
        let i = list.iter().position(|v| v == cur).unwrap_or(0) as i32;
        list[((i + d).rem_euclid(list.len() as i32)) as usize].clone()
    };
    let r = match id {
        "mode" => {
            let next = cycle(&s.sleep_modes, &s.sleep_mode);
            app.bus
                .set_sleep_mode(&next, s.sleep_mode_kept)
                .map(|_| format!("Sleep variant: {next}."))
        }
        "keep" => app
            .bus
            .set_sleep_mode(&s.sleep_mode, !s.sleep_mode_kept)
            .map(|_| {
                if s.sleep_mode_kept {
                    "No longer kept after restart.".to_string()
                } else {
                    "Kept after restart.".to_string()
                }
            }),
        _ => {
            let all: Vec<String> = SLEEP_ACTIONS.iter().map(|a| a.to_string()).collect();
            let next = cycle(&all, &sleep_value(&s, id));
            app.bus
                .set_button_action(id, &next)
                .map(|_| format!("{label}: {next}."))
        }
    };
    report(app, r);
    app.sleep.invalidate();
}

fn power(app: &mut App, f: &mut Frame, r: Rect) {
    let names: Vec<String> = WHEN.iter().map(|w| w.0.to_string()).collect();
    let chosen = app.choice.min(WHEN.len() - 1);
    choices(app, f, Rect { height: 1, ..r }, "When", &names, chosen);
    let caps = app.caps.value.clone().unwrap_or_default();
    let mut y = r.y + 2;
    for (i, (id, label)) in ACTIONS.iter().enumerate() {
        if y + 1 >= r.bottom() {
            break;
        }
        let cap = caps.iter().find(|c| c.action == *id);
        let available = cap.is_none_or(|c| c.available);
        let t = app.theme;
        let danger = matches!(*id, "power-off" | "reboot" | "reboot-firmware");
        let style = if !available {
            t.faint()
        } else if danger {
            Style::default().fg(t.danger).add_modifier(Modifier::BOLD)
        } else {
            t.title()
        };
        row(
            app,
            f,
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: 1,
            },
            i,
            Line::from(Span::styled(format!("  {label}"), style)),
        );
        if let Some(c) = cap.filter(|c| !c.available) {
            f.render_widget(
                Paragraph::new(Span::styled(c.reason.clone(), app.theme.faint()))
                    .wrap(Wrap { trim: true }),
                Rect {
                    x: r.x + 4,
                    y: y + 1,
                    width: r.width.saturating_sub(4),
                    height: 2,
                },
            );
            y += 2;
        }
        y += 1;
    }
    app.rows = ACTIONS.len();
    if y + 2 < r.bottom() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "A scheduled action is skipped, not run late, if the computer was asleep or off when it was due.",
                app.theme.faint(),
            ))
            .wrap(Wrap { trim: true }),
            Rect { x: r.x, y: y + 1, width: r.width, height: 2 },
        );
    }
}

fn queue(app: &mut App, f: &mut Frame, r: Rect) {
    let (jobs, watches) = app.jobs.value.clone().unwrap_or_default();
    if jobs.is_empty() && watches.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Nothing is scheduled or waiting.",
                app.theme.muted(),
            )),
            r,
        );
        return;
    }
    let mut y = r.y;
    for (i, j) in jobs.iter().enumerate() {
        let line = Line::from(vec![
            Span::styled(format!("  {:<24}", j.action), app.theme.title()),
            Span::styled(
                format!("{}  {}", data::local_time(j.at_unix_ms), j.repeat),
                app.theme.muted(),
            ),
        ]);
        row(
            app,
            f,
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: 1,
            },
            i,
            line,
        );
        y += 1;
    }
    for (k, w) in watches.iter().enumerate() {
        let what = match w.kind.as_str() {
            "net-idle" => format!(
                "when downloads stay below {:.0} kB/s for {} min",
                w.threshold / 1000.0,
                w.minutes
            ),
            "cpu-idle" => format!(
                "when CPU load stays below {:.0} % for {} min",
                w.threshold, w.minutes
            ),
            "process-gone" => format!("when {} exits", w.name),
            _ => "when the running game ends".into(),
        };
        let line = Line::from(vec![
            Span::styled(format!("  {:<24}", w.action), app.theme.title()),
            Span::styled(what, app.theme.muted()),
        ]);
        row(
            app,
            f,
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: 1,
            },
            jobs.len() + k,
            line,
        );
        y += 1;
    }
    app.rows = jobs.len() + watches.len();
}

const PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];
const FAN_CURVES: [(&str, [(f64, f64); 4]); 3] = [
    (
        "quiet",
        [(40.0, 20.0), (60.0, 35.0), (75.0, 60.0), (85.0, 100.0)],
    ),
    (
        "balanced",
        [(40.0, 25.0), (60.0, 45.0), (75.0, 70.0), (85.0, 100.0)],
    ),
    (
        "cool",
        [(40.0, 35.0), (55.0, 55.0), (70.0, 85.0), (80.0, 100.0)],
    ),
];

/// Power limit and clock offsets; nothing is read until Enter (it wakes the card).
fn gpu_tune_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some((pci, st)) = app.tune.clone() else {
        let lines = vec![
            Line::from(Span::styled("Power limit and clock offsets", t.title())),
            Line::from(Span::styled("Reading them wakes the graphics card for a moment, so they are read only when you ask.", t.faint())),
            Line::from(""),
        ];
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }),
            Rect { height: 3, ..r },
        );
        app.rows = 1;
        row(
            app,
            f,
            Rect {
                y: r.y + 3,
                height: 1,
                ..r
            },
            0,
            Line::from(Span::styled(" Enter  Show tuning ", t.accent())),
        );
        return;
    };
    let d = app.tune_draft;
    let mut lines = vec![Line::from(Span::styled(
        format!("Power limit and clock offsets ({pci})"),
        t.title(),
    ))];
    if !st.crashed.is_default() {
        lines.push(Line::from(Span::styled(
            format!("Not applied again: the setting being tried when the machine stopped ({:.0} W, core {:+}, memory {:+}).", st.crashed.power_limit_w, st.crashed.core_offset_mhz, st.crashed.mem_offset_mhz),
            t.warn_style(),
        )));
    }
    let h = lines.len() as u16 + 1;
    f.render_widget(Paragraph::new(lines), Rect { height: h, ..r });
    let mut y = r.y + h;
    if !(st.power_limit || st.core_offset || st.mem_offset) {
        // Nothing this card allows: the reasons, and no rows to press.
        app.rows = 0;
        let mut lines = vec![Line::from(Span::styled(
            "Not supported: this card and driver allow no power limit or clock offset from software.",
            t.warn_style(),
        ))];
        lines.extend(
            st.notes
                .iter()
                .map(|n| Line::from(Span::styled(n.clone(), t.faint()))),
        );
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }),
            Rect {
                y,
                height: r.bottom().saturating_sub(y),
                ..r
            },
        );
        return;
    }
    let mut rows: Vec<Line> = Vec::new();
    if st.power_limit {
        rows.push(Line::from(vec![
            Span::styled(format!(" {:<22}", "Power limit"), t.text()),
            Span::styled(
                format!(
                    "‹ {:.0} W ›  default {:.0}, {:.0}–{:.0} W",
                    d.power_limit_w, st.power_default_w, st.power_min_w, st.power_max_w
                ),
                t.muted(),
            ),
        ]));
    }
    if st.core_offset {
        rows.push(Line::from(vec![
            Span::styled(format!(" {:<22}", "Core clock offset"), t.text()),
            Span::styled(
                format!(
                    "‹ {:+} MHz ›  {:+}…{:+}",
                    d.core_offset_mhz, st.core_min_mhz, st.core_max_mhz
                ),
                t.muted(),
            ),
        ]));
    }
    if st.mem_offset {
        rows.push(Line::from(vec![
            Span::styled(format!(" {:<22}", "Memory clock offset"), t.text()),
            Span::styled(
                format!(
                    "‹ {:+} MHz ›  {:+}…{:+}",
                    d.mem_offset_mhz, st.mem_min_mhz, st.mem_max_mhz
                ),
                t.muted(),
            ),
        ]));
    }
    let trying = app
        .tune_until
        .map(|u| u.saturating_duration_since(Instant::now()).as_secs());
    match trying {
        Some(s) => {
            rows.push(Line::from(Span::styled(
                format!(" Enter  Keep  (undone by itself in {s} s)"),
                t.accent(),
            )));
            rows.push(Line::from(Span::styled(" Enter  Undo now", t.muted())));
        }
        None => {
            rows.push(Line::from(Span::styled(
                " Enter  Try for 30 seconds",
                t.accent(),
            )));
            rows.push(Line::from(Span::styled(
                " Enter  Card's defaults",
                t.muted(),
            )));
        }
    }
    app.rows = rows.len();
    for (i, l) in rows.into_iter().enumerate() {
        row(app, f, Rect { y, height: 1, ..r }, i, l);
        y += 1;
    }
    let mut notes = vec![Line::from(Span::styled(
        "Every change is a 30-second trial; a setting being tried when the machine froze is never applied again; a tuned card at 88 °C gets its defaults back.",
        t.faint(),
    ))];
    notes.extend(
        st.notes
            .iter()
            .map(|n| Line::from(Span::styled(n.clone(), t.faint()))),
    );
    if y + 1 < r.bottom() {
        f.render_widget(
            Paragraph::new(notes).wrap(Wrap { trim: true }),
            Rect {
                y: y + 1,
                height: r.bottom() - y - 1,
                ..r
            },
        );
    }
}

/// The adjustable rows of the GPU tuner, in order.
fn tune_fields(st: &ug_proto::GpuTuneState) -> Vec<&'static str> {
    let mut v = Vec::new();
    if st.power_limit {
        v.push("power");
    }
    if st.core_offset {
        v.push("core");
    }
    if st.mem_offset {
        v.push("mem");
    }
    v
}

fn step_tune(app: &mut App, d: i32) {
    let Some((_, st)) = app.tune.clone() else {
        return;
    };
    if app.tune_until.is_some() {
        return;
    }
    let Some(field) = tune_fields(&st).get(app.row).copied() else {
        return;
    };
    let t = &mut app.tune_draft;
    match field {
        "power" => {
            t.power_limit_w =
                (t.power_limit_w + 5.0 * d as f64).clamp(st.power_min_w, st.power_max_w)
        }
        "core" => {
            t.core_offset_mhz = (t.core_offset_mhz + 15 * d).clamp(st.core_min_mhz, st.core_max_mhz)
        }
        _ => t.mem_offset_mhz = (t.mem_offset_mhz + 50 * d).clamp(st.mem_min_mhz, st.mem_max_mhz),
    }
}

fn activate_tune(app: &mut App, i: usize) {
    let Some((pci, st)) = app.tune.clone() else {
        let r = app
            .bus
            .active_gpu()
            .and_then(|pci| app.bus.gpu_tune_state(&pci).map(|st| (pci, st)));
        match r {
            Ok((pci, st)) => {
                let a = st.applied;
                app.tune_draft = ug_proto::GpuTune {
                    power_limit_w: if a.power_limit_w == 0.0 {
                        st.power_limit_w
                    } else {
                        a.power_limit_w
                    },
                    ..a
                };
                app.tune = Some((pci, st));
                app.row = 0;
            }
            Err(e) => app.say(e, false),
        }
        return;
    };
    let fields = tune_fields(&st).len();
    if i < fields {
        step_tune(app, 1);
        return;
    }
    let first = i == fields;
    let (op, msg) = match (app.tune_until.is_some(), first) {
        (false, true) => (
            "try",
            "Trying it: keep it within 30 seconds, or it is undone by itself.",
        ),
        (false, false) => ("reset", "Back to the card's own defaults."),
        (true, true) => ("keep", "Kept. It is applied again whenever the card wakes."),
        (true, false) => ("cancel", "Undone."),
    };
    let mut t = app.tune_draft;
    if (t.power_limit_w - st.power_default_w).abs() < 0.5 {
        t.power_limit_w = 0.0;
    }
    match app.bus.gpu_tune(&pci, op, &t) {
        Ok(()) => {
            app.tune_until = (op == "try").then(|| Instant::now() + Duration::from_secs(30));
            app.say(msg, true);
            if op != "try" {
                if let Ok(st) = app.bus.gpu_tune_state(&pci) {
                    app.tune = Some((pci, st));
                }
            }
        }
        Err(e) => app.say(e, false),
    }
}

/// Rows: power profile, firmware fan mode (when there is one), then fans.
fn cooling(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some((st, ppd)) = app.cooling.value.clone() else {
        let msg = app
            .cooling
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    };
    let mut y = r.y;
    let mut i = 0;
    let profile = if ppd.is_empty() {
        "no power profile service".to_string()
    } else {
        format!("‹ {ppd} ›")
    };
    row(
        app,
        f,
        Rect { y, height: 1, ..r },
        i,
        Line::from(vec![
            Span::styled(format!(" {:<24}", "Power profile"), t.text()),
            Span::styled(profile, t.muted()),
        ]),
    );
    y += 1;
    i += 1;
    if st.fan_mode >= 0 {
        let name = st
            .fan_modes
            .iter()
            .find(|(v, _)| *v == st.fan_mode)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| "other".into());
        row(
            app,
            f,
            Rect { y, height: 1, ..r },
            i,
            Line::from(vec![
                Span::styled(format!(" {:<24}", "Laptop fan mode"), t.text()),
                Span::styled(format!("‹ {name} ›"), t.muted()),
            ]),
        );
        y += 1;
        i += 1;
    }
    y += 1;
    for fan in &st.fans {
        if y >= r.bottom() {
            break;
        }
        let mode = if fan.mode == "firmware" {
            "not supported by firmware".to_string()
        } else if fan.mode == "curve" {
            FAN_CURVES
                .iter()
                .find(|(_, c)| fan.curve == c.to_vec())
                .map(|(n, _)| format!("{n} curve"))
                .unwrap_or_else(|| "own curve".into())
        } else {
            "automatic".into()
        };
        let line = Line::from(vec![
            Span::styled(format!(" {:<24}", fan.label), t.text()),
            Span::styled(
                format!(
                    "{:>5} rpm {:>4.0} % {:>4.0} °C  ",
                    fan.rpm, fan.pct, fan.temp_c
                ),
                t.muted(),
            ),
            Span::styled(format!("‹ {mode} ›"), t.accent()),
        ]);
        row(app, f, Rect { y, height: 1, ..r }, i, line);
        y += 1;
        i += 1;
    }
    app.rows = i;
    let mut notes: Vec<Line> = st
        .notes
        .iter()
        .map(|n| Line::from(Span::styled(n.clone(), t.faint())))
        .collect();
    notes.push(Line::from(Span::styled("No curve runs a fan below 30 % at 80 °C; a stopped or crashed service hands fans back to automatic.", t.faint())));
    if y + 1 < r.bottom() {
        f.render_widget(
            Paragraph::new(notes).wrap(Wrap { trim: true }),
            Rect {
                y: y + 1,
                height: r.bottom() - y - 1,
                ..r
            },
        );
    }
}

/// ←→ (or Enter) on a cooling row.
fn step_cooling(app: &mut App, d: i32) {
    let Some((st, ppd)) = app.cooling.value.clone() else {
        return;
    };
    let cycle = |i: usize, n: usize| ((i as i32 + d).rem_euclid(n as i32)) as usize;
    let mut row = app.row;
    let r = if row == 0 {
        if ppd.is_empty() {
            return;
        }
        let i = PROFILES.iter().position(|p| *p == ppd).unwrap_or(1);
        let next = PROFILES[cycle(i, PROFILES.len())];
        app.bus
            .set_power_profile(next)
            .map(|_| format!("Power profile: {next}."))
    } else {
        row -= 1;
        if st.fan_mode >= 0 && row == 0 {
            let i = st
                .fan_modes
                .iter()
                .position(|(v, _)| *v == st.fan_mode)
                .unwrap_or(0);
            let (v, n) = st.fan_modes[cycle(i, st.fan_modes.len())].clone();
            app.bus
                .set_fan_mode(v)
                .map(|_| format!("Laptop fan mode: {n}."))
        } else {
            let fi = if st.fan_mode >= 0 { row - 1 } else { row };
            let Some(fan) = st.fans.get(fi) else { return };
            if fan.mode == "firmware" {
                report(
                    app,
                    Err(format!(
                        "{}: fan speed is not supported by the firmware.",
                        fan.label
                    )),
                );
                return;
            }
            // automatic → quiet → balanced → cool → automatic
            let cur = if fan.mode == "curve" {
                FAN_CURVES
                    .iter()
                    .position(|(_, c)| fan.curve == c.to_vec())
                    .map(|p| p + 1)
                    .unwrap_or(0)
            } else {
                0
            };
            let next = cycle(cur, FAN_CURVES.len() + 1);
            if next == 0 {
                app.bus
                    .set_fan_auto(&fan.id)
                    .map(|_| format!("{}: automatic.", fan.label))
            } else {
                let (n, c) = FAN_CURVES[next - 1];
                app.bus
                    .set_fan_curve(&fan.id, &c)
                    .map(|_| format!("{}: {n} curve.", fan.label))
            }
        }
    };
    report(app, r);
    app.cooling.invalidate();
}

fn battery(app: &mut App, f: &mut Frame, r: Rect) {
    let Some(b) = app.battery.value.clone() else {
        let msg = app
            .battery
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, app.theme.muted())), r);
        return;
    };
    let t = app.theme;
    let mut lines = vec![Line::from(Span::styled(
        "When the laptop mostly runs plugged in, keeping the battery at 100 % wears it out fastest. With protection on, charging stops at a limit and the laptop runs from the adapter.",
        t.muted(),
    ))];
    if b.capacity <= 100 {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Charge now  ", t.muted()),
            Span::styled(format!("{} %  ", b.capacity), t.title()),
            Span::styled(b.status.clone(), t.muted()),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }),
        Rect { height: 5, ..r },
    );
    if !b.supported {
        f.render_widget(
            Paragraph::new(Span::styled(b.reason.clone(), app.theme.warn_style()))
                .wrap(Wrap { trim: true }),
            Rect {
                y: r.y + 6,
                height: 3,
                ..r
            },
        );
        return;
    }
    let modes = [
        (
            "off",
            "Off (charge to 100 %)",
            "Full runtime every day; a battery kept full and warm while plugged in loses capacity fastest.",
        ),
        (
            "protect",
            "Protect",
            "Charging stops at the limit and the laptop runs from the adapter: a little less runtime, a much longer life.",
        ),
        (
            "full-once",
            "Full charge once, then protect",
            "100 % this one time for a trip; the limit comes back by itself as soon as the battery is full.",
        ),
    ];
    let mut y = r.y + 6;
    for (i, (id, label, help)) in modes.iter().enumerate() {
        let on = b.mode == *id;
        let line = Line::from(vec![
            Span::styled(
                if on { "(•) " } else { "( ) " },
                Style::default().fg(if on {
                    app.theme.accent
                } else {
                    app.theme.faint
                }),
            ),
            Span::styled(*label, app.theme.text()),
        ]);
        row(
            app,
            f,
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: 1,
            },
            i,
            line,
        );
        f.render_widget(
            Paragraph::new(Span::styled(*help, app.theme.faint())).wrap(Wrap { trim: true }),
            Rect {
                x: r.x + 4,
                y: y + 1,
                width: r.width.saturating_sub(4),
                height: 2.min(r.bottom().saturating_sub(y + 1)),
            },
        );
        y += 3;
    }
    app.rows = modes.len();
    if b.adjustable {
        let limits: Vec<String> = (50..=100).step_by(5).map(|l| format!("{l}%")).collect();
        let chosen = app.choice.min(limits.len() - 1);
        choices(
            app,
            f,
            Rect {
                x: r.x,
                y,
                width: r.width,
                height: 1,
            },
            "Stop at",
            &limits,
            chosen,
        );
    } else {
        f.render_widget(
            Paragraph::new(Span::styled("This laptop uses the manufacturer's conservation mode; the firmware sets the limit (usually 60–80 %).", app.theme.faint()))
                .wrap(Wrap { trim: true }),
            Rect { x: r.x, y, width: r.width, height: 2 },
        );
    }
    let y = y + 3;
    if y + 2 < r.bottom() {
        battery_today(
            app,
            f,
            Rect {
                y,
                height: r.bottom() - y,
                ..r
            },
        );
    }
}

/// Today on battery and on the charger, and the health estimates.
fn battery_today(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some(rep) = app.bat_report.value.clone() else {
        return;
    };
    let hm = |m: u32| format!("{}:{:02}", m / 60, m % 60);
    // local_time is "Thu 10:14"; the clock is its last five characters.
    let hhmm = |ms: u64| {
        let s = data::local_time(ms);
        s.get(s.len().saturating_sub(5)..)
            .unwrap_or_default()
            .to_string()
    };
    let mut lines = vec![
        Line::from(Span::styled("Today", t.title())),
        Line::from(vec![
            Span::styled("On battery ", t.muted()),
            Span::styled(hm(rep.on_battery_min), t.text()),
            Span::styled("   charger ", t.muted()),
            Span::styled(hm(rep.on_ac_min), t.text()),
            Span::styled("   screen on (battery) ", t.muted()),
            Span::styled(hm(rep.screen_on_battery_min), t.text()),
        ]),
    ];
    if rep.avg_battery_w.is_finite() {
        let mut l = format!(
            "Average draw {:.1} W, {:.1} Wh used ({:.0} points)",
            rep.avg_battery_w, rep.used_wh, rep.used_pct
        );
        if rep.full_runtime_min > 0 {
            l += &format!(
                "; a full charge lasts about {} at that draw",
                hm(rep.full_runtime_min)
            );
        }
        lines.push(Line::from(Span::styled(l, t.muted())));
    }
    for p in rep
        .periods
        .iter()
        .rev()
        .filter(|p| p.end_unix_ms - p.start_unix_ms >= 60_000)
        .take(4)
    {
        let what = match p.kind.as_str() {
            "battery" => "battery",
            "ac" => "charger",
            _ => "off",
        };
        let mut l = format!(
            "  {}-{}  {:<8}",
            hhmm(p.start_unix_ms),
            hhmm(p.end_unix_ms),
            what
        );
        if p.kind != "off" && p.start_pct.is_finite() {
            l += &format!(" {:.0}% → {:.0}%", p.start_pct, p.end_pct);
        }
        if p.kind == "battery" && p.avg_w.is_finite() {
            l += &format!("  {:.1} W", p.avg_w);
        }
        lines.push(Line::from(Span::styled(l, t.faint())));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Battery health", t.title())));
    for h in &rep.health {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<12}", h.method), t.text()),
            Span::styled(format!("{:>4.0} %  ", h.pct), t.accent()),
            Span::styled(format!("({} confidence) ", h.confidence), t.muted()),
            Span::styled(h.basis.clone(), t.faint()),
        ]));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), r);
}

fn monitoring(app: &mut App, f: &mut Frame, r: Rect) {
    tabs(
        app,
        f,
        Rect { height: 1, ..r },
        &["Temperature and power", "Internet traffic", "FPS", "Alerts"],
    );
    let body = Rect {
        y: r.y + 2,
        height: r.height.saturating_sub(2),
        ..r
    };
    if app.tab == 3 {
        alerts_page(app, f, body);
        return;
    }
    if app.tab == 2 {
        fps(app, f, body);
        return;
    }
    if app.tab == 0 {
        let names: Vec<String> = RANGES.iter().map(|s| s.to_string()).collect();
        let chosen = app.choice.min(3);
        choices(app, f, Rect { height: 1, ..body }, "Range", &names, chosen);
        if chosen == 0 {
            live(
                app,
                f,
                Rect {
                    y: body.y + 2,
                    height: body.height.saturating_sub(2),
                    ..body
                },
            );
            return;
        }
        let pts = app.history.value.clone().unwrap_or_default();
        let area = Rect {
            y: body.y + 2,
            height: body.height.saturating_sub(2),
            ..body
        };
        if pts.len() < 2 {
            let msg = match &app.history.error {
                Some(e) => e.clone(),
                None => "Not enough history for this range yet. The agent records a reading every minute; charts fill in as it runs.".into(),
            };
            f.render_widget(
                Paragraph::new(Span::styled(msg, app.theme.muted())).wrap(Wrap { trim: true }),
                area,
            );
            return;
        }
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(1, 3); 3])
            .split(area);
        let t0 = pts[0].ts_unix_ms as f64 / 1000.0;
        let series = |get: &dyn Fn(&ug_proto::HistoryPoint) -> f64| -> Vec<(f64, f64)> {
            pts.iter()
                .filter(|p| !get(p).is_nan())
                .map(|p| (p.ts_unix_ms as f64 / 1000.0 - t0, get(p)))
                .collect()
        };
        let span = (pts
            .last()
            .map(|p| p.ts_unix_ms as f64 / 1000.0 - t0)
            .unwrap_or(60.0))
        .max(60.0);
        // Title, unit, y range, then the two lines.
        type ChartSpec<'a> = (
            &'a str,
            &'a str,
            (f64, f64),
            Vec<(f64, f64)>,
            Vec<(f64, f64)>,
        );
        let charts: [ChartSpec; 3] = [
            (
                "Temperature",
                "°",
                (30.0, 70.0),
                series(&|p| p.cpu_temp_c),
                series(&|p| p.gpu_temp_c),
            ),
            (
                "Load",
                " %",
                (0.0, 100.0),
                series(&|p| p.cpu_load_pct),
                series(&|p| p.gpu_load_pct),
            ),
            (
                "Power",
                " W",
                (0.0, 30.0),
                series(&|p| p.cpu_w),
                series(&|p| p.gpu_w),
            ),
        ];
        for (i, (title, unit, (min_lo, min_hi), cpu, gpu)) in charts.iter().enumerate() {
            let lo = cpu
                .iter()
                .chain(gpu)
                .map(|p| p.1)
                .fold(*min_lo, f64::min)
                .floor();
            let hi = cpu
                .iter()
                .chain(gpu)
                .map(|p| p.1)
                .fold(*min_hi, f64::max)
                .ceil();
            let mut sets = vec![Dataset::default()
                .name("Processor")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(app.theme.cpu))
                .data(cpu)];
            if !gpu.is_empty() {
                sets.push(
                    Dataset::default()
                        .name("Graphics card")
                        .marker(Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(app.theme.gpu))
                        .data(gpu),
                );
            }
            let two = sets.len() > 1;
            let chart = Chart::new(sets)
                .legend_position(two.then_some(ratatui::widgets::LegendPosition::TopRight))
                .block(card(app, title))
                .x_axis(Axis::default().bounds([0.0, span]).labels([
                    Span::raw(data::local_time(pts[0].ts_unix_ms)),
                    Span::raw(data::local_time(
                        pts.last().map(|p| p.ts_unix_ms).unwrap_or(0),
                    )),
                ]))
                .y_axis(
                    Axis::default()
                        .style(app.theme.faint())
                        .bounds([lo, hi])
                        .labels([
                            Span::raw(format!("{lo:.0}{unit}")),
                            Span::raw(format!("{hi:.0}{unit}")),
                        ]),
                );
            f.render_widget(chart, parts[i]);
        }
    } else {
        let mut lines = Vec::new();
        if let Some(s) = &app.snapshot.value {
            lines.push(Line::from(vec![
                Span::styled("Now   ↓ ", app.theme.muted()),
                Span::styled(rate(s.net.rx_bps), app.theme.title()),
                Span::styled("   ↑ ", app.theme.muted()),
                Span::styled(rate(s.net.tx_bps), app.theme.title()),
                Span::styled(format!("   {}", s.net.iface), app.theme.faint()),
            ]));
            lines.push(Line::from(""));
        }
        let Some(v) = app.traffic.value.clone() else {
            lines.push(Line::from(Span::styled(
                app.traffic
                    .error
                    .clone()
                    .unwrap_or_else(|| "Reading…".into()),
                app.theme.muted(),
            )));
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
            return;
        };
        lines.push(Line::from(vec![
            Span::styled("Used so far  ", app.theme.muted()),
            Span::styled(size(v.so_far), app.theme.title()),
            Span::styled(
                if v.since.is_empty() {
                    String::new()
                } else {
                    format!("  since {}  (r twice resets the counts)", v.since)
                },
                app.theme.faint(),
            ),
        ]));
        lines.push(Line::from(""));
        let top = body.y + lines.len() as u16;
        let chosen = app.traffic_span.min(2);
        f.render_widget(Paragraph::new(lines), body);
        choices(
            app,
            f,
            Rect {
                y: top,
                height: 1,
                ..body
            },
            "Period",
            &["Day".to_string(), "Week".to_string(), "Month".to_string()],
            chosen,
        );
        let p = &v.periods[chosen];
        let (rx, tx) = p.bars.iter().fold((0, 0), |(r, t), b| (r + b.1, t + b.2));
        let mut lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(format!("{}  ", p.title), app.theme.title()),
                Span::styled(
                    format!("{}  ↓ {}  ↑ {}", size(rx + tx), size(rx), size(tx)),
                    app.theme.muted(),
                ),
            ]),
        ];
        // One column per hour or day: a bar of block characters, three rows high.
        let max = p.bars.iter().map(|b| b.1 + b.2).max().unwrap_or(0).max(1) as f64;
        let col = if p.bars.len() > 24 { 2 } else { 3 };
        const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        for row in (0..3).rev() {
            let cells: String = p
                .bars
                .iter()
                .map(|b| {
                    let eighths = (((b.1 + b.2) as f64 / max) * 24.0).round() as i32 - row * 8;
                    let c = BLOCKS[eighths.clamp(0, 8) as usize];
                    format!("{}{}", c.to_string().repeat(col - 1), ' ')
                })
                .collect();
            lines.push(Line::from(Span::styled(
                cells,
                Style::default().fg(app.theme.cpu),
            )));
        }
        let labels: String = p
            .bars
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let show = p.bars.len() <= 7 || (i + 1) % 3 == 0 || (p.bars.len() > 24 && i == 0);
                let l = if show { b.0.clone() } else { String::new() };
                format!("{l:<col$}")
            })
            .collect();
        lines.push(Line::from(Span::styled(labels, app.theme.faint())));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Apps that used the most",
            app.theme.title(),
        )));
        if p.apps.is_empty() {
            lines.push(Line::from(Span::styled("  None counted in this period (per-app counting needs internet control: Ping stability).", app.theme.muted())));
        }
        for a in p.apps.iter().take(8) {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<26}", a.name), app.theme.text()),
                Span::styled(
                    format!("{:>10}", size(a.rx_bytes + a.tx_bytes)),
                    app.theme.text(),
                ),
                Span::styled(
                    format!("   ↓ {}  ↑ {}", size(a.rx_bytes), size(a.tx_bytes)),
                    app.theme.faint(),
                ),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "←→ day, week or month. More: `ug traffic --apps`, `ug traffic --day YYYY-MM-DD`.",
            app.theme.faint(),
        )));
        let body = Rect {
            y: top + 1,
            height: body.bottom().saturating_sub(top + 1),
            ..body
        };
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    }
}

/// Playtime → Days: hours per day (7 or 30), the hours of the day played,
/// and the figures behind them, games only.
fn playtime_days(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let chosen = app.choice.min(1);
    choices(
        app,
        f,
        Rect { height: 1, ..r },
        "Days",
        &["7 days".to_string(), "30 days".to_string()],
        chosen,
    );
    let Some(all) = app.pt_sessions.value.clone() else {
        let msg = app
            .pt_sessions
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(
            Paragraph::new(Span::styled(msg, t.muted())),
            Rect {
                y: r.y + 2,
                height: 1,
                ..r
            },
        );
        return;
    };
    let n = [7, 30][chosen];
    let now = data::now_ms();
    let first = ug_core::playtime::local_midnight_offset(now, 1 - n as i32);
    let sessions: Vec<_> = all
        .into_iter()
        .filter(|s| s.ended_unix_ms > first)
        .collect();
    let days = ug_core::playtime::per_day(&sessions, now, n);
    let hours = ug_core::playtime::per_hour(&sessions);
    let total: u64 = days.iter().map(|d| d.1).sum();
    let lens: Vec<u64> = sessions
        .iter()
        .map(|s| (s.ended_unix_ms - s.started_unix_ms) / 1000)
        .collect();
    let mut fig = vec![
        Span::styled(" Daily average ", t.muted()),
        Span::styled(hm(total / n as u64), t.text()),
        Span::styled(
            format!(
                "   Days played {}/{n}",
                days.iter().filter(|d| d.1 >= 60).count()
            ),
            t.muted(),
        ),
        Span::styled(
            format!("   Streak {} days", ug_core::playtime::streak(&days)),
            t.muted(),
        ),
    ];
    if !lens.is_empty() {
        fig.push(Span::styled(
            format!(
                "   Average session {}",
                hm(lens.iter().sum::<u64>() / lens.len() as u64)
            ),
            t.muted(),
        ));
        fig.push(Span::styled(
            format!("   Longest {}", hm(*lens.iter().max().unwrap_or(&0))),
            t.muted(),
        ));
    }
    let mut lines = vec![Line::from(fig)];
    let hmax = hours.iter().copied().max().unwrap_or(0);
    if hmax > 0 {
        let peak = hours.iter().position(|h| *h == hmax).unwrap_or(0);
        let bars = " ▁▂▃▄▅▆▇█";
        let spark: String = hours
            .iter()
            .map(|h| {
                bars.chars()
                    .nth((h * 8).div_ceil(hmax) as usize)
                    .unwrap_or(' ')
            })
            .collect();
        lines.push(Line::from(vec![
            Span::styled(
                format!(" When you play  {spark}"),
                Style::default().fg(t.accent),
            ),
            Span::styled(format!("  0–23 h, most around {peak:02}:00"), t.faint()),
        ]));
    }
    lines.push(Line::from(""));
    let most = days.iter().map(|d| d.1).max().unwrap_or(0).max(1);
    let bar_w = (r.width as usize).saturating_sub(24).clamp(6, 60);
    // Newest first, so today is always on screen.
    for (ms, secs) in days.iter().rev() {
        let w = (*secs as usize * bar_w).div_ceil(most as usize);
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {}  ", &ug_core::playtime::local_time(*ms)[..10]),
                t.muted(),
            ),
            Span::styled(format!("{:>9}  ", hm(*secs)), t.text()),
            Span::styled(
                "█".repeat(if *secs == 0 { 0 } else { w.max(1) }),
                Style::default().fg(t.accent),
            ),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            y: r.y + 2,
            height: r.height.saturating_sub(2),
            ..r
        },
    );
}

/// 1.25 GB, 340.2 MB, 12 KB.
fn size(b: u64) -> String {
    match b {
        1_000_000_000.. => format!("{:.2} GB", b as f64 / 1e9),
        1_000_000.. => format!("{:.1} MB", b as f64 / 1e6),
        _ => format!("{} KB", b / 1000),
    }
}

/// One Braille line chart of up to two series against seconds ago.
#[allow(clippy::too_many_arguments)]
fn live_chart(
    app: &App,
    f: &mut Frame,
    r: Rect,
    title: &str,
    unit: &str,
    span: f64,
    min: (f64, f64),
    a: (&str, Vec<(f64, f64)>),
    b: Option<(&str, Vec<(f64, f64)>)>,
) {
    let all = a.1.iter().chain(b.iter().flat_map(|x| x.1.iter()));
    let lo = all.clone().map(|p| p.1).fold(min.0, f64::min).floor();
    let hi = all.map(|p| p.1).fold(min.1, f64::max).ceil();
    let now = a.1.last().map(|p| p.1);
    let head = match now {
        Some(v) if unit.contains("MB") || unit.contains("GB") || unit == " W" => {
            format!("{title}  {v:.1}{unit}")
        }
        Some(v) => format!("{title}  {v:.0}{unit}"),
        None => title.to_string(),
    };
    let mut sets = vec![Dataset::default()
        .name(a.0.to_string())
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(app.theme.cpu))
        .data(&a.1)];
    if let Some((name, d)) = &b {
        if !d.is_empty() {
            sets.push(
                Dataset::default()
                    .name(name.to_string())
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(app.theme.gpu))
                    .data(d),
            );
        }
    }
    let two = sets.len() > 1;
    let chart = Chart::new(sets)
        .legend_position(two.then_some(ratatui::widgets::LegendPosition::TopLeft))
        .block(card(app, &head))
        .x_axis(Axis::default().bounds([-span, 0.0]))
        .y_axis(
            Axis::default()
                .style(app.theme.faint())
                .bounds([lo, hi.max(lo + 1.0)])
                .labels([Span::raw(format!("{lo:.0}")), Span::raw(format!("{hi:.0}"))]),
        );
    f.render_widget(chart, r);
}

/// The last two minutes, one reading a second.
fn live(app: &mut App, f: &mut Frame, r: Rect) {
    if app.live.len() < 2 {
        let msg = app
            .live_due
            .error
            .clone()
            .unwrap_or_else(|| "Collecting readings…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, app.theme.muted())), r);
        return;
    }
    let t_end = app.live.back().map(|(s, _)| s.ts_unix_ms).unwrap_or(0) as f64 / 1000.0;
    let pts =
        |get: &dyn Fn(&ug_proto::Snapshot, &ug_proto::ExtraSample) -> f64| -> Vec<(f64, f64)> {
            app.live
                .iter()
                .filter_map(|(s, e)| {
                    let v = get(s, e);
                    (!v.is_nan()).then_some((s.ts_unix_ms as f64 / 1000.0 - t_end, v))
                })
                .collect()
        };
    let gpu = |s: &ug_proto::Snapshot| {
        s.gpus
            .iter()
            .find(|g| !g.asleep && !g.temp_c.is_nan())
            .cloned()
    };
    let (s, e) = app.live.back().cloned().expect("two readings");

    // A line of numbers, then per-core load as bars.
    let bars = "▁▂▃▄▅▆▇█";
    let cores: String = s
        .cpu
        .per_core_load
        .iter()
        .map(|l| {
            let i = ((l.max(0.0) / 100.0) * 7.0).round() as usize;
            bars.chars().nth(i.min(7)).unwrap_or(' ')
        })
        .collect();
    let bat = if e.battery_pct.is_nan() {
        "no battery".to_string()
    } else {
        let w = if e.battery_w.is_nan() {
            String::new()
        } else {
            format!(" {:.1} W", e.battery_w.abs())
        };
        format!(
            "{:.0} %{w}{}",
            e.battery_pct,
            if e.battery_status == "Charging" {
                " charging"
            } else {
                ""
            }
        )
    };
    let head = vec![
        Line::from(vec![
            Span::styled("Load avg ", app.theme.muted()),
            Span::styled(format!("{:.2}", e.load_avg_1), app.theme.title()),
            Span::styled("   Processes ", app.theme.muted()),
            Span::styled(e.processes.to_string(), app.theme.title()),
            Span::styled("   Disk busy ", app.theme.muted()),
            Span::styled(format!("{:.0} %", e.disk_busy_pct), app.theme.title()),
            Span::styled("   Battery ", app.theme.muted()),
            Span::styled(bat, app.theme.title()),
        ]),
        Line::from(vec![
            Span::styled("Cores ", app.theme.muted()),
            Span::styled(cores, Style::default().fg(app.theme.cpu)),
        ]),
    ];
    f.render_widget(Paragraph::new(head), Rect { height: 2, ..r });
    let area = Rect {
        y: r.y + 3,
        height: r.height.saturating_sub(3),
        ..r
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 4); 4])
        .split(area);
    let cell = |i: usize| {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2); 2])
            .split(rows[i / 2]);
        cols[i % 2]
    };
    let has_gpu = app.live.iter().any(|(s, _)| gpu(s).is_some());
    let g = |get: fn(&ug_proto::GpuSample) -> f64| -> Option<(&'static str, Vec<(f64, f64)>)> {
        has_gpu.then(|| {
            (
                "Graphics card",
                pts(&|s, _| gpu(s).map(|x| get(&x)).unwrap_or(f64::NAN)),
            )
        })
    };
    live_chart(
        app,
        f,
        cell(0),
        "Temperature",
        "°",
        120.0,
        (30.0, 70.0),
        ("Processor", pts(&|s, _| s.cpu.temp_c)),
        g(|x| x.temp_c),
    );
    live_chart(
        app,
        f,
        cell(1),
        "Load",
        " %",
        120.0,
        (0.0, 100.0),
        ("Processor", pts(&|s, _| s.cpu.load_pct)),
        g(|x| x.load_pct),
    );
    live_chart(
        app,
        f,
        cell(2),
        "Power",
        " W",
        120.0,
        (0.0, 30.0),
        ("Processor", pts(&|s, _| s.cpu.power_w)),
        g(|x| x.power_w),
    );
    live_chart(
        app,
        f,
        cell(3),
        "Clock",
        " MHz",
        120.0,
        (0.0, 3000.0),
        ("Fastest core", pts(&|s, _| s.cpu.freq_max_mhz as f64)),
        g(|x| {
            if x.core_clock_mhz == 0 {
                f64::NAN
            } else {
                x.core_clock_mhz as f64
            }
        }),
    );
    let gib = 1_048_576.0;
    live_chart(
        app,
        f,
        cell(4),
        "Memory",
        " GB",
        120.0,
        (0.0, (s.mem.total_kb as f64 / gib).ceil()),
        ("Used", pts(&|s, _| s.mem.used_kb() as f64 / gib)),
        Some(("Swap", pts(&|s, _| s.mem.swap_used_kb as f64 / gib))),
    );
    live_chart(
        app,
        f,
        cell(5),
        "Disk",
        " MB/s",
        120.0,
        (0.0, 5.0),
        ("Read", pts(&|_, e| e.disk_read_bps as f64 / 1e6)),
        Some(("Write", pts(&|_, e| e.disk_write_bps as f64 / 1e6))),
    );
    live_chart(
        app,
        f,
        cell(6),
        "Internet",
        " MB/s",
        120.0,
        (0.0, 1.0),
        ("Down", pts(&|s, _| s.net.rx_bps as f64 / 1e6)),
        Some(("Up", pts(&|s, _| s.net.tx_bps as f64 / 1e6))),
    );
    if e.battery_pct.is_nan() {
        live_chart(
            app,
            f,
            cell(7),
            "Load average",
            "",
            120.0,
            (0.0, 4.0),
            ("1 minute", pts(&|_, e| e.load_avg_1)),
            None,
        );
    } else {
        live_chart(
            app,
            f,
            cell(7),
            "Battery power",
            " W",
            120.0,
            (0.0, 20.0),
            ("Battery", pts(&|_, e| e.battery_w)),
            None,
        );
    }
}

/// The overlay's settings, one row each; ←→ changes the selected one.
const OVERLAY_ROWS: [&str; 6] = [
    "Show",
    "Layout",
    "Position",
    "Size",
    "Background",
    "Remove Unix Guardian's setup",
];

fn overlay_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let mut lines = vec![
        Line::from(Span::styled("Overlay", t.title())),
        Line::from(Span::styled(
            "MangoHud, drawn inside the game; Unix Guardian sets it up and styles it. Changes show in a running game within a second.",
            t.faint(),
        )),
        Line::from(""),
    ];
    let Some(st) = app.overlay.value.clone() else {
        let msg = app
            .overlay
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    };
    if !st.installed {
        lines.push(Line::from(Span::styled(
            "MangoHud is not installed. Install it with:",
            t.text(),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {}", st.install_hint),
            t.accent(),
        )));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), r);
        return;
    }
    if !st.set_up {
        lines.push(Line::from(Span::styled(
            "Set up adds Unix Guardian's block to ~/.config/MangoHud/MangoHud.conf (your file is backed up first).",
            t.text(),
        )));
        lines.push(Line::from(""));
        let h = lines.len() as u16;
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }),
            Rect { height: h + 1, ..r },
        );
        app.rows = 1;
        row(
            app,
            f,
            Rect {
                y: r.y + h + 1,
                height: 1,
                ..r
            },
            0,
            Line::from(Span::styled(" Enter  Set up ", t.accent())),
        );
        return;
    }
    let h = lines.len() as u16;
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }),
        Rect { height: h, ..r },
    );
    let s = &st.settings;
    let values = [
        match s.show.as_str() {
            "gaming" => "only while gaming",
            "always" => "always",
            _ => "hidden (FPS still recorded)",
        }
        .to_string(),
        s.layout.clone(),
        s.position.clone(),
        s.size.clone(),
        format!("{} %", s.opacity_pct),
        "Enter".into(),
    ];
    app.rows = OVERLAY_ROWS.len();
    for (i, (name, v)) in OVERLAY_ROWS.iter().zip(values).enumerate() {
        let line = Line::from(vec![
            Span::styled(format!(" {name:<30}"), t.text()),
            Span::styled(format!("‹ {v} ›"), t.muted()),
        ]);
        row(
            app,
            f,
            Rect {
                y: r.y + h + i as u16,
                height: 1,
                ..r
            },
            i,
            line,
        );
    }
    let y = r.y + h + OVERLAY_ROWS.len() as u16 + 1;
    if y < r.y + r.height {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    "In game, {} shows and hides it (ug overlay set --key Alt_L+o). Load it per game: `mangohud %command%` in Steam.",
                    s.toggle_key
                ),
                t.faint(),
            )),
            Rect { y, height: 1, ..r },
        );
    }
}

/// The Set up progress from Flatpak: the G mark, a bar, the percent and
/// the step; before Flatpak reports any, what it is doing.
fn install_bar(app: &App, st: &ug_proto::RecorderState) -> Line<'static> {
    let t = app.theme;
    if st.setup_step.is_empty() {
        return Line::from(vec![
            Span::styled("G ", t.accent()),
            Span::styled("Asking Flathub what to download…", t.muted()),
        ]);
    }
    const W: usize = 30;
    let filled = (st.setup_pct.min(100) as usize * W).div_ceil(100);
    Line::from(vec![
        Span::styled("G ", t.accent()),
        Span::styled("━".repeat(filled), t.accent()),
        Span::styled("─".repeat(W - filled), t.faint()),
        Span::styled(format!(" {:>3} %", st.setup_pct), t.title()),
        Span::styled(
            format!(
                "  step {}  (a broken download continues on the next Set up)",
                st.setup_step
            ),
            t.faint(),
        ),
    ])
}

/// The recorder's rows: two actions, then the settings.
const RECORDER_ROWS: [&str; 8] = [
    "Recording",
    "Replay buffer",
    "Save the last seconds",
    "Quality",
    "Frame rate",
    "Replay keeps",
    "The game's sound",
    "Microphone",
];

fn recorder_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some(st) = app.recorder.value.clone() else {
        let msg = app
            .recorder
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(
            Paragraph::new(Span::styled(msg, t.muted())).wrap(Wrap { trim: true }),
            r,
        );
        return;
    };
    let line = |y: u16| Rect {
        x: r.x,
        y,
        width: r.width,
        height: 1,
    };
    if st.engine.is_empty() {
        app.rows = if st.can_set_up { 1 } else { 0 };
        let mut lines = vec![
            Line::from(Span::styled("gpu-screen-recorder is not installed.", t.text())),
            Line::from(Span::styled("It encodes on the graphics card, so games lose almost no frames, and records on Wayland too.", t.faint())),
            Line::from(""),
        ];
        if st.mode == "installing" {
            lines.push(Line::from(Span::styled(
                "Installing… this can take a few minutes.",
                t.accent(),
            )));
            lines.push(install_bar(app, &st));
        } else if !st.can_set_up {
            lines.push(Line::from(Span::styled("Flatpak is not installed, so install gpu-screen-recorder with your package manager.", t.muted())));
        }
        if !st.problem.is_empty() {
            lines.push(Line::from(Span::styled(st.problem.clone(), t.warn_style())));
        }
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }),
            Rect {
                height: r.height.saturating_sub(2),
                ..r
            },
        );
        if st.can_set_up && st.mode != "installing" {
            let l = Line::from(Span::styled(
                " Set it up (Flatpak for you only, no password)",
                t.accent(),
            ));
            row(app, f, line(r.y + 5), 0, l);
        }
        return;
    }
    let s = &st.settings;
    let elapsed = if st.since_unix_ms > 0 {
        (ug_driver::apply::now_ms().saturating_sub(st.since_unix_ms)) / 1000
    } else {
        0
    };
    let rec = match st.mode.as_str() {
        "recording" => format!(
            "● recording {}:{:02} — Enter stops and saves",
            elapsed / 60,
            elapsed % 60
        ),
        "replay" => "the replay buffer is running".into(),
        _ => "Enter starts".into(),
    };
    let replay = match st.mode.as_str() {
        "replay" => format!("keeping the last {} s — Enter stops", s.replay_seconds),
        "recording" => "not while recording".into(),
        _ => "Enter starts".into(),
    };
    let save = if st.mode == "replay" {
        "Enter writes them to a file".to_string()
    } else {
        "start the buffer first".into()
    };
    let onoff = |b: bool| if b { "on" } else { "off" };
    let values = [
        rec,
        replay,
        save,
        format!("‹ {} ›", s.quality),
        format!("‹ {} fps ›", s.fps),
        format!(
            "‹ {} s ›  ≈ {} MB of memory",
            s.replay_seconds, st.replay_mb
        ),
        format!("‹ {} ›", onoff(s.audio_output)),
        format!("‹ {} ›", onoff(s.audio_mic)),
    ];
    app.rows = RECORDER_ROWS.len();
    for (i, (name, v)) in RECORDER_ROWS.iter().zip(values.iter()).enumerate() {
        let style = if i == 0 && st.mode == "recording" {
            Style::default().fg(t.danger)
        } else if i < 3 {
            t.accent()
        } else {
            t.text()
        };
        // A blank line between the actions and the settings.
        let y = r.y + i as u16 + u16::from(i >= 3);
        row(
            app,
            f,
            line(y),
            i,
            Line::from(vec![
                Span::styled(format!(" {name:<24}"), t.muted()),
                Span::styled(v.clone(), style),
            ]),
        );
    }
    let mut notes = vec![
        Line::from(Span::styled(
            format!(
                "About {} MB a minute. Capture: {}. Codec: {}.",
                st.mb_per_minute,
                if s.capture.is_empty() {
                    "default (the desktop asks on Wayland)"
                } else {
                    &s.capture
                },
                s.codec
            ),
            t.faint(),
        )),
        Line::from(Span::styled(format!("Folder: {}", st.folder), t.faint())),
    ];
    if st.mode == "recording" {
        notes.push(Line::from(Span::styled(
            format!("Recording to {}", st.file),
            t.text(),
        )));
    }
    if !st.last_saved.is_empty() {
        notes.push(Line::from(Span::styled(
            format!("Last saved: {}", st.last_saved),
            t.accent(),
        )));
    }
    if !st.problem.is_empty() {
        notes.push(Line::from(Span::styled(st.problem.clone(), t.warn_style())));
    }
    notes.push(Line::from(Span::styled(
        "Capture, codec and folder: ug record set --capture … --codec … --folder …",
        t.faint(),
    )));
    let y = r.y + RECORDER_ROWS.len() as u16 + 2;
    if y < r.bottom() {
        f.render_widget(
            Paragraph::new(notes).wrap(Wrap { trim: true }),
            Rect {
                y,
                height: r.bottom() - y,
                ..r
            },
        );
    }
}

fn activate_recorder(app: &mut App, i: usize) {
    let Some(st) = app.recorder.value.clone() else {
        return;
    };
    let what = if st.engine.is_empty() {
        if !st.can_set_up || st.mode == "installing" {
            return;
        }
        "setup"
    } else {
        match (i, st.mode.as_str()) {
            (0, "idle") => "start",
            (0, "recording") | (1, "replay") => "stop",
            (1, "idle") => "replay",
            (2, "replay") => "save",
            (3..=5, _) => return step_recorder(app, i, 1),
            (6, _) | (7, _) => {
                let mut s = st.settings.clone();
                if i == 6 {
                    s.audio_output = !s.audio_output;
                } else {
                    s.audio_mic = !s.audio_mic;
                }
                let r = app.bus.recorder_set(&s).map(|()| "Saved.".to_string());
                report(app, r);
                app.recorder.invalidate();
                return;
            }
            _ => return,
        }
    };
    let r = app.bus.recorder_do(what).map(|file| match what {
        "start" => format!("Recording to {file}"),
        "stop" if file.is_empty() => "Replay buffer stopped.".to_string(),
        "stop" | "save" => format!("Saved {file}"),
        "replay" => {
            "Keeping the last seconds; Enter on \"Save the last seconds\" writes them.".into()
        }
        _ => "Installing gpu-screen-recorder…".into(),
    });
    report(app, r);
    app.recorder.invalidate();
}

/// ←→ on the quality, frame rate and buffer rows.
fn step_recorder(app: &mut App, i: usize, d: i32) {
    let Some(st) = app.recorder.value.clone() else {
        return;
    };
    let mut s = st.settings.clone();
    let step = |list: &[&str], v: &str| -> String {
        let at = list.iter().position(|x| *x == v).unwrap_or(0) as i32;
        list[(at + d).clamp(0, list.len() as i32 - 1) as usize].to_string()
    };
    match i {
        3 => s.quality = step(&["small", "balanced", "high"], &s.quality),
        4 => {
            s.fps = step(&["30", "60", "120"], &s.fps.to_string())
                .parse()
                .unwrap_or(60)
        }
        5 => {
            s.replay_seconds = step(&["15", "30", "60", "120"], &s.replay_seconds.to_string())
                .parse()
                .unwrap_or(60)
        }
        _ => return,
    }
    if s != st.settings {
        let r = app.bus.recorder_set(&s);
        if let Err(e) = r {
            app.say(e, false);
        }
        app.recorder.invalidate();
    }
}

/// Global shortcuts through the desktop's portal: row 0 turns them on or
/// off, the others do what each shortcut does, now; `c` records new keys
/// for one, `x` goes back to the suggested keys.
fn shortcuts_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let lines = vec![
        Line::from(Span::styled(
            "Keys that work anywhere on the desktop, even with a game in front. The desktop owns them: it asks you to confirm them the first time, and they also show in its own shortcut settings.",
            t.faint(),
        )),
        Line::from(""),
    ];
    let Some(st) = app.shortcuts.value.clone() else {
        let msg = app
            .shortcuts
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    };
    let mut h = 0;
    for l in lines {
        let n = (l.width() as u16).div_ceil(r.width.max(1)).max(1);
        f.render_widget(
            Paragraph::new(l).wrap(Wrap { trim: true }),
            Rect {
                y: r.y + h,
                height: n,
                ..r
            },
        );
        h += n;
    }
    let status = match st.status.as_str() {
        "off" => "off",
        "connecting" => "connecting to the desktop…",
        "waiting" => "confirm the keys in the desktop's dialog",
        "active" => "on",
        "unavailable" => "not offered by this desktop",
        _ => "could not bind them",
    };
    // GNOME and XFCE: a row that writes the keys into the desktop's settings.
    let desk = app.desk_keys.clone().filter(|d| {
        (st.status == "unavailable" || st.status == "failed") && (!d.desktop.is_empty() || d.set_up)
    });
    app.rows = 1 + st.shortcuts.len() + desk.is_some() as usize;
    let on = Line::from(vec![
        Span::styled(format!(" {:<34}", "Use global shortcuts"), t.text()),
        Span::styled(
            format!("‹ {} ›", if st.enabled { "on" } else { "off" }),
            t.accent(),
        ),
        Span::styled(format!("  {status}"), t.muted()),
    ]);
    row(
        app,
        f,
        Rect {
            y: r.y + h,
            height: 1,
            ..r
        },
        0,
        on,
    );
    let fallback = (st.status == "unavailable" || st.status == "failed")
        && !desk.as_ref().is_some_and(|d| d.set_up);
    let recording = app.recording.clone();
    for (i, k) in st.shortcuts.iter().enumerate() {
        // Bound: the desktop's keys; not yet: the keys that will be asked for.
        let (keys, style) = match &recording {
            Some((id, meta)) if *id == k.id => (
                format!("{}press the keys…", if *meta { "Meta+" } else { "" }),
                t.accent(),
            ),
            _ if !k.trigger.is_empty() => (k.trigger.clone(), t.accent()),
            _ if k.chosen => (format!("{} (yours)", k.keys), t.muted()),
            _ => (k.keys.clone(), t.muted()),
        };
        let mut spans = vec![
            Span::styled(format!(" {:<34}", k.description), t.text()),
            Span::styled(format!("{keys:<24}"), style),
        ];
        if fallback {
            spans.push(Span::styled(format!("ug shortcut {}", k.id), t.muted()));
        }
        row(
            app,
            f,
            Rect {
                y: r.y + h + 1 + i as u16,
                height: 1,
                ..r
            },
            i + 1,
            Line::from(spans),
        );
    }
    let mut extra = 0;
    if let Some(d) = &desk {
        let name = if d.desktop.is_empty() {
            "the desktop"
        } else {
            &d.desktop
        };
        let line = Line::from(vec![
            Span::styled(
                format!(" {:<34}", format!("Keys in {name}'s own settings")),
                t.text(),
            ),
            Span::styled(
                if d.set_up {
                    "‹ written ›".to_string()
                } else {
                    "‹ set up ›".to_string()
                },
                t.accent(),
            ),
            Span::styled(
                if d.set_up {
                    "  Enter removes them".to_string()
                } else {
                    "  Enter writes them there".to_string()
                },
                t.muted(),
            ),
        ]);
        let i = 1 + st.shortcuts.len();
        row(
            app,
            f,
            Rect {
                y: r.y + h + i as u16,
                height: 1,
                ..r
            },
            i,
            line,
        );
        extra = 1;
    }
    let mut notes = Vec::new();
    if let Some(p) = desk
        .as_ref()
        .map(|d| d.problem.clone())
        .filter(|p| !p.is_empty())
    {
        notes.push(p);
    }
    if st.status == "failed" && !st.problem.is_empty() {
        notes.push(st.problem.clone());
    }
    if fallback {
        notes.push(if desk.is_some() {
            "Bind the commands above in your desktop's own shortcut settings, or let the last row write them there.".into()
        } else {
            "Bind the commands above in your desktop's own shortcut settings instead.".into()
        });
    }
    match st.gamemode_by_hand.as_str() {
        "on" => notes.push("Game mode is on by hand; it stays on until you turn it off.".into()),
        "off" => notes.push("Game mode is off by hand until the running games close.".into()),
        _ => {}
    }
    if recording.is_some() {
        notes.push("Hold Ctrl or Alt and press a key (F1 to F12 work alone). Tab adds or drops Meta, which most terminals cannot see; Esc cancels.".into());
    } else {
        notes.push("Enter on a shortcut runs it now; c picks new keys for it, x goes back to the suggested ones.".into());
    }
    let y = r.y + h + 2 + st.shortcuts.len() as u16 + extra;
    if y < r.bottom() {
        let text: Vec<Line> = notes
            .into_iter()
            .map(|n| Line::from(Span::styled(n, t.faint())))
            .collect();
        f.render_widget(
            Paragraph::new(text).wrap(Wrap { trim: true }),
            Rect {
                y,
                height: r.bottom() - y,
                ..r
            },
        );
    }
}

/// Starts recording keys for shortcut row `i` (row 0 is the switch).
pub fn record_shortcut(app: &mut App, i: usize) {
    let Some(k) = app
        .shortcuts
        .value
        .as_ref()
        .and_then(|s| s.shortcuts.get(i.wrapping_sub(1)))
    else {
        return;
    };
    app.recording = Some((k.id.clone(), false));
}

/// A press while recording: Esc cancels, Tab adds or drops Meta, anything
/// else is the new keys (the agent refuses what can't be a shortcut).
pub fn record_keys(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let Some((id, meta)) = app.recording.clone() else {
        return;
    };
    match code {
        KeyCode::Esc if mods.is_empty() => app.recording = None,
        KeyCode::Tab if mods.is_empty() => app.recording = Some((id, !meta)),
        _ => {
            let Some(keys) = combo(code, mods, meta) else {
                return;
            };
            app.recording = None;
            let r = app.bus.shortcut_set_keys(&id, &keys);
            report(app, r);
            app.shortcuts.invalidate();
        }
    }
}

/// A terminal key press as a shortcut ("Meta+Ctrl+G"), or None for a key a
/// shortcut can't use. Upper-case letters mean Shift.
pub fn combo(code: KeyCode, mods: KeyModifiers, meta: bool) -> Option<String> {
    let mut shift = mods.contains(KeyModifiers::SHIFT);
    let key = match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) if c.is_ascii_uppercase() => {
            shift = true;
            c.to_string()
        }
        KeyCode::Char(c) if c.is_ascii_graphic() => c.to_ascii_uppercase().to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Enter => "Return".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => {
            shift = true;
            "Tab".into()
        }
        KeyCode::Backspace => "BackSpace".into(),
        KeyCode::Esc => "Escape".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "Page_Up".into(),
        KeyCode::PageDown => "Page_Down".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        _ => return None,
    };
    let mut parts = Vec::new();
    if meta || mods.contains(KeyModifiers::SUPER) || mods.contains(KeyModifiers::META) {
        parts.push("Meta");
    }
    if mods.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if mods.contains(KeyModifiers::ALT) {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    parts.push(&key);
    Some(parts.join("+"))
}

fn activate_shortcut(app: &mut App, i: usize) {
    let Some(st) = app.shortcuts.value.clone() else {
        return;
    };
    let r = if i == 0 {
        let on = !st.enabled;
        app.bus.shortcuts_set_enabled(on).map(|_| {
            if on {
                "Asking the desktop for the keys…"
            } else {
                "Global shortcuts off."
            }
            .to_string()
        })
    } else if let Some(k) = st.shortcuts.get(i - 1) {
        app.bus.shortcut_run(&k.id)
    } else if let Some(d) = app
        .desk_keys
        .clone()
        .filter(|_| i == 1 + st.shortcuts.len())
    {
        let r = app.bus.set_up_desktop_keys(!d.set_up);
        app.desk_keys = app.bus.desktop_keys().ok();
        r
    } else {
        return;
    };
    report(app, r);
    app.shortcuts.invalidate();
}

/// A finding's category as a short label.
fn category(c: &str) -> &str {
    match c {
        "graphics" => "Graphics",
        "wifi" => "Wi-Fi",
        "bluetooth" => "Bluetooth",
        "audio" => "Sound",
        "input" => "Input",
        "camera" => "Camera",
        "printing" => "Printing",
        "sleep" => "Sleep",
        "firmware" => "Firmware",
        other => other,
    }
}

/// The driver check: findings as rows, the selected one explained below,
/// and the review of its fix when asked for.
fn driver_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some(rep) = app.driver.value.clone() else {
        f.render_widget(Paragraph::new(Span::styled("Checking…", t.muted())), r);
        return;
    };
    let line = |y: u16| Rect {
        x: r.x,
        y,
        width: r.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Span::styled(format!("Hardware and driver check — {}. Reads files only; nothing changes until you review a fix.", rep.distro), t.faint())),
        line(r.y),
    );
    // The boot-parameter editor needs typing, so it lives in the command.
    let cmdline = rep
        .facts
        .iter()
        .find(|x| x.key == "kernel.cmdline")
        .map(|x| x.value.clone())
        .unwrap_or_default();
    if r.height > 8 {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Kernel parameters: ", t.muted()),
                Span::styled(cmdline, t.text()),
                Span::styled("  (change: ug driver param add|remove <param>)", t.faint()),
            ])),
            line(r.bottom() - 1),
        );
    }
    if rep.findings.is_empty() {
        app.rows = 0;
        f.render_widget(
            Paragraph::new(Span::styled(
                "No problems found. The drivers are set up as they should be.",
                t.accent(),
            )),
            line(r.y + 2),
        );
        return;
    }
    app.rows = rep.findings.len();
    let mut y = r.y + 2;
    for (i, fd) in rep.findings.iter().enumerate() {
        let (label, style) = match fd.severity.as_str() {
            "critical" => ("CRITICAL ", Style::default().fg(t.danger)),
            "important" => ("IMPORTANT", t.warn_style()),
            _ => ("ADVICE   ", t.muted()),
        };
        let auto = if fd.fix_auto { "  · fixable here" } else { "" };
        row(
            app,
            f,
            line(y),
            i,
            Line::from(vec![
                Span::styled(format!(" {label} "), style),
                Span::styled(format!("{:<10}", category(&fd.category)), t.muted()),
                Span::styled(fd.title_en.clone(), t.text()),
                Span::styled(auto, t.faint()),
            ]),
        );
        y += 1;
    }
    y += 1;
    let Some(fd) = rep.findings.get(app.row).cloned() else {
        return;
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("What  ", t.muted()),
            Span::styled(fd.what_en.clone(), t.text()),
        ]),
        Line::from(vec![
            Span::styled("Why   ", t.muted()),
            Span::styled(fd.why_en.clone(), t.text()),
        ]),
        Line::from(vec![
            Span::styled("Fix   ", t.muted()),
            Span::styled(fd.how_en.clone(), t.text()),
        ]),
    ];
    let review = app
        .driver_review
        .as_ref()
        .filter(|(id, _)| *id == fd.fix_id)
        .map(|(_, p)| p.clone());
    match review {
        Some(Ok(p)) if p.refused.is_empty() => {
            lines.push(Line::from(Span::styled(
                "Exactly this will happen (each step is undone if a later one fails):",
                t.accent(),
            )));
            for (n, s) in p.steps.iter().enumerate() {
                lines.push(Line::from(Span::styled(
                    format!(" {}. {} {}", n + 1, s.kind, s.target),
                    t.text(),
                )));
                if s.kind == "write-file" || s.kind == "swap" {
                    for l in s.detail.lines() {
                        lines.push(Line::from(Span::styled(format!("      | {l}"), t.muted())));
                    }
                }
                lines.push(Line::from(Span::styled(
                    format!("      undo: {}", s.undo),
                    t.faint(),
                )));
            }
            if !p.snapshot.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("Safety net: {}", p.snapshot),
                    t.accent(),
                )));
            }
            let after = if p.needs_reboot {
                "; a restart is needed after"
            } else {
                ""
            };
            lines.push(Line::from(Span::styled(
                format!("Enter again applies it (your password is asked{after}). From a text console: sudo ug driver fix ") + &fd.fix_id,
                t.accent(),
            )));
        }
        Some(Ok(p)) => lines.push(Line::from(Span::styled(
            format!("Not available here: {}", p.refused),
            t.warn_style(),
        ))),
        Some(Err(e)) => lines.push(Line::from(Span::styled(e, Style::default().fg(t.danger)))),
        None => {
            if fd.fix_auto {
                lines.push(Line::from(Span::styled(
                    "Enter: review the fix Unix Guardian can make (with a backup and undo).",
                    t.accent(),
                )));
            }
            if !fd.commands.is_empty() {
                lines.push(Line::from(Span::styled(
                    if fd.fix_auto {
                        "Or by hand:"
                    } else {
                        "Commands for this system:"
                    },
                    t.muted(),
                )));
                for c in &fd.commands {
                    lines.push(Line::from(Span::styled(format!("  {c}"), t.text())));
                }
            }
        }
    }
    if y < r.bottom() {
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect {
                y,
                height: r.bottom() - y,
                ..r
            },
        );
    }
}

/// Enter on a finding: the first press shows the plan, the second applies it.
fn activate_driver(app: &mut App, i: usize) {
    let Some(fd) = app
        .driver
        .value
        .as_ref()
        .and_then(|r| r.findings.get(i))
        .cloned()
    else {
        return;
    };
    if fd.fix_id == "reboot" {
        // Two presses, like the power page.
        let confirm = "restart for the driver".to_string();
        if app.armed.as_ref().is_some_and(|(a, _)| *a == confirm) {
            app.armed = None;
            let r = app.bus.power_now("reboot");
            report(app, r.map(|_| "Restart requested.".to_string()));
        } else {
            app.say("Press Enter again to restart now.", true);
            app.armed = Some((confirm, std::time::Instant::now()));
        }
        return;
    }
    if !fd.fix_auto {
        // The service's own reason, the same check "do it" would hit.
        let why = match app.bus.driver_plan(&fd.fix_id) {
            Err(e) => e,
            Ok(_) => {
                "Unix Guardian does not run this fix itself; the commands are shown below.".into()
            }
        };
        report(app, Err(format!("Not automatic here: {why}")));
        return;
    }
    let confirm = format!("apply {}", fd.fix_id);
    let reviewed = app
        .driver_review
        .as_ref()
        .is_some_and(|(id, p)| *id == fd.fix_id && p.as_ref().is_ok_and(|p| p.refused.is_empty()));
    if reviewed && app.armed.as_ref().is_some_and(|(a, _)| *a == confirm) {
        app.armed = None;
        app.driver_review = None;
        let r = app.bus.driver_apply(&fd.fix_id);
        report(
            app,
            r.map(|id| format!("Started {id}; follow it under Changes made (Tab).")),
        );
        app.driver_ops.invalidate();
    } else {
        let p = app.bus.driver_plan(&fd.fix_id);
        app.driver_review = Some((fd.fix_id.clone(), p));
        app.armed = Some((confirm, std::time::Instant::now()));
    }
}

fn driver_ops_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let line = |y: u16| Rect {
        x: r.x,
        y,
        width: r.width,
        height: 1,
    };
    let ops = match (&app.driver_ops.value, &app.driver_ops.error) {
        (Some(v), _) => v.clone(),
        (None, Some(e)) => {
            f.render_widget(
                Paragraph::new(Span::styled(e.clone(), t.muted())).wrap(Wrap { trim: true }),
                Rect { height: 3, ..r },
            );
            return;
        }
        _ => Vec::new(),
    };
    if ops.is_empty() {
        app.rows = 0;
        f.render_widget(
            Paragraph::new(Span::styled("No driver changes have been made.", t.muted())),
            line(r.y),
        );
        return;
    }
    app.rows = ops.len();
    let mut y = r.y;
    let list = (ops.len() as u16).min(r.height / 3).max(1);
    let first = app.row.saturating_sub(list as usize - 1);
    for (i, o) in ops.iter().enumerate().skip(first).take(list as usize) {
        let style = match o.state.as_str() {
            "failed" | "interrupted" => Style::default().fg(t.danger),
            "running" | "undoing" => t.warn_style(),
            _ => t.accent(),
        };
        let state = if o.verify == "check" {
            "check it".to_string()
        } else {
            o.state.clone()
        };
        let style = if o.verify == "check" {
            t.warn_style()
        } else {
            style
        };
        row(
            app,
            f,
            line(y),
            i,
            Line::from(vec![
                Span::styled(format!(" {state:<12}"), style),
                Span::styled(format!("{}  ", o.title), t.text()),
                Span::styled(o.id.clone(), t.faint()),
            ]),
        );
        y += 1;
    }
    y += 1;
    let Some(o) = ops.get(app.row) else { return };
    let mut lines: Vec<Line> = o
        .log
        .iter()
        .map(|l| Line::from(Span::styled(format!("  {l}"), t.text())))
        .collect();
    if o.verify == "check" {
        lines.push(Line::from(Span::styled(
            "Restarted with this change. Does everything work? y keeps it; Enter twice undoes it.",
            t.warn_style(),
        )));
    } else if o.state == "done" || o.state == "interrupted" {
        lines.push(Line::from(Span::styled(
            "Enter twice undoes this change. From a text console: sudo ug driver rollback",
            t.accent(),
        )));
    }
    if y < r.bottom() {
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect {
                y,
                height: r.bottom() - y,
                ..r
            },
        );
    }
}

/// Keeps a change checked after its restart.
pub fn keep_driver_op(app: &mut App, i: usize) {
    let Some(o) = app
        .driver_ops
        .value
        .as_ref()
        .and_then(|v| v.get(i))
        .cloned()
    else {
        return;
    };
    if o.verify != "check" {
        return;
    }
    let r = app.bus.driver_keep(&o.id);
    report(app, r.map(|_| format!("{} kept.", o.title)));
    app.driver_ops.invalidate();
}

fn activate_driver_op(app: &mut App, i: usize) {
    let Some(o) = app
        .driver_ops
        .value
        .as_ref()
        .and_then(|v| v.get(i))
        .cloned()
    else {
        return;
    };
    if o.state != "done" && o.state != "interrupted" {
        return;
    }
    let confirm = format!("undo {}", o.id);
    if app.armed.as_ref().is_some_and(|(a, _)| *a == confirm) {
        app.armed = None;
        let r = app.bus.driver_rollback(&o.id);
        report(
            app,
            r.map(|_| format!("{} undone. Restart for it to take effect.", o.title)),
        );
        app.driver_ops.invalidate();
        app.driver.invalidate();
    } else {
        app.armed = Some((confirm, std::time::Instant::now()));
    }
}

/// The settings rows, grouped under headings in the order of the window's
/// Settings page; then what this installation is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SettingsRow {
    Tray,
    CloseWindow,
    Notifications,
    CheckUpdates,
    ScanInterval,
    Shortcuts,
    ResetTraffic,
}
const SETTINGS_GROUPS: [(&str, &[SettingsRow]); 5] = [
    (
        "Window and tray",
        &[SettingsRow::Tray, SettingsRow::CloseWindow],
    ),
    (
        "Notifications and updates",
        &[SettingsRow::Notifications, SettingsRow::CheckUpdates],
    ),
    ("Finding games", &[SettingsRow::ScanInterval]),
    ("Global shortcuts", &[SettingsRow::Shortcuts]),
    ("Internet use", &[SettingsRow::ResetTraffic]),
];

/// The selectable rows, in order (what `app.row` counts).
fn settings_rows() -> impl Iterator<Item = SettingsRow> {
    SETTINGS_GROUPS
        .iter()
        .flat_map(|(_, rows)| rows.iter().copied())
}

fn settings_row_name(r: SettingsRow) -> &'static str {
    match r {
        SettingsRow::Tray => "Tray icon",
        SettingsRow::CloseWindow => "Close the window when a game starts",
        SettingsRow::Notifications => "Notifications",
        SettingsRow::CheckUpdates => "Look for updates daily",
        SettingsRow::ScanInterval => "Look for games every",
        SettingsRow::Shortcuts => "Global shortcuts",
        SettingsRow::ResetTraffic => "Reset the internet-use counts",
    }
}
const SCAN_STEPS: [u32; 4] = [1, 2, 5, 10];

fn settings_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    f.render_widget(
        Paragraph::new(Span::styled("Settings", t.title())),
        Rect { height: 1, ..r },
    );
    let Some(st) = app.settings.value.clone() else {
        let msg = app
            .settings
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(
            Paragraph::new(Span::styled(msg, t.muted())).wrap(Wrap { trim: true }),
            Rect {
                y: r.y + 2,
                height: 3,
                ..r
            },
        );
        return;
    };
    let s = &st.settings;
    let yes = |b: bool| if b { "on" } else { "off" };
    let shortcuts = match app.shortcuts.value.as_ref() {
        Some(k) if k.enabled && k.status == "active" => "on".to_string(),
        Some(k) if k.enabled => format!("on ({})", k.status),
        Some(_) => "off".to_string(),
        None => "—".to_string(),
    };
    let traffic_armed = app
        .armed
        .as_ref()
        .is_some_and(|(a, _)| a == "reset the internet-use counts");
    let tray = app.tray.value;
    let value = |r: SettingsRow| -> String {
        match r {
            SettingsRow::Tray => match tray {
                Some(true) => "shown".into(),
                Some(false) => "hidden".into(),
                None => "—".into(),
            },
            SettingsRow::CloseWindow => yes(s.close_window_on_game).into(),
            SettingsRow::Notifications => yes(s.notifications).into(),
            SettingsRow::CheckUpdates => yes(s.check_updates).into(),
            SettingsRow::ScanInterval if st.scan_interval_fixed => {
                format!("{} s (set by ug-agent --scan-interval)", s.scan_interval_s)
            }
            SettingsRow::ScanInterval => format!("{} s", s.scan_interval_s),
            SettingsRow::Shortcuts => shortcuts.clone(),
            SettingsRow::ResetTraffic if traffic_armed => "Enter again to reset".into(),
            SettingsRow::ResetTraffic => "Enter twice".into(),
        }
    };
    app.rows = settings_rows().count();
    let mut y = r.y + 2;
    let mut i = 0;
    for (heading, rows) in SETTINGS_GROUPS {
        if y >= r.bottom() {
            break;
        }
        f.render_widget(
            Paragraph::new(Span::styled(heading, t.muted())),
            Rect { y, height: 1, ..r },
        );
        y += 1;
        for &sr in rows {
            if y >= r.bottom() {
                break;
            }
            let line = Line::from(vec![
                Span::styled(format!("   {:<38}", settings_row_name(sr)), t.text()),
                Span::styled(format!("‹ {} ›", value(sr)), t.muted()),
            ]);
            row(app, f, Rect { y, height: 1, ..r }, i, line);
            y += 1;
            i += 1;
        }
    }
    let note = match settings_rows().nth(app.row) {
        Some(SettingsRow::Notifications) => "When a game is found, when a background download could be limited, and when a shortcut is used.",
        Some(SettingsRow::CloseWindow) => "The window's web view gives its memory back to the game; this terminal interface stays open.",
        Some(SettingsRow::ScanInterval) => "Sooner means game mode starts sooner, for a little more CPU time.",
        Some(SettingsRow::CheckUpdates) => "Once a day Unix Guardian asks GitHub for a newer version and tells you once. It never installs anything by itself.",
        Some(SettingsRow::Tray) => "Hidden, it comes back from here or with `ug tray`; Unix Guardian keeps running either way.",
        Some(SettingsRow::ResetTraffic) => "Starts the internet-use counts again from now: the total so far, the day, week and month charts and the per-app list. The speed charts are kept.",
        _ => "Keys that work anywhere; the desktop asks to confirm them. Overlay → Global shortcuts shows and changes the keys.",
    };
    let service = app
        .daemon_version
        .value
        .clone()
        .unwrap_or_else(|| "not reachable".into());
    let about = vec![
        Line::from(Span::styled(note, t.faint())),
        Line::from(""),
        Line::from(vec![Span::styled(" Agent    ", t.muted()), Span::styled(st.agent_version.clone(), t.text())]),
        Line::from(vec![Span::styled(" Service  ", t.muted()), Span::styled(service, t.text())]),
        Line::from(vec![Span::styled(" Files    ", t.muted()), Span::styled(format!("{}  {}", st.config_dir, st.data_dir), t.text())]),
        Line::from(""),
        Line::from(Span::styled(
            "This interface is in English; the window can be switched to Persian on its Settings page. Free software under the GNU GPL, version 3 or later.",
            t.faint(),
        )),
    ];
    let y = y + 1;
    if y < r.bottom() {
        f.render_widget(
            Paragraph::new(about).wrap(Wrap { trim: true }),
            Rect {
                y,
                height: r.bottom() - y,
                ..r
            },
        );
    }
}

/// The Updates page's actions, in order, for the state shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UpdateAction {
    Check,
    Install,
    Rollback,
}

fn update_actions(u: &ug_proto::UpdateState) -> Vec<UpdateAction> {
    let mut a = vec![UpdateAction::Check];
    if u.step.is_empty() {
        if u.can_install {
            a.push(UpdateAction::Install);
        }
        if !u.previous.is_empty() {
            a.push(UpdateAction::Rollback);
        }
    }
    a
}

/// This version, the last check, the new release and how to get it.
fn updates_page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    f.render_widget(
        Paragraph::new(Span::styled("Updates", t.title())),
        Rect { height: 1, ..r },
    );
    let Some(u) = app.update.value.clone() else {
        let msg = app
            .update
            .error
            .clone()
            .unwrap_or_else(|| "Reading…".into());
        f.render_widget(
            Paragraph::new(Span::styled(msg, t.muted())).wrap(Wrap { trim: true }),
            Rect {
                y: r.y + 2,
                height: 3,
                ..r
            },
        );
        return;
    };
    let mut head = vec![Line::from(vec![
        Span::styled(" This version   ", t.muted()),
        Span::styled(u.current.clone(), t.text()),
    ])];
    let checked = if u.checked_unix_ms == 0 {
        "not yet".to_string()
    } else {
        ago(u.checked_unix_ms)
    };
    head.push(Line::from(vec![
        Span::styled(" Last checked   ", t.muted()),
        Span::styled(checked, t.text()),
    ]));
    if !u.previous.is_empty() {
        head.push(Line::from(vec![
            Span::styled(" Previous       ", t.muted()),
            Span::styled(u.previous.clone(), t.text()),
        ]));
    }
    let status = if !u.step.is_empty() {
        Span::styled(
            if u.step == "downloading" {
                " Downloading the new version…"
            } else {
                " Checking its signature and installing; your password is asked for."
            },
            t.text(),
        )
    } else if !u.problem.is_empty() {
        Span::styled(format!(" {}", u.problem), t.warn_style())
    } else if !u.result.is_empty() {
        Span::styled(format!(" {}", u.result), t.text())
    } else if u.checked_unix_ms != 0 && u.latest.is_empty() {
        Span::styled(" Unix Guardian is up to date.", t.text())
    } else {
        Span::raw("")
    };
    head.push(Line::from(status));
    let mut y = r.y + 2;
    let h = head.len() as u16;
    f.render_widget(
        Paragraph::new(head),
        Rect {
            y,
            height: h.min(r.bottom().saturating_sub(y)),
            ..r
        },
    );
    y += h + 1;

    let actions = update_actions(&u);
    app.rows = actions.len();
    for (i, a) in actions.iter().enumerate() {
        if y >= r.bottom() {
            return;
        }
        let label = match a {
            UpdateAction::Check => "Check now".to_string(),
            UpdateAction::Install => format!(
                "Install {}",
                if u.name.is_empty() {
                    &u.latest
                } else {
                    &u.name
                }
            ),
            UpdateAction::Rollback => format!("Go back to {}", u.previous),
        };
        row(
            app,
            f,
            Rect { y, height: 1, ..r },
            i,
            Line::from(Span::styled(format!(" {label}"), t.text())),
        );
        y += 1;
    }
    y += 1;

    let mut body: Vec<Line> = Vec::new();
    match actions.get(app.row) {
        Some(UpdateAction::Install) => body.push(Line::from(Span::styled(
            "The download is checked against Unix Guardian's signing key before anything is installed; the version you have now is kept, so you can go back.",
            t.faint(),
        ))),
        Some(UpdateAction::Rollback) => {
            body.push(Line::from(Span::styled("Puts the version from before the last update back; your password is asked for.", t.faint())))
        }
        _ => body.push(Line::from(Span::styled(
            "Looks on GitHub now. Once a day this happens by itself (Settings); nothing is installed without you.",
            t.faint(),
        ))),
    }
    if !u.latest.is_empty() {
        body.push(Line::from(""));
        let mut title = vec![Span::styled(
            format!(
                " New version: {}",
                if u.name.is_empty() {
                    &u.latest
                } else {
                    &u.name
                }
            ),
            t.title(),
        )];
        if u.published.len() >= 10 {
            title.push(Span::styled(
                format!("  ({})", &u.published[..10]),
                t.muted(),
            ));
        }
        body.push(Line::from(title));
        if u.step.is_empty() && !u.can_install {
            let how = match u.kind.as_str() {
                "package" | "development" => format!(" Update this copy with: {}", u.how),
                "tarball" => " This release has no signed download for this install; get it from the release page and run its install.sh.".into(),
                _ => " This copy runs from a build folder; build the new version from source.".into(),
            };
            body.push(Line::from(Span::styled(how, t.warn_style())));
        }
        for l in u
            .notes
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
        {
            let l = l.trim_start();
            let text = if let Some(h) = l.strip_prefix('#') {
                Span::styled(format!(" {}", h.trim_start_matches('#').trim()), t.text())
            } else if let Some(i) = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
                Span::styled(
                    format!("   • {}", i.replace("**", "").replace('`', "")),
                    t.muted(),
                )
            } else {
                Span::styled(
                    format!(" {}", l.replace("**", "").replace('`', "")),
                    t.muted(),
                )
            };
            body.push(Line::from(text));
        }
        if !u.url.is_empty() {
            body.push(Line::from(vec![
                Span::styled(" Release page   ", t.muted()),
                Span::styled(u.url.clone(), t.text()),
            ]));
        }
    }
    if y < r.bottom() {
        f.render_widget(
            Paragraph::new(body).wrap(Wrap { trim: false }),
            Rect {
                y,
                height: r.bottom() - y,
                ..r
            },
        );
    }
}

/// "5 minutes ago" for a time in the past.
fn ago(ms: u64) -> String {
    let m = data::now_ms().saturating_sub(ms) / 60_000;
    match m {
        0 => "just now".into(),
        1..=59 => format!("{m} minutes ago"),
        60..=1439 => format!("{} hours ago", m / 60),
        _ => format!("{} days ago", m / 1440),
    }
}

/// Enter on an Updates row.
fn activate_update(app: &mut App, i: usize) {
    let Some(u) = app.update.value.clone() else {
        return;
    };
    let r = match update_actions(&u).get(i) {
        Some(UpdateAction::Check) => app.bus.update_check().and_then(|s| {
            let r = if !s.problem.is_empty() {
                Err(s.problem.clone())
            } else if s.latest.is_empty() {
                Ok("Unix Guardian is up to date.".to_string())
            } else {
                Ok(format!(
                    "{} is out.",
                    if s.name.is_empty() {
                        &s.latest
                    } else {
                        &s.name
                    }
                ))
            };
            app.update.store(Ok(s));
            r
        }),
        Some(UpdateAction::Install) => app
            .bus
            .update_install()
            .map(|_| "Downloading; the service will ask for your password.".to_string()),
        Some(UpdateAction::Rollback) => app.bus.update_rollback(),
        None => return,
    };
    report(app, r);
    app.update.invalidate();
}

/// ←→ or Enter on a settings row.
fn step_settings(app: &mut App, d: i32) {
    let Some(st) = app.settings.value.clone() else {
        return;
    };
    let mut s = st.settings.clone();
    let Some(which) = settings_rows().nth(app.row) else {
        return;
    };
    let r = match which {
        SettingsRow::Notifications => {
            s.notifications = !s.notifications;
            app.bus.settings_set(&s).map(|_| {
                format!(
                    "Notifications {}.",
                    if s.notifications { "on" } else { "off" }
                )
            })
        }
        SettingsRow::CloseWindow => {
            s.close_window_on_game = !s.close_window_on_game;
            app.bus.settings_set(&s).map(|_| "Saved.".to_string())
        }
        SettingsRow::ScanInterval if st.scan_interval_fixed => {
            Err("Set by ug-agent --scan-interval in the service; change it there.".to_string())
        }
        SettingsRow::ScanInterval => {
            let i = SCAN_STEPS
                .iter()
                .position(|v| *v >= s.scan_interval_s)
                .unwrap_or(1) as i32;
            s.scan_interval_s = SCAN_STEPS[(i + d).clamp(0, SCAN_STEPS.len() as i32 - 1) as usize];
            app.bus
                .settings_set(&s)
                .map(|_| format!("Looking for games every {} s.", s.scan_interval_s))
        }
        SettingsRow::CheckUpdates => {
            s.check_updates = !s.check_updates;
            app.bus.settings_set(&s).map(|_| {
                format!(
                    "Daily update check {}.",
                    if s.check_updates { "on" } else { "off" }
                )
            })
        }
        SettingsRow::Tray => {
            let on = app.tray.value != Some(true);
            let r = app.bus.tray_set(on).map(|_| {
                if on {
                    "Tray icon shown."
                } else {
                    "Tray icon hidden."
                }
                .to_string()
            });
            app.tray.invalidate();
            r
        }
        SettingsRow::Shortcuts => {
            let on = !app.shortcuts.value.as_ref().is_some_and(|k| k.enabled);
            let r = app.bus.shortcuts_set_enabled(on).map(|_| {
                if on {
                    "Asking the desktop for the keys…"
                } else {
                    "Global shortcuts off."
                }
                .to_string()
            });
            app.shortcuts.invalidate();
            r
        }
        // Asks twice, like r on Monitoring → Internet traffic.
        SettingsRow::ResetTraffic => {
            reset_traffic(app);
            return;
        }
    };
    report(app, r);
    app.settings.invalidate();
}

/// ←→ on an overlay row.
fn step_overlay(app: &mut App, d: i32) {
    let Some(st) = app.overlay.value.clone() else {
        return;
    };
    if !st.set_up {
        return;
    }
    let mut s = st.settings.clone();
    let cycle = |cur: &str, all: &[&str]| -> String {
        let i = all.iter().position(|x| *x == cur).unwrap_or(0) as i32;
        all[((i + d).rem_euclid(all.len() as i32)) as usize].to_string()
    };
    match app.row {
        0 => s.show = cycle(&s.show, &["gaming", "always", "off"]),
        1 => s.layout = cycle(&s.layout, &["card", "compact"]),
        2 => {
            s.position = cycle(
                &s.position,
                &[
                    "top-left",
                    "top-center",
                    "top-right",
                    "bottom-right",
                    "bottom-left",
                ],
            )
        }
        3 => s.size = cycle(&s.size, &["small", "medium", "large"]),
        4 => s.opacity_pct = (s.opacity_pct as i32 + 10 * d).clamp(0, 100) as u8,
        _ => return,
    }
    let r = app.bus.overlay_set(&s);
    report(app, r.map(|_| "Saved.".to_string()));
    app.overlay.invalidate();
}

/// FPS from MangoHud, or what to do to get it.
fn fps(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some(fr) = app.fps.value.clone() else {
        let msg = app.fps.error.clone().unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "Linux has no system-wide FPS counter: frames are counted inside each game. Unix Guardian reads them from MangoHud's frame log.",
            t.faint(),
        )),
        Line::from(""),
    ];
    app.rows = 0;
    match fr.state.as_str() {
        "not-installed" => {
            lines.push(Line::from(Span::styled(
                "MangoHud is not installed. Install it with:",
                t.text(),
            )));
            lines.push(Line::from(Span::styled(
                format!("  {}", fr.install_hint),
                t.accent(),
            )));
        }
        "not-set-up" => {
            lines.push(Line::from(Span::styled(
                "MangoHud is installed. Set up adds a marked block to ~/.config/MangoHud/MangoHud.conf (your file is backed up first); Undo removes exactly that block.",
                t.text(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(" Enter  Set up ", t.selected())));
            app.rows = 1;
        }
        "idle" => {
            lines.push(Line::from(Span::styled("Waiting for a game running with MangoHud. In Steam, set a game's launch options to:", t.text())));
            lines.push(Line::from(Span::styled("  mangohud %command%", t.accent())));
            lines.push(Line::from(Span::styled(
                "Other launchers: start the game with MANGOHUD=1 or through the mangohud command.",
                t.faint(),
            )));
            if fr.live_unsupported {
                lines.push(Line::from(Span::styled(
                    format!("MangoHud {} writes its frame log only when the game closes: no live FPS, but each session is recorded below.", fr.mangohud_version),
                    t.warn_style(),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Enter  Undo the setup ",
                t.muted(),
            )));
            app.rows = 1;
        }
        _ => {
            lines.push(Line::from(vec![
                Span::styled("Now playing ", t.muted()),
                Span::styled(fr.game.clone(), t.title()),
                Span::styled("    FPS ", t.muted()),
                Span::styled(format!("{:.0}", fr.fps), t.accent()),
                Span::styled("    1% low ", t.muted()),
                Span::styled(format!("{:.0}", fr.low_1pct_fps), t.title()),
                Span::styled("    Frame time ", t.muted()),
                Span::styled(format!("{:.1} ms", fr.frametime_ms), t.title()),
            ]));
        }
    }
    let h = lines.len() as u16 + 1;
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }),
        Rect {
            height: h.min(r.height),
            ..r
        },
    );
    if fr.state == "live" && fr.recent.len() >= 2 {
        let end = fr.recent.last().map(|p| p.0).unwrap_or(0) as f64 / 1000.0;
        let d: Vec<(f64, f64)> = fr
            .recent
            .iter()
            .map(|(ts, v)| (*ts as f64 / 1000.0 - end, *v))
            .collect();
        let hi = d.iter().map(|p| p.1).fold(60.0, f64::max).ceil();
        let chart = Chart::new(vec![Dataset::default()
            .name("FPS")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(t.cpu))
            .data(&d)])
        .block(card(app, "FPS, last ten seconds"))
        .x_axis(Axis::default().bounds([-10.0, 0.0]))
        .y_axis(
            Axis::default()
                .style(t.faint())
                .bounds([0.0, hi])
                .labels([Span::raw("0"), Span::raw(format!("{hi:.0}"))]),
        );
        f.render_widget(
            chart,
            Rect {
                y: r.y + h + 1,
                height: r.height.saturating_sub(h + 1).min(16),
                ..r
            },
        );
    }
    // Finished sessions of the last week, under whatever is above.
    let used = if fr.state == "live" && fr.recent.len() >= 2 {
        h + 1 + r.height.saturating_sub(h + 1).min(16)
    } else {
        h
    };
    let y = r.y + used + 1;
    if y + 2 >= r.y + r.height {
        return;
    }
    let sessions = app.sessions.value.clone().unwrap_or_default();
    let mut lines = vec![Line::from(Span::styled(
        "Game sessions this week",
        t.title(),
    ))];
    if sessions.is_empty() {
        lines.push(Line::from(Span::styled(
            "None yet: recorded when a game runs with MangoHud and the overlay set up.",
            t.faint(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                " {:<22} {:<10} {:>5} {:>7} {:>7} {:>9}  ",
                "game", "started", "min", "avg", "1% low", "0.1% low"
            ),
            t.faint(),
        )));
    }
    for s in sessions
        .iter()
        .take((r.y + r.height - y).saturating_sub(3) as usize)
    {
        let smooth = s.stutters as f64 <= (s.duration_s as f64 / 600.0).max(1.0);
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    " {:<22} {:<10} {:>5} ",
                    s.game.chars().take(22).collect::<String>(),
                    data::local_time(s.started_unix_ms),
                    s.duration_s / 60
                ),
                t.text(),
            ),
            Span::styled(
                format!(
                    "{:>7.0} {:>7.0} {:>9.0}  ",
                    s.avg_fps, s.low_1pct_fps, s.low_01pct_fps
                ),
                t.title(),
            ),
            if smooth {
                Span::styled("smooth", Style::default().fg(t.ok))
            } else {
                Span::styled(format!("{} stutters", s.stutters), t.warn_style())
            },
        ]));
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            y,
            height: r.y + r.height - y,
            ..r
        },
    );
}

/// Apps worth listing as background tasks: a known kind that is moving
/// data, anything uploading 125 kB/s, or anything with a rule.
fn is_background(a: &ug_proto::AppTraffic) -> bool {
    !a.has_game
        && a.kind != "session"
        && a.category != "vpn"
        && ((!a.state.is_empty() && a.state != "background")
            || (!a.category.is_empty() && a.rx_bps + a.tx_bps >= 50_000)
            || a.tx_bps >= 125_000)
}

/// The apps the ping page lists on its current tab, in order.
fn listed_apps(app: &App) -> Vec<ug_proto::AppTraffic> {
    let apps = app
        .net
        .value
        .as_ref()
        .map(|n| n.0.clone())
        .unwrap_or_default();
    if app.tab == 0 {
        apps.into_iter().filter(is_background).collect()
    } else {
        apps.into_iter()
            .filter(|a| a.category != "vpn" && (a.rx_bytes + a.tx_bytes > 0 || !a.state.is_empty()))
            .collect()
    }
}

fn app_line(app: &App, a: &ug_proto::AppTraffic, width: u16) -> Line<'static> {
    let t = app.theme;
    let name: String = a.name.chars().take(24).collect();
    let tag = match a.state.as_str() {
        "blocked" => Span::styled(" blocked ", Style::default().fg(t.danger)),
        "limited" => Span::styled(" limited ", t.accent()),
        _ if !a.category.is_empty() => Span::styled(format!(" {} ", a.category), t.faint()),
        _ => Span::raw(""),
    };
    let mut spans = vec![Span::styled(format!(" {name:<24}"), t.text()), tag];
    let rates = format!("↓ {:>11}  ↑ {:>11}", rate(a.rx_bps), rate(a.tx_bps));
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (width as usize).saturating_sub(used + rates.chars().count() + 12);
    spans.push(Span::raw(" ".repeat(pad.min(40))));
    spans.push(Span::styled(rates, t.muted()));
    spans.push(Span::styled(
        format!("  {:>7.1} MB", (a.rx_bytes + a.tx_bytes) as f64 / 1e6),
        t.faint(),
    ));
    if a.has_game {
        spans.push(Span::styled(" game inside", Style::default().fg(t.warn)));
    }
    Line::from(spans)
}

fn ping_page(app: &mut App, f: &mut Frame, r: Rect) {
    tabs(
        app,
        f,
        Rect { height: 1, ..r },
        &["Ping stability", "Internet control"],
    );
    let body = Rect {
        y: r.y + 2,
        height: r.height.saturating_sub(2),
        ..r
    };
    let t = app.theme;
    let mut y = body.y;
    // Problems first: they explain everything below.
    if let Some((_, _, st)) = &app.net.value {
        let mut notes: Vec<(String, Style)> = Vec::new();
        if st.simulated {
            notes.push((
                "Development service: the numbers are invented and nothing is changed.".into(),
                t.faint(),
            ));
        }
        if !st.available {
            notes.push((
                format!("Internet control is not available here. {}", st.problem),
                Style::default().fg(t.danger),
            ));
        }
        for p in &st.shaping_problems {
            notes.push((p.clone(), t.warn_style()));
        }
        for (n, s) in notes {
            f.render_widget(
                Paragraph::new(Span::styled(n, s)),
                Rect {
                    x: body.x,
                    y,
                    width: body.width,
                    height: 1,
                },
            );
            y += 1;
        }
    }
    if let Some(e) = &app.net.error {
        f.render_widget(
            Paragraph::new(Span::styled(e.clone(), Style::default().fg(t.danger))),
            Rect {
                x: body.x,
                y,
                width: body.width,
                height: 1,
            },
        );
        y += 1;
    }
    // "Slow down background apps": the switch for a steady ping.
    if let Some((on, up, down, when)) = app.net_bg.clone() {
        let cap = |k: u32| {
            if k == 0 {
                "no cap".to_string()
            } else if k >= 1000 {
                format!("{:.1} Mbit/s", k as f64 / 1000.0)
            } else {
                format!("{k} kbit/s")
            }
        };
        let slowed = app
            .net
            .value
            .as_ref()
            .map(|(a, _, _)| a.iter().filter(|x| x.state == "background").count())
            .unwrap_or(0);
        let line = Line::from(vec![
            Span::styled("Slow down background apps  ", t.title()),
            Span::styled(
                if on { "On" } else { "Off" },
                if on { t.accent() } else { t.muted() },
            ),
            Span::styled(
                format!(
                    "   up {}, down {} shared, {}{}   s turns it {}",
                    cap(up),
                    cap(down),
                    if when == "gaming" {
                        "while gaming"
                    } else {
                        "always"
                    },
                    if on {
                        format!("; {slowed} slowed now")
                    } else {
                        String::new()
                    },
                    if on { "off" } else { "on" }
                ),
                t.muted(),
            ),
        ]);
        f.render_widget(
            Paragraph::new(line),
            Rect {
                x: body.x,
                y,
                width: body.width,
                height: 1,
            },
        );
        y += 1;
        f.render_widget(
            Paragraph::new(Span::styled("  The running game, VPNs and proxies, DNS managers and apps with their own rule are left alone.", t.faint())),
            Rect { x: body.x, y, width: body.width, height: 1 },
        );
        y += 2;
    }
    let bottom = body.y + body.height;
    if app.tab == 0 {
        let rep = app.ping.value.clone().unwrap_or_default();
        if rep.targets.is_empty() {
            let msg = app.ping.error.clone().unwrap_or_else(|| {
                "Measuring… one echo a second while this page is open or a game runs.".into()
            });
            f.render_widget(
                Paragraph::new(Span::styled(msg, t.muted())),
                Rect {
                    x: body.x,
                    y,
                    width: body.width,
                    height: 1,
                },
            );
            y += 2;
        } else {
            let fmt = |v: f64| {
                if v.is_nan() {
                    "—".to_string()
                } else if v < 10.0 {
                    format!("{v:.1}")
                } else {
                    format!("{v:.0}")
                }
            };
            for tg in &rep.targets {
                let name = match tg.id.as_str() {
                    "gateway" => "Router".to_string(),
                    "internet" => "Internet (DNS)".to_string(),
                    _ => format!("Game: {}", tg.label),
                };
                let line = if tg.problem.is_empty() {
                    Line::from(vec![
                        Span::styled(format!("{name:<22}"), t.muted()),
                        Span::styled(format!("{:>6} ms", fmt(tg.stats.median_ms)), t.title()),
                        Span::styled(
                            format!(
                                "   jitter {:>5}   95% {:>5}   loss {:>3.0} %",
                                fmt(tg.stats.jitter_ms),
                                fmt(tg.stats.p95_ms),
                                tg.stats.loss_pct
                            ),
                            t.muted(),
                        ),
                        Span::styled(format!("   {}", tg.addr), t.faint()),
                        Span::styled(
                            if tg.via.is_empty() {
                                String::new()
                            } else if tg.through_vpn {
                                format!("  through the VPN ({})", tg.via)
                            } else {
                                format!("  direct ({})", tg.via)
                            },
                            if tg.through_vpn {
                                t.accent()
                            } else {
                                t.faint()
                            },
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled(format!("{name:<22}"), t.muted()),
                        Span::styled(tg.problem.clone(), t.faint()),
                    ])
                };
                f.render_widget(
                    Paragraph::new(line),
                    Rect {
                        x: body.x,
                        y,
                        width: body.width,
                        height: 1,
                    },
                );
                y += 1;
            }
            y += 1;
            // Five minutes, router against the far end.
            let near = rep.targets.iter().find(|x| x.id == "gateway");
            let far = rep
                .targets
                .iter()
                .find(|x| x.id == "game" && !x.addr.is_empty())
                .or_else(|| rep.targets.iter().find(|x| x.id == "internet"));
            let now = data::now_ms() as f64 / 1000.0;
            let pts = |tg: Option<&ug_proto::PingTarget>| -> Vec<(f64, f64)> {
                tg.map(|tg| {
                    tg.samples
                        .iter()
                        .filter(|s| !s.rtt_ms.is_nan())
                        .map(|s| (s.ts_unix_ms as f64 / 1000.0 - now, s.rtt_ms))
                        .collect()
                })
                .unwrap_or_default()
            };
            let (a, b) = (pts(near), pts(far));
            let h = 10.min(bottom.saturating_sub(y + 6));
            if h >= 5 {
                let far_name = far
                    .map(|x| {
                        if x.id == "game" {
                            x.label.clone()
                        } else {
                            "Internet".into()
                        }
                    })
                    .unwrap_or_default();
                live_chart(
                    app,
                    f,
                    Rect {
                        x: body.x,
                        y,
                        width: body.width,
                        height: h,
                    },
                    "Ping, last five minutes",
                    " ms",
                    300.0,
                    (0.0, 50.0),
                    ("Router", a),
                    Some((far_name.as_str(), b)),
                );
                y += h;
            }
            let verdict = match &app.ping_compare {
                Some((_, c)) => {
                    let text = match c.verdict.as_str() {
                        "flatter" => "Flatter since game mode went on: less jitter and fewer spikes.",
                        "worse" => "Worse since game mode went on; something else on the network may be busy.",
                        "same" => "No noticeable difference since game mode went on.",
                        _ => "Not enough readings on both sides of game mode yet.",
                    };
                    Line::from(vec![
                        Span::styled("Before/after game mode  ", t.muted()),
                        Span::styled(
                            format!("jitter {} → {} ms   ", fmt(c.before.jitter_ms), fmt(c.after.jitter_ms)),
                            t.title(),
                        ),
                        Span::styled(text, if c.verdict == "flatter" { Style::default().fg(t.ok) } else { t.muted() }),
                    ])
                }
                None => Line::from(Span::styled("When game mode turns on, ping before and after is compared from real readings.", t.faint())),
            };
            if y < bottom {
                f.render_widget(
                    Paragraph::new(verdict),
                    Rect {
                        x: body.x,
                        y,
                        width: body.width,
                        height: 1,
                    },
                );
                y += 2;
            }
        }
        if y < bottom {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "Background tasks using the network",
                    t.title(),
                )),
                Rect {
                    x: body.x,
                    y,
                    width: body.width,
                    height: 1,
                },
            );
            y += 1;
        }
    }
    let list = listed_apps(app);
    if app.tab == 1 && y < bottom {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    " {:<24}   {:>11}     {:>11}",
                    "App or service", "down", "up"
                ),
                t.faint(),
            )),
            Rect {
                x: body.x,
                y,
                width: body.width,
                height: 1,
            },
        );
        y += 1;
    }
    if list.is_empty() && y < bottom {
        let msg = if app.tab == 0 {
            "Nothing in the background is using the network much right now."
        } else {
            "No app has used the internet since the service started."
        };
        f.render_widget(
            Paragraph::new(Span::styled(msg, t.muted())),
            Rect {
                x: body.x,
                y,
                width: body.width,
                height: 1,
            },
        );
        y += 1;
    }
    app.rows = list.len();
    // Keep the selection on screen.
    let room = bottom.saturating_sub(y + if app.tab == 1 { 4 } else { 1 }) as usize;
    let first = app.row.saturating_sub(room.saturating_sub(1));
    for (i, a) in list.iter().enumerate().skip(first).take(room) {
        let line = app_line(app, a, body.width);
        row(
            app,
            f,
            Rect {
                x: body.x,
                y,
                width: body.width,
                height: 1,
            },
            i,
            line,
        );
        y += 1;
    }
    // The VPN carrying everything gets one line, not a place in the list.
    let vpn_tun = app.net.value.as_ref().is_some_and(|n| n.2.vpn_tun);
    let vpns: Vec<ug_proto::AppTraffic> = app
        .net
        .value
        .as_ref()
        .map(|n| {
            n.0.iter()
                .filter(|a| a.category == "vpn")
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for v in vpns {
        if y >= bottom {
            break;
        }
        let line = Line::from(vec![
            Span::styled(format!(" VPN {} ", v.name), t.accent()),
            Span::styled(
                format!("↓ {} ↑ {} of its own", rate(v.rx_bps), rate(v.tx_bps)),
                t.muted(),
            ),
            Span::styled(
                format!(
                    "  ({} mode: what it carries is counted for the apps, not again)",
                    if vpn_tun { "TUN" } else { "proxy" }
                ),
                t.faint(),
            ),
        ]);
        f.render_widget(
            Paragraph::new(line),
            Rect {
                x: body.x,
                y,
                width: body.width,
                height: 1,
            },
        );
        y += 1;
    }
    if app.tab == 1 && y + 2 < bottom {
        y += 1;
        let rules = app
            .net
            .value
            .as_ref()
            .map(|n| n.1.clone())
            .unwrap_or_default();
        let kb = |k: u32| {
            if k == 0 {
                "—".to_string()
            } else if k >= 1000 {
                format!("{:.1} Mbit/s", k as f64 / 1000.0)
            } else {
                format!("{k} kbit/s")
            }
        };
        let mut lines = vec![Line::from(Span::styled("Rules", t.title()))];
        if rules.is_empty() {
            lines.push(Line::from(Span::styled(
                "No rules. Enter limits the selected app while you play; b blocks it.",
                t.faint(),
            )));
        }
        for r in rules {
            let what = if r.block {
                "blocked".to_string()
            } else {
                format!("up {}  down {}", kb(r.up_kbps), kb(r.down_kbps))
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {:<24} ", r.app), t.text()),
                Span::styled(format!("{what:<30}"), t.muted()),
                Span::styled(
                    if r.when == "gaming" {
                        "while gaming  "
                    } else {
                        "always        "
                    },
                    t.faint(),
                ),
                Span::styled(
                    if r.active {
                        "in effect"
                    } else {
                        "waits for a game"
                    },
                    if r.active { t.accent() } else { t.faint() },
                ),
            ]));
        }
        f.render_widget(
            Paragraph::new(lines),
            Rect {
                x: body.x,
                y,
                width: body.width,
                height: bottom - y,
            },
        );
    }
}

/// The page's own keys, for the key bar.
pub fn keys(app: &App) -> Vec<(&'static str, &'static str)> {
    match (app.page, app.tab) {
        (1, 1) => vec![
            ("Tab", "launch options"),
            ("←→", "change"),
            ("Enter", "run"),
        ],
        (1, 2) => vec![
            ("Tab", "game mode"),
            ("↑↓", "game"),
            ("←→", "card"),
            ("o", "overlay"),
            ("Enter", "write"),
            ("u", "undo"),
        ],
        (1, _) => vec![
            ("Tab", "graphics card"),
            ("←→", "profile"),
            ("Enter", "toggle"),
            ("g", "game mode now"),
        ],
        (2, 0) => vec![("←→", "when"), ("Enter", "run")],
        (2, 1) => vec![("Tab", "next tab"), ("x", "cancel")],
        (2, 3) => vec![("↑↓", "row"), ("←→", "change"), ("Enter", "next")],
        (2, _) => vec![("Enter", "choose"), ("←→", "limit")],
        (3, 0) => vec![("Tab", "traffic"), ("←→", "range")],
        (3, 1) => vec![("Tab", "FPS"), ("←→", "period"), ("r r", "reset counts")],
        (3, 2) => vec![("Tab", "alerts"), ("Enter", "set up / undo")],
        (3, 3) => vec![
            ("Tab", "temperature"),
            ("↑↓", "row"),
            ("←→", "limit"),
            ("Enter", "on / off"),
        ],
        (3, _) => vec![("Tab", "temperature"), ("Enter", "set up / undo")],
        (4, _) => vec![
            ("Tab", "next tab"),
            ("s", "slow background"),
            ("Enter", "limit while gaming"),
            ("b", "block"),
            ("x", "allow"),
        ],
        (8, 1) => vec![("Tab", "check"), ("Enter ×2", "undo"), ("y", "keep")],
        (8, _) => vec![
            ("Tab", "changes"),
            ("Enter", "review / apply"),
            ("r", "check again"),
        ],
        (12, _) => vec![("↑↓", "setting"), ("←→", "change"), ("Enter", "toggle")],
        (10, _) => crate::lab::keys(app),
        (7, _) => vec![("Enter", "start / stop / save"), ("←→", "change")],
        (5, 1) if app.recording.is_some() => vec![("Tab", "+Meta"), ("Esc", "cancel")],
        (5, 1) => vec![
            ("Tab", "overlay"),
            ("Enter", "on/off / run"),
            ("c", "change keys"),
            ("x", "suggested keys"),
        ],
        (5, _) => vec![
            ("Tab", "shortcuts"),
            ("↑↓", "setting"),
            ("←→", "change"),
            ("Enter", "next / run"),
        ],
        (6, 2) => vec![("Tab", "games"), ("←→", "7 / 30 days")],
        (6, 0) => vec![
            ("Tab", "apps"),
            ("←→", "period"),
            ("x", "delete"),
            ("n", "not a game"),
            ("i", "import Steam"),
            ("e", "CSV"),
        ],
        (6, _) => vec![
            ("Tab", "days"),
            ("←→", "period"),
            ("Enter", "count / stop"),
            ("x", "delete"),
        ],
        (0, _) if gpu_switch(app).is_some() => vec![("g", "game mode"), ("←→", "card for ug run")],
        (0, _) => vec![("g", "game mode")],
        _ => vec![],
    }
}

/// g: game mode on now (on the Game mode page with the profile shown), or
/// off when it is on.
pub fn toggle_game_mode(app: &mut App) {
    let game = app.running.value.as_ref().is_some_and(|v| !v.is_empty());
    let hand = app.hand.value.clone().unwrap_or_default();
    let on = hand.0 == "on" || (hand.0 != "off" && (game || app.gamemode.value == Some(true)));
    let profile = if app.page == 1 {
        app.profiles
            .value
            .as_ref()
            .and_then(|p| p.get(app.choice))
            .map(|p| p.id.clone())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let r = if on {
        app.bus.set_game_mode(false, "").map(|_| {
            if game {
                "Game mode off until the game ends."
            } else {
                "Game mode off."
            }
            .to_string()
        })
    } else {
        app.bus.set_game_mode(true, &profile).map(|_| {
            let used = if profile.is_empty() {
                hand.1.clone()
            } else {
                profile.clone()
            };
            format!("Game mode on with the {used} profile.")
        })
    };
    report(app, r);
    app.hand.invalidate();
    app.gamemode.invalidate();
}

pub fn set_choice(app: &mut App, i: usize) {
    if app.page == 0 {
        choose_gpu(app, i);
        return;
    }
    if (app.page, app.tab) == (3, 1) {
        app.traffic_span = i.min(2);
        return;
    }
    app.choice = i;
    app.armed = None;
    if app.page == 6 {
        app.playtime.invalidate();
        app.row = 0;
    }
    if app.page == 3 {
        app.history.invalidate();
        app.live.clear();
        app.live_due.invalidate();
    }
    if app.page == 2 && app.tab == 2 {
        // A new limit applies at once when protection is on.
        if let Some(b) = app.battery.value.clone() {
            if b.mode != "off" {
                let limit = 50 + 5 * i as u8;
                let r = app.bus.set_battery(&b.mode, limit);
                report(app, r.map(|_| format!("Charging stops at {limit} %.")));
                app.battery.invalidate();
            }
        }
    }
}

pub fn step_choice(app: &mut App, d: i32) {
    if app.page == 0 {
        if let Some((_, chosen)) = gpu_switch(app) {
            choose_gpu(app, (chosen as i32 + d).clamp(0, 1) as usize);
        }
        return;
    }
    if (app.page, app.tab) == (1, 1) {
        step_tune(app, d);
        return;
    }
    if app.page == 7 {
        step_recorder(app, app.row, d);
        return;
    }
    if (app.page, app.tab) == (1, 2) {
        step_launch_gpu(app, d);
        return;
    }
    if (app.page, app.tab) == (5, 0) {
        step_overlay(app, d);
        return;
    }
    if app.page == 12 {
        step_settings(app, d);
        return;
    }
    if (app.page, app.tab) == (2, 3) {
        step_cooling(app, d);
        return;
    }
    if (app.page, app.tab) == (2, 4) {
        step_sleep(app, d);
        return;
    }
    if (app.page, app.tab) == (3, 3) {
        step_alert(app, d);
        return;
    }
    let max = match (app.page, app.tab) {
        (1, _) => app.profiles.value.as_ref().map(|p| p.len()).unwrap_or(1),
        (2, 0) => WHEN.len(),
        (2, 2) => 11,
        (3, 0) => RANGES.len(),
        (3, 1) => 3,
        (6, 2) => 2,
        (6, _) => 3,
        _ => return,
    };
    let current = if (app.page, app.tab) == (3, 1) {
        app.traffic_span
    } else {
        app.choice
    };
    let next = (current as i32 + d).clamp(0, max as i32 - 1) as usize;
    set_choice(app, next);
}

fn report(app: &mut App, r: Result<String, String>) {
    match r {
        Ok(m) => app.say(m, true),
        Err(e) => app.say(e, false),
    }
}

pub fn activate(app: &mut App, i: usize) {
    match (app.page, app.tab) {
        (6, _) => activate_playtime(app, i),
        (1, 1) => activate_tune(app, i),
        (5, 1) => activate_shortcut(app, i),
        (7, _) => activate_recorder(app, i),
        (8, 0) => activate_driver(app, i),
        (8, _) => activate_driver_op(app, i),
        (10, _) => crate::lab::activate(app, i),
        (11, _) => activate_update(app, i),
        (12, _) => {
            let _ = i;
            step_settings(app, 1);
        }
        (1, 2) => {
            let _ = i;
            write_launch(app);
        }
        (2, 3) => {
            let _ = i;
            step_cooling(app, 1);
        }
        (2, 4) => {
            let _ = i;
            step_sleep(app, 1);
        }
        (3, 3) => activate_alert(app, i),
        (5, _) => {
            let Some(st) = app.overlay.value.clone() else {
                return;
            };
            if !st.installed {
                return;
            }
            if !st.set_up {
                let r = app.bus.overlay_set_up(true);
                report(app, r.map(|_| "Overlay set up.".to_string()));
            } else if i == OVERLAY_ROWS.len() - 1 {
                let r = app.bus.overlay_set_up(false);
                report(
                    app,
                    r.map(|_| "MangoHud's configuration is back as it was.".to_string()),
                );
            } else {
                step_overlay(app, 1);
                return;
            }
            app.overlay.invalidate();
        }
        (4, _) => {
            let Some(a) = listed_apps(app).get(i).cloned() else {
                return;
            };
            if !a.state.is_empty() && a.state != "background" {
                app.say(
                    format!("{} already has a rule; x removes it.", a.name),
                    false,
                );
                return;
            }
            let r = app.bus.net_set_rule(&a.name, false, 1000, 8000, "gaming");
            report(
                app,
                r.map(|_| {
                    format!(
                        "{} is capped while you play: 1 Mbit/s up, 8 Mbit/s down.",
                        a.name
                    )
                }),
            );
            app.net.invalidate();
        }
        (3, 2) => {
            let on = app
                .fps
                .value
                .as_ref()
                .is_some_and(|f| f.state == "not-set-up");
            if !matches!(
                app.fps.value.as_ref().map(|f| f.state.as_str()),
                Some("not-set-up" | "idle" | "live")
            ) {
                return;
            }
            let r = app.bus.set_up_fps(on);
            report(
                app,
                r.map(|_| {
                    if on {
                        "FPS reading set up in MangoHud.conf.".into()
                    } else {
                        "MangoHud.conf is back as it was.".into()
                    }
                }),
            );
            app.fps.invalidate();
            let _ = i;
        }
        (1, _) => {
            let Some(profiles) = app.profiles.value.clone() else {
                return;
            };
            let p = &profiles[app.choice.min(profiles.len() - 1)];
            if i < OPTIONS.len() {
                let (key, label, _) = OPTIONS[i];
                let now = [
                    p.cpu_performance,
                    p.cpu_no_idle_states,
                    p.cpu_preferred_cores,
                    p.cpu_priority,
                    p.memory_tuning,
                    p.pause_services,
                    p.network_tuning,
                    p.power_profile,
                ][i];
                let r = app.bus.set_option(&p.id, key, !now);
                report(
                    app,
                    r.map(|_| format!("{}: {label} {}.", p.id, if now { "off" } else { "on" })),
                );
            } else if let Some(g) = app
                .installed
                .value
                .clone()
                .and_then(|g| g.get(i - OPTIONS.len()).cloned())
            {
                // Cycle: suggested → each profile → suggested.
                let current = profiles.iter().position(|p| p.games.contains(&g.id));
                let next = match current {
                    None => profiles.first().map(|p| p.id.clone()).unwrap_or_default(),
                    Some(k) if k + 1 < profiles.len() => profiles[k + 1].id.clone(),
                    Some(_) => String::new(),
                };
                let r = app.bus.set_game_profile(&g.id, &next);
                let shown = if next.is_empty() {
                    "the suggested profile".to_string()
                } else {
                    format!("the {next} profile")
                };
                report(app, r.map(|_| format!("{} now runs with {shown}.", g.name)));
            }
            app.profiles.invalidate();
        }
        (2, 0) => {
            let Some((id, label)) = ACTIONS.get(i).copied() else {
                return;
            };
            if let Some(c) = app
                .caps
                .value
                .as_ref()
                .and_then(|c| c.iter().find(|c| c.action == id))
            {
                if !c.available {
                    let why = c.reason.clone();
                    app.say(why, false);
                    return;
                }
            }
            let (when, minutes) = WHEN[app.choice.min(WHEN.len() - 1)];
            if minutes > 0 {
                let r = app
                    .bus
                    .power_schedule(id, data::now_ms() + minutes * 60_000, "once");
                report(
                    app,
                    r.map(|j| {
                        format!(
                            "{label} scheduled {when} (#{}). Cancel it under Queue.",
                            j.id
                        )
                    }),
                );
                app.jobs.invalidate();
                return;
            }
            let confirm = label.to_lowercase();
            let anyway = format!("{confirm} anyway");
            let no_lock = format!("{confirm} without locking the screen");
            let sleep = !matches!(id, "power-off" | "reboot" | "reboot-firmware");
            let armed_for =
                |app: &App, key: &str| app.armed.as_ref().is_some_and(|(a, _)| a.starts_with(key));
            if armed_for(app, &no_lock) {
                app.armed = None;
                let r = app.bus.power_now_anyway(id);
                report(
                    app,
                    r.map(|_| format!("{label}: requested, without the screen lock.")),
                );
            } else if armed_for(app, &anyway) {
                app.armed = None;
                if sleep {
                    if let ug_core::sleepguard::Locked::No(why) =
                        ug_core::sleepguard::lock_screen_now()
                    {
                        // Never asleep unlocked without asking.
                        app.armed = Some((
                            format!("{no_lock} ({why} Anyone could use it when it wakes)"),
                            std::time::Instant::now(),
                        ));
                        return;
                    }
                }
                let r = app.bus.power_now_anyway(id);
                report(
                    app,
                    r.map(|_| format!("{label}: requested, past the programs keeping it awake.")),
                );
            } else if app.armed.as_ref().is_some_and(|(a, _)| *a == confirm) {
                app.armed = None;
                match app.bus.power_now_or_blockers(id) {
                    Ok(None) => app.say(format!("{label}: requested."), true),
                    Ok(Some(list)) => {
                        // Shown in the confirmation line: who, then how to go ahead.
                        app.armed = Some((
                            format!(
                                "{anyway} (kept awake by: {}{}; asks for the password)",
                                list.join("; "),
                                if sleep {
                                    "; the screen is locked first"
                                } else {
                                    ""
                                }
                            ),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(e) => app.say(e, false),
                }
            } else {
                app.armed = Some((confirm, std::time::Instant::now()));
            }
        }
        (2, 1) => remove(app, i),
        (2, 2) => {
            let mode = ["off", "protect", "full-once"][i.min(2)];
            let limit = 50 + 5 * app.choice.min(10) as u8;
            let limit = if app.battery.value.as_ref().is_some_and(|b| b.adjustable) {
                limit
            } else {
                80
            };
            let r = app.bus.set_battery(mode, limit);
            report(app, r.map(|_| "Battery protection updated.".to_string()));
            app.battery.invalidate();
        }
        _ => {}
    }
}

/// `b` on the ping page: block the selected app.
pub fn block(app: &mut App, i: usize) {
    if app.page != 4 {
        return;
    }
    let Some(a) = listed_apps(app).get(i).cloned() else {
        return;
    };
    let r = app.bus.net_set_rule(&a.name, true, 0, 0, "always");
    report(
        app,
        r.map(|_| format!("{} is blocked. x allows it again.", a.name)),
    );
    app.net.invalidate();
}

pub fn remove(app: &mut App, i: usize) {
    if (app.page, app.tab) == (5, 1) {
        let Some(k) = app
            .shortcuts
            .value
            .as_ref()
            .and_then(|s| s.shortcuts.get(i.wrapping_sub(1)))
            .cloned()
        else {
            return;
        };
        if k.chosen {
            let r = app.bus.shortcut_set_keys(&k.id, "");
            report(app, r);
            app.shortcuts.invalidate();
        }
        return;
    }
    if app.page == 6 {
        forget_game(app, i);
        return;
    }
    if app.page == 4 {
        let Some(a) = listed_apps(app).get(i).cloned() else {
            return;
        };
        let r = app.bus.net_remove_rule(&a.name);
        report(
            app,
            r.map(|had| {
                if had {
                    format!("{} has no limits any more.", a.name)
                } else {
                    format!("{} had no rule.", a.name)
                }
            }),
        );
        app.net.invalidate();
        return;
    }
    if (app.page, app.tab) != (2, 1) {
        return;
    }
    let (jobs, watches) = app.jobs.value.clone().unwrap_or_default();
    let r = if let Some(j) = jobs.get(i) {
        app.bus.power_cancel(j.id).map(|_| {
            format!(
                "Cancelled: {} at {}.",
                j.action,
                data::local_time(j.at_unix_ms)
            )
        })
    } else if let Some(w) = watches.get(i - jobs.len()) {
        app.bus
            .power_unwatch(w.id)
            .map(|_| format!("Removed: {} ({}).", w.action, w.kind))
    } else {
        return;
    };
    report(app, r);
    app.jobs.invalidate();
    app.row = app.row.saturating_sub(1);
}
