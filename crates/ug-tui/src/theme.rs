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
    pub mem: Color,
    pub net: Color,
    pub power: Color,
    pub selected_bg: Color,
}

impl Theme {
    pub fn detect() -> Theme {
        let truecolor = std::env::var("COLORTERM")
            .is_ok_and(|v| v.contains("truecolor") || v.contains("24bit"));
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
                // The window's category colours: processor blue, graphics
                // green, memory teal, network light violet, power yellow.
                cpu: Color::Rgb(96, 165, 250),
                gpu: Color::Rgb(52, 211, 153),
                mem: Color::Rgb(45, 212, 191),
                net: Color::Rgb(196, 181, 253),
                power: Color::Rgb(250, 204, 21),
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
                cpu: Color::Blue,
                gpu: Color::Green,
                mem: Color::Cyan,
                net: Color::LightMagenta,
                power: Color::Yellow,
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
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }
    pub fn selected(&self) -> Style {
        Style::default()
            .bg(self.selected_bg)
            .fg(self.text)
            .add_modifier(Modifier::BOLD)
    }
    pub fn warn_style(&self) -> Style {
        Style::default().fg(self.warn)
    }
    pub fn border(&self) -> Style {
        Style::default().fg(self.line)
    }
}
