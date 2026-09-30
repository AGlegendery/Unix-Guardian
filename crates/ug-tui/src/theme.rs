//! The window's palette in the terminal: dark ground, one purple accent.
//! Truecolor terminals get the exact colours; others the nearest ANSI ones,
//! chosen so text stays readable on both dark and light terminal themes.

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub line: Color,
    pub ok: Color,
    pub warn: Color,
    pub danger: Color,
    pub cpu: Color,
    pub gpu: Color,
    pub selected_bg: Color,
}

impl Theme {
    pub fn detect() -> Theme {
        let truecolor = std::env::var("COLORTERM").is_ok_and(|v| v.contains("truecolor") || v.contains("24bit"));
        if truecolor {
            Theme {
                accent: Color::Rgb(139, 92, 246),
                text: Color::Rgb(236, 234, 246),
                muted: Color::Rgb(177, 174, 203),
                faint: Color::Rgb(129, 126, 158),
                line: Color::Rgb(58, 55, 86),
                ok: Color::Rgb(52, 211, 153),
                warn: Color::Rgb(251, 191, 36),
                danger: Color::Rgb(248, 113, 113),
                // The window's chart colours, checked for colour-blind separation.
                cpu: Color::Rgb(143, 106, 242),
                gpu: Color::Rgb(201, 133, 0),
                selected_bg: Color::Rgb(44, 36, 74),
            }
        } else {
            Theme {
                accent: Color::Magenta,
                text: Color::Reset,
                muted: Color::Gray,
                faint: Color::DarkGray,
                line: Color::DarkGray,
                ok: Color::Green,
                warn: Color::Yellow,
                danger: Color::Red,
                cpu: Color::Magenta,
                gpu: Color::Yellow,
                selected_bg: Color::DarkGray,
            }
        }
    }

    pub fn text(&self) -> Style {
        Style::default().fg(self.text)
    }
    pub fn muted(&self) -> Style {
        Style::default().fg(self.muted)
    }
    pub fn faint(&self) -> Style {
        Style::default().fg(self.faint)
    }
    pub fn title(&self) -> Style {
        Style::default().fg(self.text).add_modifier(Modifier::BOLD)
    }
    pub fn accent(&self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }
    pub fn selected(&self) -> Style {
        Style::default().bg(self.selected_bg).fg(self.text).add_modifier(Modifier::BOLD)
    }
    pub fn warn_style(&self) -> Style {
        Style::default().fg(self.warn)
    }
    pub fn border(&self) -> Style {
        Style::default().fg(self.line)
    }
}
