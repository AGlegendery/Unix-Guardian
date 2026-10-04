//! The Test lab page: the same engine as the window and `ug testlab`
//! (ug-testlab), run on a thread of its own so the screen keeps drawing.
//!
//! Why it exists, one "Run automatic test" row, a confirmation listing what
//! turns on for a moment, a progress bar, then the report to copy (c) or
//! save (s) and email.

use crate::{App, Hit};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Wrap};
use ratatui::Frame;
use std::sync::{Arc, Mutex};
use ug_testlab::{Progress, Report};

const WHY: &str = "Unix Guardian is made for every Linux system, but so far it has only been \
     tested on a few computers. This test tries every feature on yours for a moment, measures a \
     few things, puts everything back, and writes a report. Sending it is the best way to help \
     Unix Guardian work on machines like yours.";
const NEVER: &str = "It never logs out, restarts, installs packages or changes the desktop \
     session. Personal details are removed from the report and nothing is sent by itself. Close \
     games and recordings first; it takes about a minute.";

#[derive(Default)]
struct Shared {
    running: bool,
    progress: Option<Progress>,
    report: Option<Result<Report, String>>,
    recovered: Vec<String>,
}

pub struct Lab {
    shared: Arc<Mutex<Shared>>,
    pub daemon_session: bool,
    /// Showing what will be turned on, waiting for Start or Cancel.
    pub confirm: bool,
    /// First report line shown.
    pub scroll: u16,
}

impl Lab {
    pub fn new(daemon_session: bool) -> Lab {
        Lab { shared: Default::default(), daemon_session, confirm: false, scroll: 0 }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn options(&self) -> ug_testlab::Options {
        ug_testlab::Options::new("terminal UI", self.daemon_session)
    }

    /// At start: puts back what an interrupted test left.
    pub fn recover_in_background(&self) {
        let shared = Arc::clone(&self.shared);
        let opts = self.options();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            let done = rt.block_on(ug_testlab::recover(&opts));
            shared.lock().unwrap_or_else(|e| e.into_inner()).recovered = done;
        });
    }

    pub fn running(&self) -> bool {
        self.lock().running
    }

    fn start(&mut self) {
        let mut s = self.lock();
        if s.running {
            return;
        }
        *s = Shared { running: true, recovered: std::mem::take(&mut s.recovered), ..Default::default() };
        drop(s);
        self.confirm = false;
        self.scroll = 0;
        let shared = Arc::clone(&self.shared);
        let opts = self.options();
        std::thread::spawn(move || {
            let r = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => {
                    let progress = Arc::clone(&shared);
                    rt.block_on(ug_testlab::run(&opts, move |p| {
                        progress.lock().unwrap_or_else(|e| e.into_inner()).progress = Some(p);
                    }))
                }
                Err(e) => Err(e.to_string()),
            };
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.report = Some(r);
            s.running = false;
        });
    }

    fn report_text(&self) -> Option<String> {
        self.lock().report.as_ref().and_then(|r| r.as_ref().ok()).map(|r| r.text.clone())
    }
}

/// The rows under the explanation, by state.
fn rows(app: &App) -> Vec<&'static str> {
    let s = app.lab.lock();
    if s.running {
        vec![]
    } else if app.lab.confirm {
        vec!["Start the test", "Cancel"]
    } else if matches!(s.report, Some(Ok(_))) {
        vec!["Copy report (c)", "Save to a file (s)", "Run again"]
    } else {
        vec!["Run automatic test"]
    }
}

pub fn keys(app: &App) -> Vec<(&'static str, &'static str)> {
    if app.lab.running() {
        vec![]
    } else if app.lab.report_text().is_some() && !app.lab.confirm {
        vec![("Enter", "choose"), ("c", "copy"), ("s", "save"), ("PgUp/PgDn", "scroll report")]
    } else {
        vec![("Enter", "choose")]
    }
}

pub fn activate(app: &mut App, i: usize) {
    let rows = rows(app);
    match rows.get(i).copied() {
        Some("Run automatic test") | Some("Run again") => {
            app.lab.confirm = true;
            app.row = 0;
        }
        Some("Start the test") => app.lab.start(),
        Some("Cancel") => {
            app.lab.confirm = false;
            app.row = 0;
        }
        Some("Copy report (c)") => copy(app),
        Some("Save to a file (s)") => save(app),
        _ => {}
    }
}

pub fn scroll(app: &mut App, down: bool) {
    let lines = app.lab.report_text().map(|t| t.lines().count()).unwrap_or(0) as u16;
    app.lab.scroll = if down { (app.lab.scroll + 10).min(lines.saturating_sub(5)) } else { app.lab.scroll.saturating_sub(10) };
}

/// The desktop's clipboard tool if there is one, else the terminal's own
/// clipboard escape (OSC 52), which most terminals honour.
pub fn copy(app: &mut App) {
    let Some(text) = app.lab.report_text() else { return };
    use std::io::Write;
    let tools: [(&str, &[&str]); 3] = [("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])];
    for (tool, args) in tools {
        let child = std::process::Command::new(tool)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(mut c) = child {
            let ok = c.stdin.take().is_some_and(|mut i| i.write_all(text.as_bytes()).is_ok());
            if ok && c.wait().is_ok_and(|s| s.success()) {
                return app.say(format!("Copied. Paste it into an email to {}.", ug_core::BUG_REPORT_EMAIL), true);
            }
        }
    }
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
    app.say("Sent to the terminal's clipboard. If pasting gives nothing, save it with s instead.", true);
}

pub fn save(app: &mut App) {
    let Some(text) = app.lab.report_text() else { return };
    let stamp = ug_core::playtime::local_time(crate::data::now_ms()).replace([' ', ':'], "-");
    let path = ug_core::playtime::downloads_dir().join(format!("unix-guardian-test-report-{stamp}.txt"));
    match std::fs::write(&path, text) {
        Ok(()) => app.say(format!("Saved: {}", path.display()), true),
        Err(e) => app.say(format!("{}: {e}", path.display()), false),
    }
}

fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

pub fn page(app: &mut App, f: &mut Frame, r: Rect) {
    let t = &app.theme;
    let width = r.width.max(20);
    let mut lines = vec![Line::from(Span::styled("Test lab", t.title())), Line::from("")];
    lines.push(Line::from(Span::styled(WHY, t.text())));
    let (running, progress, report, recovered) = {
        let s = app.lab.lock();
        (s.running, s.progress.clone(), s.report.clone(), s.recovered.clone())
    };
    if !recovered.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("An earlier test was interrupted. Put back: {}", recovered.join("; ")), t.muted())));
    }
    if app.lab.confirm {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("For a moment it turns on, then off again:", t.text())));
        for item in ug_testlab::TEMPORARY_EN {
            lines.push(Line::from(Span::styled(format!("  • {item}"), t.text())));
        }
        lines.push(Line::from(Span::styled(NEVER, t.muted())));
    }
    if let Some(Err(e)) = &report {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(e.clone(), t.warn_style())));
    }
    // Height of the wrapped text above the rows.
    let text_h: u16 = lines
        .iter()
        .map(|l| (l.width() as u16).div_ceil(width).max(1))
        .sum::<u16>()
        .min(r.height);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), Rect { height: text_h, ..r });
    let mut y = r.y + text_h + 1;
    let bottom = r.y + r.height;

    if running {
        let (pct, label) = progress.map(|p| ((p.done * 100 / p.total.max(1)) as u16, format!("{}/{} · {}", (p.done + 1).min(p.total), p.total, p.label))).unwrap_or((0, "Starting".into()));
        if y + 3 <= bottom {
            f.render_widget(Paragraph::new(Span::styled("Testing…", t.text())), Rect { x: r.x, y, width: r.width, height: 1 });
            let g = Gauge::default().percent(pct.min(100)).label(format!("{pct} %")).gauge_style(t.accent());
            f.render_widget(g, Rect { x: r.x, y: y + 1, width: r.width.min(60), height: 1 });
            f.render_widget(Paragraph::new(Span::styled(label, t.muted())), Rect { x: r.x, y: y + 2, width: r.width, height: 1 });
        }
        app.rows = 0;
        return;
    }

    let rows = rows(app);
    app.rows = rows.len();
    app.row = app.row.min(rows.len().saturating_sub(1));
    for (i, label) in rows.iter().enumerate() {
        if y >= bottom {
            break;
        }
        let sel = i == app.row;
        let style = if sel { t.selected() } else { t.text().add_modifier(Modifier::BOLD) };
        let area = Rect { x: r.x, y, width: (label.len() as u16 + 4).min(r.width), height: 1 };
        f.render_widget(Paragraph::new(Span::styled(format!(" ▶ {label} "), style)), area);
        app.hits.push((area, Hit::Row(i)));
        y += 1;
    }

    if let (Some(Ok(rep)), false) = (&report, app.lab.confirm) {
        y += 1;
        if y < bottom {
            let summary = format!(
                "{} passed, {} warnings, {} failed, {} skipped. Copy or save it and email it to {}.",
                rep.passed,
                rep.warned,
                rep.failed,
                rep.skipped,
                ug_core::BUG_REPORT_EMAIL
            );
            f.render_widget(Paragraph::new(Span::styled(summary, t.text())).wrap(Wrap { trim: true }), Rect { x: r.x, y, width: r.width, height: 2.min(bottom - y) });
            y += 3;
        }
        if y < bottom {
            let body: Vec<Line> = rep.text.lines().map(|l| Line::from(Span::styled(l.to_string(), t.muted()))).collect();
            f.render_widget(Paragraph::new(body).scroll((app.lab.scroll, 0)), Rect { x: r.x, y, width: r.width, height: bottom - y });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Bus;
    use ratatui::backend::TestBackend;

    fn render(app: &mut App) -> String {
        let mut term = ratatui::Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..40).map(|y| (0..120).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect()
    }

    #[test]
    fn the_test_lab_explains_asks_first_and_then_shows_the_report() {
        let mut app = App::new(Bus::offline());
        app.go(10);
        let screen = render(&mut app);
        for want in ["Test lab", "only been", "Run automatic test"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        assert!(!screen.contains("Not built yet"));
        activate(&mut app, 0);
        let screen = render(&mut app);
        for want in ["turns on, then off again", "game mode", "never logs out", "Start the test", "Cancel"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        activate(&mut app, 1);
        assert!(!app.lab.confirm, "Cancel goes back");
        app.lab.lock().report = Some(Ok(Report {
            text: "Unix Guardian test lab report\nRESULT: 5 passed".into(),
            passed: 5,
            ..Default::default()
        }));
        let screen = render(&mut app);
        for want in ["Copy report", "Save to a file", "Run again", "5 passed", "unixguardian@atomicmail.io", "RESULT: 5 passed"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    #[test]
    fn the_clipboard_escape_is_standard_base64() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64("گزارش".as_bytes()), "2q/Ystin2LHYtA==");
    }
}
