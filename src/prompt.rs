//! A one-line question a panel can ask.
//!
//! Most of what the dashboard lets you change is a toggle — units, sort order,
//! whether seconds show — and a toggle needs no interface beyond the key that
//! flips it. Three settings are not toggles: the agenda's `.ics` file, the
//! weather location, and the name of a timezone to add to the clock. Each of
//! those is free text, and each was stuck behind "edit your config file"
//! for want of somewhere to type.
//!
//! This is that somewhere, once, rather than three times. A panel opens it with
//! a label and the value as it stands, reads [`Outcome`] back out of
//! `handle_key`, and decides for itself whether the answer is any good — the
//! prompt has no idea what a timezone is and does not want one. A rejected
//! answer comes back with [`Prompt::reject`] and the dialog stays open with the
//! text still in it, because retyping a long path to fix one character is the
//! kind of thing that stops people using a feature at all.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::textfield::TextField;
use crate::theme::Theme;

/// What `Tab` offers to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// Nothing; `Tab` does nothing.
    None,
    /// Filesystem paths, for the agenda file.
    Paths,
    /// A fixed list the caller supplies, for timezone names.
    Names(&'static [&'static str]),
}

/// What a keypress did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Still typing.
    Editing,
    /// Backed out; the panel should close the prompt and change nothing.
    Cancelled,
    /// The user pressed Enter. The panel decides whether to accept it.
    Submitted(String),
}

/// An open prompt.
#[derive(Debug)]
pub struct Prompt {
    label: &'static str,
    help: &'static str,
    field: TextField,
    error: Option<String>,
    completion: Completion,
}

impl Prompt {
    /// Ask `label`, starting from `value`.
    pub fn new(
        label: &'static str,
        help: &'static str,
        value: &str,
        completion: Completion,
    ) -> Self {
        Self {
            label,
            help,
            field: TextField::with_value(value),
            error: None,
            completion,
        }
    }

    /// Refuse the answer and say why, leaving the text in place to be fixed.
    pub fn reject(&mut self, why: impl Into<String>) {
        self.error = Some(why.into());
    }

    /// The text as it stands. Exposed for tests.
    #[cfg(test)]
    pub fn value(&self) -> &str {
        self.field.value()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        // A keystroke means the user is dealing with the complaint, so the
        // complaint goes.
        self.error = None;

        match key.code {
            KeyCode::Esc => Outcome::Cancelled,
            KeyCode::Enter => Outcome::Submitted(self.field.trimmed().to_string()),
            KeyCode::Tab => {
                self.complete();
                Outcome::Editing
            }
            _ => {
                self.field.handle_key(key);
                Outcome::Editing
            }
        }
    }

    /// Extend the text as far as every candidate agrees.
    ///
    /// Completing only to the common prefix is what a shell does, and it is the
    /// behaviour that never surprises: it either finishes the job or stops at
    /// the point where a choice genuinely has to be made. Completing to the
    /// first match instead would silently pick one of several.
    fn complete(&mut self) {
        let typed = self.field.value().to_string();
        let (fixed, stem) = match self.completion {
            Completion::None => return,
            Completion::Paths => match typed.rfind('/') {
                Some(cut) => (typed[..=cut].to_string(), typed[cut + 1..].to_string()),
                None => (String::new(), typed.clone()),
            },
            Completion::Names(_) => (String::new(), typed.clone()),
        };

        let candidates = self.candidates(&fixed, &stem);
        let Some(shared) = common_prefix(&candidates) else {
            return;
        };
        if shared.len() <= stem.len() {
            return;
        }

        self.field = TextField::with_value(format!("{fixed}{shared}"));
        self.field.end();
    }

    fn candidates(&self, fixed: &str, stem: &str) -> Vec<String> {
        match self.completion {
            Completion::None => Vec::new(),
            Completion::Names(names) => names
                .iter()
                .filter(|name| {
                    name.len() >= stem.len() && name[..stem.len()].eq_ignore_ascii_case(stem)
                })
                .map(|name| (*name).to_string())
                .collect(),
            Completion::Paths => {
                let directory = expand_tilde(if fixed.is_empty() { "./" } else { fixed });
                let Ok(entries) = std::fs::read_dir(&directory) else {
                    return Vec::new();
                };
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if !name.starts_with(stem) {
                            return None;
                        }
                        // A trailing slash on a directory means one Tab gets you
                        // into it rather than up to its edge.
                        let directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
                        Some(if directory { format!("{name}/") } else { name })
                    })
                    .collect()
            }
        }
    }

    /// Draw the prompt over the middle of `area`.
    pub fn render(&self, frame: &mut ratatui::Frame, area: Rect, theme: &Theme) {
        let width = area.width.clamp(20, 64);
        let popup = crate::frame::centred(area, width, 3 + crate::frame::FRAME_HEIGHT);

        // Room for the text, inside the border and its padding.
        let inner = usize::from(width).saturating_sub(usize::from(crate::frame::FRAME_WIDTH));
        let (visible, cursor) = self.field.visible(inner);

        let mut lines = vec![
            Line::from(Span::styled(visible, Style::default().fg(theme.text))),
            Line::from(""),
        ];
        lines.push(match &self.error {
            Some(error) => Line::from(Span::styled(
                crate::grid::truncate(error, inner),
                Style::default().fg(theme.error),
            )),
            None => Line::from(Span::styled(
                crate::grid::truncate(self.help, inner),
                Style::default().fg(theme.muted),
            )),
        });

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_focused))
                    .padding(Padding::horizontal(1))
                    .title_top(Line::from(Span::styled(
                        self.label,
                        Style::default()
                            .fg(theme.title)
                            .add_modifier(Modifier::BOLD),
                    ))),
            ),
            popup,
        );

        // A visible cursor, because a text field without one reads as a label.
        let x = popup.x + 2 + u16::try_from(cursor).unwrap_or(0);
        frame.set_cursor_position((x.min(popup.x + popup.width - 2), popup.y + 1));
    }
}

/// Replace a leading `~` with the user's home directory.
pub fn expand_tilde(path: &str) -> std::path::PathBuf {
    let Some(rest) = path.strip_prefix('~') else {
        return std::path::PathBuf::from(path);
    };
    let Some(home) = dirs::home_dir() else {
        return std::path::PathBuf::from(path);
    };
    home.join(rest.trim_start_matches('/'))
}

/// The longest start every candidate shares, or `None` if there are none.
fn common_prefix(candidates: &[String]) -> Option<String> {
    let first = candidates.first()?;
    let mut shared = first.len();
    for other in &candidates[1..] {
        shared = shared.min(
            first
                .char_indices()
                .zip(other.char_indices())
                .take_while(|((_, a), (_, b))| a == b)
                .map(|((index, a), _)| index + a.len_utf8())
                .last()
                .unwrap_or(0),
        );
    }
    Some(first[..shared].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONES: &[&str] = &[
        "Europe/London",
        "Europe/Lisbon",
        "Europe/Madrid",
        "Asia/Tokyo",
    ];

    fn prompt(value: &str) -> Prompt {
        Prompt::new("ZONE", "help", value, Completion::Names(ZONES))
    }

    fn press(prompt: &mut Prompt, code: KeyCode) -> Outcome {
        prompt.handle_key(KeyEvent::from(code))
    }

    #[test]
    fn enter_submits_the_trimmed_text_and_esc_abandons_it() {
        let mut p = prompt("  Asia/Tokyo  ");
        assert_eq!(
            press(&mut p, KeyCode::Enter),
            Outcome::Submitted("Asia/Tokyo".into())
        );
        assert_eq!(press(&mut prompt("x"), KeyCode::Esc), Outcome::Cancelled);
    }

    /// Completing to the first match would pick Lisbon over London with nothing
    /// on screen saying a choice had been made.
    #[test]
    fn tab_stops_where_the_candidates_stop_agreeing() {
        let mut p = prompt("Europe/L");
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.value(), "Europe/L", "London and Lisbon differ here");

        let mut p = prompt("Europe/Lo");
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.value(), "Europe/London", "only one candidate left");
    }

    #[test]
    fn tab_on_nothing_matching_leaves_the_text_alone() {
        let mut p = prompt("Mars/");
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.value(), "Mars/");
    }

    /// The whole reason a rejection keeps the text: a path is long, and being
    /// sent back to an empty box to fix one character is how a feature stops
    /// getting used.
    #[test]
    fn a_rejected_answer_keeps_what_was_typed_and_the_next_key_clears_the_complaint() {
        let mut p = prompt("Europe/Lundon");
        p.reject("no such timezone");
        assert!(p.error.is_some());

        press(&mut p, KeyCode::Backspace);
        assert!(p.error.is_none(), "typing dismisses the complaint");
        assert_eq!(p.value(), "Europe/Lundo", "and the text is still there");
    }

    #[test]
    fn completion_can_be_turned_off_entirely() {
        let mut p = Prompt::new("CITY", "help", "Bost", Completion::None);
        press(&mut p, KeyCode::Tab);
        assert_eq!(
            p.value(),
            "Bost",
            "a city name has nothing to complete from"
        );
    }
}
