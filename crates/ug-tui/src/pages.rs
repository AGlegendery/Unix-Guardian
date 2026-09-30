//! The pages: drawing, and what their keys and clicks do.

use crate::{data, App, Hit, PAGES};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, BorderType, Borders, Chart, Dataset, GraphType, Paragraph, Wrap};
use ratatui::Frame;

const OPTIONS: [(&str, &str, &str); 7] = [
    ("cpu_performance", "CPU at full speed", "Governor and energy preference set to performance."),
    ("cpu_no_idle_states", "No clock drops between frames", "Keeps the CPU out of deep sleep states between frames."),
    ("cpu_preferred_cores", "Game on the fastest cores", "Performance cores (Intel) or V-Cache cores (AMD X3D)."),
    ("cpu_priority", "Game gets priority", "Higher scheduling priority than other apps."),
    ("memory_tuning", "Memory tuned for games", "Less swapping, larger memory-map limit, no background compaction."),
    ("pause_services", "Background updates held", "Package updates and indexing wait until you stop playing."),
    ("network_tuning", "Network tuned for low ping", "Wi-Fi power saving off, TCP Fast Open, smaller send queue, BBR."),
];
const WHEN: [(&str, u64); 6] = [("Now", 0), ("in 15 min", 15), ("in 30 min", 30), ("in 1 h", 60), ("in 2 h", 120), ("in 4 h", 240)];
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

fn opt(v: f64, unit: &str, digits: usize) -> String {
    if v.is_nan() { "—".into() } else { format!("{v:.digits$}{unit}") }
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
    if c.is_nan() { ("", 0) } else if c >= 85.0 { ("hot", 2) } else if c >= 70.0 { ("warm", 1) } else { ("cool", 0) }
}

/// Draws the current page into `r`.
pub fn draw(app: &mut App, f: &mut Frame, r: Rect, wide: bool) {
    let inner = Rect { x: r.x + 1, y: r.y, width: r.width.saturating_sub(2), height: r.height };
    app.rows = 0;
    match app.page {
        0 => dashboard(app, f, inner, wide),
        1 => game_mode(app, f, inner),
        2 => system(app, f, inner),
        3 => monitoring(app, f, inner),
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

fn tabs(app: &mut App, f: &mut Frame, r: Rect, names: &[&str]) {
    let mut x = r.x;
    for (i, n) in names.iter().enumerate() {
        let w = n.chars().count() as u16 + 2;
        let rect = Rect { x, y: r.y, width: w.min(r.right().saturating_sub(x)), height: 1 };
        let style = if i == app.tab { app.theme.accent().add_modifier(Modifier::UNDERLINED) } else { app.theme.muted() };
        f.render_widget(Paragraph::new(Span::styled(format!(" {n} "), style)), rect);
        app.hits.push((rect, Hit::Tab(i)));
        x += w + 1;
    }
}

/// A row of choices; the chosen one highlighted, each clickable.
fn choices(app: &mut App, f: &mut Frame, r: Rect, label: &str, names: &[String], chosen: usize) {
    let mut x = r.x;
    f.render_widget(Paragraph::new(Span::styled(label, app.theme.muted())), Rect { x, y: r.y, width: label.len() as u16, height: 1 });
    x += label.len() as u16 + 1;
    for (i, n) in names.iter().enumerate() {
        let w = n.chars().count() as u16 + 2;
        if x + w > r.right() {
            break;
        }
        let rect = Rect { x, y: r.y, width: w, height: 1 };
        let style = if i == chosen { app.theme.selected() } else { app.theme.muted() };
        f.render_widget(Paragraph::new(Span::styled(format!(" {n} "), style)), rect);
        app.hits.push((rect, Hit::Choice(i)));
        x += w + 1;
    }
}

/// A selectable list row; returns the rect so callers can add detail lines.
fn row(app: &mut App, f: &mut Frame, r: Rect, i: usize, line: Line) {
    let selected = app.focus == crate::Focus::Page && app.row == i;
    let style = if selected { app.theme.selected() } else { Style::default() };
    f.render_widget(Paragraph::new(line).style(style), r);
    app.hits.push((r, Hit::Row(i)));
}

fn dashboard(app: &mut App, f: &mut Frame, r: Rect, wide: bool) {
    let t = app.theme;
    let Some(s) = app.snapshot.value.clone() else {
        let msg = app.snapshot.error.clone().unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    };
    let inv = app.inventory.value.clone();
    let gpu_awake = s.gpus.iter().find(|g| !g.asleep && !g.temp_c.is_nan());
    let dgpu = inv.as_ref().and_then(|i| i.gpus.iter().find(|g| !g.boot_vga));
    let igpu = inv.as_ref().and_then(|i| i.gpus.iter().find(|g| g.boot_vga));
    let (heat_word, level) = heat(s.cpu.temp_c);
    let heat_color = [t.ok, t.warn, t.danger][level];
    let mem_used = s.mem.total_kb.saturating_sub(s.mem.available_kb) as f64;
    let mem_frac = if s.mem.total_kb > 0 { mem_used / s.mem.total_kb as f64 } else { 0.0 };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if wide {
            vec![Constraint::Length(6), Constraint::Min(8), Constraint::Length(3)]
        } else {
            vec![Constraint::Length(5), Constraint::Min(6), Constraint::Length(3)]
        })
        .split(r);

    // Tiles: the GPU in use, the processor, memory, network.
    let gpu_lines: Vec<Line> = match (gpu_awake, dgpu) {
        (Some(g), Some(d)) => vec![
            Line::from(vec![Span::styled(opt(g.temp_c, "°", 0), t.title()), Span::styled(format!("  {}", opt(g.load_pct, " %", 0)), t.muted())]),
            Line::from(Span::styled(d.name.clone(), t.faint())),
        ],
        _ => {
            let clock = igpu.and_then(|i| s.gpus.iter().find(|g| g.pci == i.pci)).map(|g| g.core_clock_mhz).unwrap_or(0);
            vec![
                Line::from(vec![Span::styled(format!("{clock} MHz"), t.title()), Span::styled("  in use", Style::default().fg(t.accent))]),
                Line::from(Span::styled(igpu.map(|g| g.name.clone()).unwrap_or_default(), t.faint())),
                Line::from(Span::styled(dgpu.map(|d| format!("{}: asleep", short_name(&d.name))).unwrap_or_default(), t.faint())),
            ]
        }
    };
    let cpu_lines = vec![
        Line::from(vec![
            Span::styled(opt(s.cpu.temp_c, "°", 0), Style::default().fg(heat_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {heat_word}   {}", opt(s.cpu.load_pct, " %", 0)), t.muted()),
            Span::styled(format!("   {}", opt(s.cpu.power_w, " W", 0)), t.muted()),
        ]),
        Line::from(Span::styled(bar(16, s.cpu.load_pct.max(0.0) / 100.0), Style::default().fg(t.accent))),
        Line::from(Span::styled(inv.as_ref().map(|i| short_name(&i.cpu.model)).unwrap_or_default(), t.faint())),
    ];
    let mem_lines = vec![
        Line::from(vec![
            Span::styled(format!("{:.1}", mem_used / 1_048_576.0), t.title()),
            Span::styled(format!(" / {:.1} GB  {:.0} %", s.mem.total_kb as f64 / 1_048_576.0, mem_frac * 100.0), t.muted()),
        ]),
        Line::from(Span::styled(bar(16, mem_frac), Style::default().fg(t.accent))),
    ];
    let net_lines = vec![
        Line::from(vec![Span::styled("↓ ", t.muted()), Span::styled(rate(s.net.rx_bps), t.title())]),
        Line::from(vec![Span::styled("↑ ", t.muted()), Span::styled(rate(s.net.tx_bps), t.text())]),
        Line::from(Span::styled(s.net.iface.clone(), t.faint())),
    ];
    let tiles = [("Graphics card", gpu_lines), ("Processor", cpu_lines), ("Memory", mem_lines), ("Network", net_lines)];
    if wide {
        let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Ratio(1, 4); 4]).split(rows[0]);
        for (i, (title, lines)) in tiles.into_iter().enumerate() {
            f.render_widget(Paragraph::new(lines).block(card(app, title)), cols[i]);
        }
    } else {
        let mut lines = Vec::new();
        for (title, l) in tiles {
            let mut first = vec![Span::styled(format!("{title:<14}"), t.muted())];
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
    let on = game.is_some() || app.gamemode.value == Some(true);
    let profile_id = game.as_ref().map(|g| g.profile.clone()).unwrap_or_else(|| "balanced".into());
    let profile = app.profiles.value.as_ref().and_then(|p| p.iter().find(|p| p.id == profile_id).cloned());
    let mut gm = vec![Line::from(vec![
        Span::styled(if on { "On " } else { "Off " }, if on { t.accent() } else { t.muted() }),
        Span::styled(
            match &game {
                Some(g) => format!("— {} ({} profile)", g.name, g.profile),
                None => format!("— starts with a game ({profile_id} profile)"),
            },
            t.muted(),
        ),
    ])];
    if let Some(p) = profile {
        let flags = [p.cpu_performance, p.cpu_no_idle_states, p.cpu_preferred_cores, p.cpu_priority, p.memory_tuning, p.pause_services, p.network_tuning];
        for ((_, label, _), on) in OPTIONS.iter().zip(flags) {
            gm.push(Line::from(vec![
                Span::styled(if on { "■ " } else { "□ " }, Style::default().fg(if on { t.accent } else { t.faint })),
                Span::styled(*label, if on { t.text() } else { t.faint() }),
            ]));
        }
    }
    f.render_widget(Paragraph::new(gm).wrap(Wrap { trim: true }).block(card(app, "Game mode")), mid[0]);
    temp_chart(app, f, mid[1]);

    // Per-core load, P and E cores apart.
    let p_cores: Vec<u32> = inv.as_ref().map(|i| i.cpu.p_cores.clone()).unwrap_or_default();
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let glyph = |l: f64| blocks[((l.clamp(0.0, 100.0) / 100.0) * 7.0).round() as usize];
    let mut p = String::new();
    let mut e = String::new();
    for (i, l) in s.cpu.per_core_load.iter().enumerate() {
        let g = glyph(if l.is_nan() { 0.0 } else { *l });
        if p_cores.is_empty() || p_cores.contains(&(i as u32)) { p.push(g) } else { e.push(g) }
    }
    let mut spans = vec![Span::styled(if e.is_empty() { "Cores " } else { "Performance " }, t.muted()), Span::styled(p, Style::default().fg(t.accent))];
    if !e.is_empty() {
        spans.push(Span::styled("   Efficiency ", t.muted()));
        spans.push(Span::styled(e, Style::default().fg(t.accent)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).block(card(app, "Load per core")), rows[2]);
}

fn short_name(s: &str) -> String {
    s.replace("12th Gen Intel(R) Core(TM) ", "").replace("NVIDIA GeForce ", "").replace(" Laptop GPU", "")
}

fn temp_chart(app: &App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let Some(&(t0, _, _)) = app.temps.front() else {
        f.render_widget(Paragraph::new("Collecting readings…").block(card(app, "Temperature, last 2 minutes")), r);
        return;
    };
    let cpu: Vec<(f64, f64)> = app.temps.iter().filter(|p| !p.1.is_nan()).map(|p| (p.0 - t0, p.1)).collect();
    let gpu: Vec<(f64, f64)> = app.temps.iter().filter(|p| !p.2.is_nan()).map(|p| (p.0 - t0, p.2)).collect();
    let span = app.temps.back().map(|p| p.0 - t0).unwrap_or(1.0).max(60.0);
    let lo = cpu.iter().chain(&gpu).map(|p| p.1).fold(30.0_f64, f64::min).floor();
    let hi = cpu.iter().chain(&gpu).map(|p| p.1).fold(70.0_f64, f64::max).ceil();
    let mut sets = vec![Dataset::default().name("Processor").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(t.cpu)).data(&cpu)];
    if !gpu.is_empty() {
        sets.push(Dataset::default().name("Graphics card").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(t.gpu)).data(&gpu));
    }
    let two = sets.len() > 1;
    let chart = Chart::new(sets)
        .legend_position(two.then_some(ratatui::widgets::LegendPosition::TopRight))
        .block(card(app, "Temperature, last 2 minutes"))
        .x_axis(Axis::default().bounds([0.0, span]))
        .y_axis(
            Axis::default()
                .style(t.faint())
                .bounds([lo, hi])
                .labels([Span::raw(format!("{lo:.0}°")), Span::raw(format!("{:.0}°", (lo + hi) / 2.0)), Span::raw(format!("{hi:.0}°"))]),
        );
    f.render_widget(chart, r);
}

fn game_mode(app: &mut App, f: &mut Frame, r: Rect) {
    let t = app.theme;
    let profiles = app.profiles.value.clone().unwrap_or_default();
    if profiles.is_empty() {
        let msg = app.profiles.error.clone().unwrap_or_else(|| "Reading…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, t.muted())), r);
        return;
    }
    app.choice = app.choice.min(profiles.len() - 1);
    let game = app.running.value.as_ref().and_then(|v| v.first().cloned());
    let on = game.is_some() || app.gamemode.value == Some(true);
    let head = Line::from(vec![
        Span::styled("Game mode  ", t.title()),
        Span::styled(if on { "On" } else { "Off" }, if on { t.accent() } else { t.muted() }),
        Span::styled(
            match &game {
                Some(g) => format!("   {} is running ({} profile)", g.name, g.profile),
                None => "   Starts by itself when a game starts".into(),
            },
            t.muted(),
        ),
    ]);
    f.render_widget(Paragraph::new(head), Rect { height: 1, ..r });
    let names: Vec<String> = profiles.iter().map(|p| if p.builtin { capital(&p.id) } else { format!("{} (yours)", p.id) }).collect();
    let chosen = app.choice;
    choices(app, f, Rect { y: r.y + 2, height: 1, ..r }, "Profile", &names, chosen);
    let t = app.theme;
    let p = profiles[chosen].clone();
    let flags = [p.cpu_performance, p.cpu_no_idle_states, p.cpu_preferred_cores, p.cpu_priority, p.memory_tuning, p.pause_services, p.network_tuning];
    let has_preferred = app.inventory.value.as_ref().is_some_and(|i| i.cpu.has_vcache || !i.cpu.p_cores.is_empty());
    let mut y = r.y + 4;
    for (i, ((key, label, explain), on)) in OPTIONS.iter().zip(flags).enumerate() {
        if y + 1 >= r.bottom() {
            break;
        }
        let blocked = *key == "cpu_preferred_cores" && !has_preferred;
        let mark = if blocked { "[–]" } else if on { "[■]" } else { "[ ]" };
        let line = Line::from(vec![
            Span::styled(format!("{mark} "), Style::default().fg(if on { t.accent } else { t.faint })),
            Span::styled(*label, t.text()),
        ]);
        row(app, f, Rect { x: r.x, y, width: r.width, height: 1 }, i, line);
        let t = app.theme;
        let why = if blocked { "This CPU has no faster core group to prefer." } else { explain };
        f.render_widget(Paragraph::new(Span::styled(format!("    {why}"), t.faint())), Rect { x: r.x, y: y + 1, width: r.width, height: 1 });
        y += 2;
    }
    let t = app.theme;
    y += 1;
    let games = app.installed.value.clone().unwrap_or_default();
    if y < r.bottom() {
        f.render_widget(Paragraph::new(Span::styled("Your games (Enter changes the profile)", t.title())), Rect { x: r.x, y, width: r.width, height: 1 });
        y += 1;
    }
    for (gi, g) in games.iter().enumerate() {
        if y >= r.bottom() {
            break;
        }
        let chosen = profiles.iter().find(|p| p.games.contains(&g.id)).map(|p| p.id.clone()).unwrap_or_else(|| "suggested".into());
        let line = Line::from(vec![Span::styled(format!("{:<40}", g.name), app.theme.text()), Span::styled(format!(" {chosen}"), app.theme.accent())]);
        row(app, f, Rect { x: r.x, y, width: r.width, height: 1 }, OPTIONS.len() + gi, line);
        y += 1;
    }
    app.rows = OPTIONS.len() + games.len();
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn system(app: &mut App, f: &mut Frame, r: Rect) {
    tabs(app, f, Rect { height: 1, ..r }, &["Power command", "Queue", "Battery"]);
    let body = Rect { y: r.y + 2, height: r.height.saturating_sub(2), ..r };
    match app.tab {
        0 => power(app, f, body),
        1 => queue(app, f, body),
        _ => battery(app, f, body),
    }
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
        let style = if !available { t.faint() } else if danger { Style::default().fg(t.danger).add_modifier(Modifier::BOLD) } else { t.title() };
        row(app, f, Rect { x: r.x, y, width: r.width, height: 1 }, i, Line::from(Span::styled(format!("  {label}"), style)));
        if let Some(c) = cap.filter(|c| !c.available) {
            f.render_widget(
                Paragraph::new(Span::styled(c.reason.clone(), app.theme.faint())).wrap(Wrap { trim: true }),
                Rect { x: r.x + 4, y: y + 1, width: r.width.saturating_sub(4), height: 2 },
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
        f.render_widget(Paragraph::new(Span::styled("Nothing is scheduled or waiting.", app.theme.muted())), r);
        return;
    }
    let mut y = r.y;
    for (i, j) in jobs.iter().enumerate() {
        let line = Line::from(vec![
            Span::styled(format!("  {:<24}", j.action), app.theme.title()),
            Span::styled(format!("{}  {}", data::local_time(j.at_unix_ms), j.repeat), app.theme.muted()),
        ]);
        row(app, f, Rect { x: r.x, y, width: r.width, height: 1 }, i, line);
        y += 1;
    }
    for (k, w) in watches.iter().enumerate() {
        let what = match w.kind.as_str() {
            "net-idle" => format!("when downloads stay below {:.0} kB/s for {} min", w.threshold / 1000.0, w.minutes),
            "cpu-idle" => format!("when CPU load stays below {:.0} % for {} min", w.threshold, w.minutes),
            "process-gone" => format!("when {} exits", w.name),
            _ => "when the running game ends".into(),
        };
        let line = Line::from(vec![Span::styled(format!("  {:<24}", w.action), app.theme.title()), Span::styled(what, app.theme.muted())]);
        row(app, f, Rect { x: r.x, y, width: r.width, height: 1 }, jobs.len() + k, line);
        y += 1;
    }
    app.rows = jobs.len() + watches.len();
}

fn battery(app: &mut App, f: &mut Frame, r: Rect) {
    let Some(b) = app.battery.value.clone() else {
        let msg = app.battery.error.clone().unwrap_or_else(|| "Reading…".into());
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
        lines.push(Line::from(vec![Span::styled("Charge now  ", t.muted()), Span::styled(format!("{} %  ", b.capacity), t.title()), Span::styled(b.status.clone(), t.muted())]));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), Rect { height: 5, ..r });
    if !b.supported {
        f.render_widget(Paragraph::new(Span::styled(b.reason.clone(), app.theme.warn_style())).wrap(Wrap { trim: true }), Rect { y: r.y + 6, height: 3, ..r });
        return;
    }
    let modes = [("off", "Off (charge to 100 %)"), ("protect", "Protect"), ("full-once", "Full charge once, then protect")];
    let mut y = r.y + 6;
    for (i, (id, label)) in modes.iter().enumerate() {
        let on = b.mode == *id;
        let line = Line::from(vec![
            Span::styled(if on { "(•) " } else { "( ) " }, Style::default().fg(if on { app.theme.accent } else { app.theme.faint })),
            Span::styled(*label, app.theme.text()),
        ]);
        row(app, f, Rect { x: r.x, y, width: r.width, height: 1 }, i, line);
        y += 1;
    }
    app.rows = modes.len();
    y += 1;
    if b.adjustable {
        let limits: Vec<String> = (50..=100).step_by(5).map(|l| format!("{l}%")).collect();
        let chosen = app.choice.min(limits.len() - 1);
        choices(app, f, Rect { x: r.x, y, width: r.width, height: 1 }, "Stop at", &limits, chosen);
    } else {
        f.render_widget(
            Paragraph::new(Span::styled("This laptop uses the manufacturer's conservation mode; the firmware sets the limit (usually 60–80 %).", app.theme.faint()))
                .wrap(Wrap { trim: true }),
            Rect { x: r.x, y, width: r.width, height: 2 },
        );
    }
}

fn monitoring(app: &mut App, f: &mut Frame, r: Rect) {
    tabs(app, f, Rect { height: 1, ..r }, &["Temperature and power", "Internet traffic", "FPS"]);
    let body = Rect { y: r.y + 2, height: r.height.saturating_sub(2), ..r };
    if app.tab == 2 {
        fps(app, f, body);
        return;
    }
    if app.tab == 0 {
        let names: Vec<String> = RANGES.iter().map(|s| s.to_string()).collect();
        let chosen = app.choice.min(3);
        choices(app, f, Rect { height: 1, ..body }, "Range", &names, chosen);
        if chosen == 0 {
            live(app, f, Rect { y: body.y + 2, height: body.height.saturating_sub(2), ..body });
            return;
        }
        let pts = app.history.value.clone().unwrap_or_default();
        let area = Rect { y: body.y + 2, height: body.height.saturating_sub(2), ..body };
        if pts.len() < 2 {
            let msg = match &app.history.error {
                Some(e) => e.clone(),
                None => "Not enough history for this range yet. The agent records a reading every minute; charts fill in as it runs.".into(),
            };
            f.render_widget(
                Paragraph::new(Span::styled(msg, app.theme.muted()))
                    .wrap(Wrap { trim: true }),
                area,
            );
            return;
        }
        let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Ratio(1, 3); 3]).split(area);
        let t0 = pts[0].ts_unix_ms as f64 / 1000.0;
        let series = |get: &dyn Fn(&ug_proto::HistoryPoint) -> f64| -> Vec<(f64, f64)> {
            pts.iter().filter(|p| !get(p).is_nan()).map(|p| (p.ts_unix_ms as f64 / 1000.0 - t0, get(p))).collect()
        };
        let span = (pts.last().map(|p| p.ts_unix_ms as f64 / 1000.0 - t0).unwrap_or(60.0)).max(60.0);
        let charts: [(&str, &str, (f64, f64), Vec<(f64, f64)>, Vec<(f64, f64)>); 3] = [
            ("Temperature", "°", (30.0, 70.0), series(&|p| p.cpu_temp_c), series(&|p| p.gpu_temp_c)),
            ("Load", " %", (0.0, 100.0), series(&|p| p.cpu_load_pct), series(&|p| p.gpu_load_pct)),
            ("Power", " W", (0.0, 30.0), series(&|p| p.cpu_w), series(&|p| p.gpu_w)),
        ];
        for (i, (title, unit, (min_lo, min_hi), cpu, gpu)) in charts.iter().enumerate() {
            let lo = cpu.iter().chain(gpu).map(|p| p.1).fold(*min_lo, f64::min).floor();
            let hi = cpu.iter().chain(gpu).map(|p| p.1).fold(*min_hi, f64::max).ceil();
            let mut sets = vec![Dataset::default().name("Processor").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(app.theme.cpu)).data(cpu)];
            if !gpu.is_empty() {
                sets.push(Dataset::default().name("Graphics card").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(app.theme.gpu)).data(gpu));
            }
            let two = sets.len() > 1;
            let chart = Chart::new(sets)
                .legend_position(two.then_some(ratatui::widgets::LegendPosition::TopRight))
                .block(card(app, title))
                .x_axis(Axis::default().bounds([0.0, span]).labels([
                    Span::raw(data::local_time(pts[0].ts_unix_ms)),
                    Span::raw(data::local_time(pts.last().map(|p| p.ts_unix_ms).unwrap_or(0))),
                ]))
                .y_axis(Axis::default().style(app.theme.faint()).bounds([lo, hi]).labels([Span::raw(format!("{lo:.0}{unit}")), Span::raw(format!("{hi:.0}{unit}"))]));
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
        let days = app.traffic.value.clone().unwrap_or_default();
        let max = days.iter().map(|d| d.rx_bytes + d.tx_bytes).max().unwrap_or(1).max(1);
        lines.push(Line::from(Span::styled("Traffic per day (download █, upload ▒)", app.theme.title())));
        for d in &days {
            let w = body.width.saturating_sub(34) as f64;
            let down = ((d.rx_bytes as f64 / max as f64) * w).round() as usize;
            let up = ((d.tx_bytes as f64 / max as f64) * w).round() as usize;
            lines.push(Line::from(vec![
                Span::styled(format!("{}  ", d.date), app.theme.muted()),
                Span::styled("█".repeat(down), Style::default().fg(app.theme.cpu)),
                Span::styled("▒".repeat(up), Style::default().fg(app.theme.gpu)),
                Span::styled(format!("  {:.1} GB", (d.rx_bytes + d.tx_bytes) as f64 / 1e9), app.theme.muted()),
            ]));
        }
        if days.is_empty() {
            lines.push(Line::from(Span::styled("No traffic recorded yet.", app.theme.muted())));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Traffic per app: `ug net` (its page comes with ping stability).", app.theme.faint())));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    }
}

/// One Braille line chart of up to two series against seconds ago.
#[allow(clippy::too_many_arguments)]
fn live_chart(app: &App, f: &mut Frame, r: Rect, title: &str, unit: &str, min: (f64, f64), a: (&str, Vec<(f64, f64)>), b: Option<(&str, Vec<(f64, f64)>)>) {
    let all = a.1.iter().chain(b.iter().flat_map(|x| x.1.iter()));
    let lo = all.clone().map(|p| p.1).fold(min.0, f64::min).floor();
    let hi = all.map(|p| p.1).fold(min.1, f64::max).ceil();
    let now = a.1.last().map(|p| p.1);
    let head = match now {
        Some(v) if unit.contains("MB") || unit.contains("GB") || unit == " W" => format!("{title}  {v:.1}{unit}"),
        Some(v) => format!("{title}  {v:.0}{unit}"),
        None => title.to_string(),
    };
    let mut sets = vec![Dataset::default().name(a.0.to_string()).marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(app.theme.cpu)).data(&a.1)];
    if let Some((name, d)) = &b {
        if !d.is_empty() {
            sets.push(Dataset::default().name(name.to_string()).marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(app.theme.gpu)).data(d));
        }
    }
    let two = sets.len() > 1;
    let chart = Chart::new(sets)
        .legend_position(two.then_some(ratatui::widgets::LegendPosition::TopLeft))
        .block(card(app, &head))
        .x_axis(Axis::default().bounds([-120.0, 0.0]))
        .y_axis(Axis::default().style(app.theme.faint()).bounds([lo, hi.max(lo + 1.0)]).labels([Span::raw(format!("{lo:.0}")), Span::raw(format!("{hi:.0}"))]));
    f.render_widget(chart, r);
}

/// The last two minutes, one reading a second.
fn live(app: &mut App, f: &mut Frame, r: Rect) {
    if app.live.len() < 2 {
        let msg = app.live_due.error.clone().unwrap_or_else(|| "Collecting readings…".into());
        f.render_widget(Paragraph::new(Span::styled(msg, app.theme.muted())), r);
        return;
    }
    let t_end = app.live.back().map(|(s, _)| s.ts_unix_ms).unwrap_or(0) as f64 / 1000.0;
    let pts = |get: &dyn Fn(&ug_proto::Snapshot, &ug_proto::ExtraSample) -> f64| -> Vec<(f64, f64)> {
        app.live
            .iter()
            .filter_map(|(s, e)| {
                let v = get(s, e);
                (!v.is_nan()).then_some((s.ts_unix_ms as f64 / 1000.0 - t_end, v))
            })
            .collect()
    };
    let gpu = |s: &ug_proto::Snapshot| s.gpus.iter().find(|g| !g.asleep && !g.temp_c.is_nan()).cloned();
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
        let w = if e.battery_w.is_nan() { String::new() } else { format!(" {:.1} W", e.battery_w.abs()) };
        format!("{:.0} %{w}{}", e.battery_pct, if e.battery_status == "Charging" { " charging" } else { "" })
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
        Line::from(vec![Span::styled("Cores ", app.theme.muted()), Span::styled(cores, Style::default().fg(app.theme.cpu))]),
    ];
    f.render_widget(Paragraph::new(head), Rect { height: 2, ..r });
    let area = Rect { y: r.y + 3, height: r.height.saturating_sub(3), ..r };
    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Ratio(1, 4); 4]).split(area);
    let cell = |i: usize| {
        let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Ratio(1, 2); 2]).split(rows[i / 2]);
        cols[i % 2]
    };
    let has_gpu = app.live.iter().any(|(s, _)| gpu(s).is_some());
    let g = |get: fn(&ug_proto::GpuSample) -> f64| -> Option<(&'static str, Vec<(f64, f64)>)> {
        has_gpu.then(|| ("Graphics card", pts(&|s, _| gpu(s).map(|x| get(&x)).unwrap_or(f64::NAN))))
    };
    live_chart(app, f, cell(0), "Temperature", "°", (30.0, 70.0), ("Processor", pts(&|s, _| s.cpu.temp_c)), g(|x| x.temp_c));
    live_chart(app, f, cell(1), "Load", " %", (0.0, 100.0), ("Processor", pts(&|s, _| s.cpu.load_pct)), g(|x| x.load_pct));
    live_chart(app, f, cell(2), "Power", " W", (0.0, 30.0), ("Processor", pts(&|s, _| s.cpu.power_w)), g(|x| x.power_w));
    live_chart(app, f, cell(3), "Clock", " MHz", (0.0, 3000.0), ("Fastest core", pts(&|s, _| s.cpu.freq_max_mhz as f64)), g(|x| if x.core_clock_mhz == 0 { f64::NAN } else { x.core_clock_mhz as f64 }));
    let gib = 1_048_576.0;
    live_chart(app, f, cell(4), "Memory", " GB", (0.0, (s.mem.total_kb as f64 / gib).ceil()), ("Used", pts(&|s, _| s.mem.used_kb() as f64 / gib)), Some(("Swap", pts(&|s, _| s.mem.swap_used_kb as f64 / gib))));
    live_chart(app, f, cell(5), "Disk", " MB/s", (0.0, 5.0), ("Read", pts(&|_, e| e.disk_read_bps as f64 / 1e6)), Some(("Write", pts(&|_, e| e.disk_write_bps as f64 / 1e6))));
    live_chart(app, f, cell(6), "Internet", " MB/s", (0.0, 1.0), ("Down", pts(&|s, _| s.net.rx_bps as f64 / 1e6)), Some(("Up", pts(&|s, _| s.net.tx_bps as f64 / 1e6))));
    if e.battery_pct.is_nan() {
        live_chart(app, f, cell(7), "Load average", "", (0.0, 4.0), ("1 minute", pts(&|_, e| e.load_avg_1)), None);
    } else {
        live_chart(app, f, cell(7), "Battery power", " W", (0.0, 20.0), ("Battery", pts(&|_, e| e.battery_w)), None);
    }
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
            lines.push(Line::from(Span::styled("MangoHud is not installed. Install it with:", t.text())));
            lines.push(Line::from(Span::styled(format!("  {}", fr.install_hint), t.accent())));
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
            lines.push(Line::from(Span::styled("Other launchers: start the game with MANGOHUD=1 or through the mangohud command.", t.faint())));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(" Enter  Undo the setup ", t.muted())));
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
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), Rect { height: h.min(r.height), ..r });
    if fr.state == "live" && fr.recent.len() >= 2 {
        let end = fr.recent.last().map(|p| p.0).unwrap_or(0) as f64 / 1000.0;
        let d: Vec<(f64, f64)> = fr.recent.iter().map(|(ts, v)| (*ts as f64 / 1000.0 - end, *v)).collect();
        let hi = d.iter().map(|p| p.1).fold(60.0, f64::max).ceil();
        let chart = Chart::new(vec![Dataset::default().name("FPS").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(t.cpu)).data(&d)])
            .block(card(app, "FPS, last ten seconds"))
            .x_axis(Axis::default().bounds([-10.0, 0.0]))
            .y_axis(Axis::default().style(t.faint()).bounds([0.0, hi]).labels([Span::raw("0"), Span::raw(format!("{hi:.0}"))]));
        f.render_widget(chart, Rect { y: r.y + h + 1, height: r.height.saturating_sub(h + 1).min(16), ..r });
    }
}

/// The page's own keys, for the key bar.
pub fn keys(app: &App) -> Vec<(&'static str, &'static str)> {
    match (app.page, app.tab) {
        (1, _) => vec![("←→", "profile"), ("Enter", "toggle")],
        (2, 0) => vec![("←→", "when"), ("Enter", "run")],
        (2, 1) => vec![("Tab", "next tab"), ("x", "cancel")],
        (2, _) => vec![("Enter", "choose"), ("←→", "limit")],
        (3, 0) => vec![("Tab", "traffic"), ("←→", "range")],
        (3, 1) => vec![("Tab", "FPS")],
        (3, _) => vec![("Tab", "temperature"), ("Enter", "set up / undo")],
        _ => vec![],
    }
}

pub fn set_choice(app: &mut App, i: usize) {
    app.choice = i;
    app.armed = None;
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
    let max = match (app.page, app.tab) {
        (1, _) => app.profiles.value.as_ref().map(|p| p.len()).unwrap_or(1),
        (2, 0) => WHEN.len(),
        (2, 2) => 11,
        (3, 0) => RANGES.len(),
        _ => return,
    };
    let next = (app.choice as i32 + d).clamp(0, max as i32 - 1) as usize;
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
        (3, 2) => {
            let on = app.fps.value.as_ref().is_some_and(|f| f.state == "not-set-up");
            if !matches!(app.fps.value.as_ref().map(|f| f.state.as_str()), Some("not-set-up" | "idle" | "live")) {
                return;
            }
            let r = app.bus.set_up_fps(on);
            report(app, r.map(|_| if on { "FPS reading set up in MangoHud.conf.".into() } else { "MangoHud.conf is back as it was.".into() }));
            app.fps.invalidate();
            let _ = i;
        }
        (1, _) => {
            let Some(profiles) = app.profiles.value.clone() else { return };
            let p = &profiles[app.choice.min(profiles.len() - 1)];
            if i < OPTIONS.len() {
                let (key, label, _) = OPTIONS[i];
                let now = [p.cpu_performance, p.cpu_no_idle_states, p.cpu_preferred_cores, p.cpu_priority, p.memory_tuning, p.pause_services, p.network_tuning][i];
                let r = app.bus.set_option(&p.id, key, !now);
                report(app, r.map(|_| format!("{}: {label} {}.", p.id, if now { "off" } else { "on" })));
            } else if let Some(g) = app.installed.value.clone().and_then(|g| g.get(i - OPTIONS.len()).cloned()) {
                // Cycle: suggested → each profile → suggested.
                let current = profiles.iter().position(|p| p.games.contains(&g.id));
                let next = match current {
                    None => profiles.first().map(|p| p.id.clone()).unwrap_or_default(),
                    Some(k) if k + 1 < profiles.len() => profiles[k + 1].id.clone(),
                    Some(_) => String::new(),
                };
                let r = app.bus.set_game_profile(&g.id, &next);
                let shown = if next.is_empty() { "the suggested profile".to_string() } else { format!("the {next} profile") };
                report(app, r.map(|_| format!("{} now runs with {shown}.", g.name)));
            }
            app.profiles.invalidate();
        }
        (2, 0) => {
            let Some((id, label)) = ACTIONS.get(i).copied() else { return };
            if let Some(c) = app.caps.value.as_ref().and_then(|c| c.iter().find(|c| c.action == id)) {
                if !c.available {
                    let why = c.reason.clone();
                    app.say(why, false);
                    return;
                }
            }
            let (when, minutes) = WHEN[app.choice.min(WHEN.len() - 1)];
            if minutes > 0 {
                let r = app.bus.power_schedule(id, data::now_ms() + minutes * 60_000, "once");
                report(app, r.map(|j| format!("{label} scheduled {when} (#{}). Cancel it under Queue.", j.id)));
                app.jobs.invalidate();
                return;
            }
            let confirm = label.to_lowercase();
            if app.armed.as_ref().is_some_and(|(a, _)| *a == confirm) {
                app.armed = None;
                let r = app.bus.power_now(id);
                report(app, r.map(|_| format!("{label}: requested.")));
            } else {
                app.armed = Some((confirm, std::time::Instant::now()));
            }
        }
        (2, 1) => remove(app, i),
        (2, 2) => {
            let mode = ["off", "protect", "full-once"][i.min(2)];
            let limit = 50 + 5 * app.choice.min(10) as u8;
            let limit = if app.battery.value.as_ref().is_some_and(|b| b.adjustable) { limit } else { 80 };
            let r = app.bus.set_battery(mode, limit);
            report(app, r.map(|_| "Battery protection updated.".to_string()));
            app.battery.invalidate();
        }
        _ => {}
    }
}

pub fn remove(app: &mut App, i: usize) {
    if (app.page, app.tab) != (2, 1) {
        return;
    }
    let (jobs, watches) = app.jobs.value.clone().unwrap_or_default();
    let r = if let Some(j) = jobs.get(i) {
        app.bus.power_cancel(j.id).map(|_| format!("Cancelled: {} at {}.", j.action, data::local_time(j.at_unix_ms)))
    } else if let Some(w) = watches.get(i - jobs.len()) {
        app.bus.power_unwatch(w.id).map(|_| format!("Removed: {} ({}).", w.action, w.kind))
    } else {
        return;
    };
    report(app, r);
    app.jobs.invalidate();
    app.row = app.row.saturating_sub(1);
}
