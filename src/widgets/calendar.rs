//! Calendar: the current month and the ones after it, laid out the way `cal`
//! prints them.
//!
//! The panel answers "what is the date, and how far away is that?" — the
//! question a wall calendar answers and a clock does not. Showing the next
//! month alongside this one is the whole point: the useful lookups near a
//! month's end ("the 3rd is a Tuesday") fall off the edge of a single month.
//!
//! Deliberately offline and read-only. Reading an `.ics` file and showing
//! actual events is a separate, larger panel; this one is a date grid.

use jiff::civil::{Date, Weekday};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::config::CalendarConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::panel::{KeyOutcome, Panel, RenderContext};
use crate::theme::Theme;

const BINDINGS: &[Binding] = &[
    Binding::primary("n/p", "month"),
    Binding::primary("t", "today"),
    Binding::extra("←/→", "month"),
    Binding::extra("↑/↓", "year"),
    Binding::extra("wheel", "month"),
];

/// Seven two-cell columns with a space between them, exactly as `cal` prints.
const MONTH_WIDTH: u16 = 20;
/// Blank columns between two months laid side by side.
const GAP: u16 = 3;
/// Title, weekday header, and six week rows.
///
/// Six is used even for months that need five, so a month block is always the
/// same height. A block that grew and shrank as you scrolled would make the
/// whole panel jump, which is exactly the restlessness this dashboard avoids.
const MONTH_HEIGHT: u16 = 8;
const WEEK_ROWS: usize = 6;

/// Never put more than a year on screen at once. Past that the grid stops
/// being something you read and becomes something you search.
const MAX_MONTHS: usize = 12;

/// How far the view has been scrolled, in whole months.
type MonthOffset = i32;

pub struct CalendarPanel {
    config: CalendarConfig,
    /// Months from the current one. Zero means "showing today".
    offset: MonthOffset,
    today: Date,
}

impl CalendarPanel {
    pub fn new(config: CalendarConfig) -> Self {
        Self {
            config,
            offset: 0,
            today: jiff::Zoned::now().date(),
        }
    }

    /// The day the week starts on, per config.
    fn week_start(&self) -> Weekday {
        if self.config.week_starts.eq_ignore_ascii_case("monday") {
            Weekday::Monday
        } else {
            Weekday::Sunday
        }
    }

    /// Move the view, saturating rather than wrapping at the ends of the range
    /// `Date` can represent.
    fn scroll(&mut self, delta: MonthOffset) {
        let next = self.offset.saturating_add(delta);
        // Only accept a scroll that lands somewhere renderable, so holding a
        // key down parks at the boundary instead of blanking the panel.
        if shift_month(first_of_month(self.today), next).is_some() {
            self.offset = next;
        }
    }
}

/// The first of `date`'s month.
fn first_of_month(date: Date) -> Date {
    date.first_of_month()
}

/// `delta` months after `anchor`, as a first-of-month date.
///
/// Done in whole months rather than by adding days: "one month after 31 Jan"
/// has no single right answer in days, and anchoring to day 1 sidesteps the
/// question entirely.
fn shift_month(anchor: Date, delta: MonthOffset) -> Option<Date> {
    let months = MonthOffset::from(anchor.year()) * 12 + MonthOffset::from(anchor.month()) - 1;
    let total = months.checked_add(delta)?;
    let year = i16::try_from(total.div_euclid(12)).ok()?;
    let month = i8::try_from(total.rem_euclid(12) + 1).ok()?;
    Date::new(year, month, 1).ok()
}

/// Blank day cells before the 1st, given which weekday the week starts on.
fn leading_blanks(first: Date, week_start: Weekday) -> usize {
    let offset = if week_start == Weekday::Monday {
        first.weekday().to_monday_zero_offset()
    } else {
        first.weekday().to_sunday_zero_offset()
    };
    usize::try_from(offset).unwrap_or(0)
}

/// Two-letter weekday headings starting from `week_start`.
fn weekday_headings(week_start: Weekday) -> [&'static str; 7] {
    const FROM_SUNDAY: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
    const FROM_MONDAY: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
    if week_start == Weekday::Monday {
        FROM_MONDAY
    } else {
        FROM_SUNDAY
    }
}

fn month_name(month: i8) -> &'static str {
    const NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    NAMES
        .get(usize::try_from(month - 1).unwrap_or(0))
        .copied()
        .unwrap_or("")
}

/// Centre `text` in a field of exactly `width` cells.
///
/// The trailing padding matters as much as the leading: a month block is
/// composed left to right, so a title line that stops short of the full width
/// drags every month after it leftwards by the shortfall. Uneven remainders go
/// to the right, which is where `cal` puts them.
///
/// Month names are ASCII, so counting chars is counting cells here.
fn centred(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        // Ellipsised rather than left to hang over the edge. A title is prose,
        // so `Aug…` is a plainly abridged month where a title the terminal cut
        // would read as a shorter name.
        return crate::grid::truncate(text, width);
    }
    let left = (width - len) / 2;
    let right = width - len - left;
    format!("{:left$}{text}{:right$}", "", "")
}

/// Cells a month occupies when only `day_columns` of its seven weekdays fit.
///
/// Each day is two cells with one between them, so the whole-column widths are
/// 2, 5, 8 … 20 and nothing else. Any other width cuts a date down the middle,
/// and a date cut down the middle is still a date: `14` clipped to `1` under
/// THU is not a fragment the reader can spot, it is Thursday the 1st.
const fn month_width(day_columns: usize) -> usize {
    day_columns * 3 - 1
}

/// The most whole weekday columns that fit in `width`.
///
/// Zero when not even one day fits, which is the only honest answer at a
/// couple of cells — the panel then draws nothing rather than half a number.
const fn day_columns_for(width: u16) -> usize {
    let fits = (width as usize + 1) / 3;
    if fits > 7 { 7 } else { fits }
}

/// One month as exactly `MONTH_HEIGHT` lines of exactly
/// `month_width(day_columns)` cells.
///
/// Narrowing drops whole weekday columns from the end, the way the shared grid
/// drops whole table columns. What is left is true — the dates under SU MO TU
/// are the right dates — and it is visibly a cut-off calendar, which the
/// alternative was not.
fn month_block(
    first: Date,
    today: Date,
    week_start: Weekday,
    theme: &Theme,
    focused: bool,
    day_columns: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(usize::from(MONTH_HEIGHT));
    let width = month_width(day_columns);

    lines.push(Line::from(Span::styled(
        centred(
            &format!("{} {}", month_name(first.month()), first.year()),
            width,
        ),
        Style::default()
            .fg(theme.title)
            .add_modifier(Modifier::BOLD),
    )));

    lines.push(Line::from(Span::styled(
        weekday_headings(week_start)[..day_columns].join(" "),
        Style::default().fg(theme.label),
    )));

    let days = usize::try_from(first.days_in_month()).unwrap_or(0);
    let blanks = leading_blanks(first, week_start);
    let is_today_month = today.year() == first.year() && today.month() == first.month();

    let mut day = 1usize;
    for week in 0..WEEK_ROWS {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(13);
        for column in 0..7 {
            let leading = week == 0 && column < blanks;
            let drawn = !(leading || day > days);
            // A dropped column still consumes its date: the month must go on
            // advancing behind the edge of the panel, or the second week would
            // start on the wrong day.
            if column >= day_columns {
                if drawn {
                    day += 1;
                }
                continue;
            }
            if column > 0 {
                spans.push(Span::raw(" "));
            }
            if !drawn {
                spans.push(Span::raw("  "));
                continue;
            }
            let is_today = is_today_month && day == usize::try_from(today.day()).unwrap_or(0);
            let style = if is_today {
                // Reversed, as `cal` marks today: the brass becomes the
                // background, so the date reads as a stamped marker rather
                // than as one more coloured number among many.
                let base = Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD);
                if focused {
                    base
                } else {
                    // Unfocused panels recede; today stays marked but stops
                    // competing with the panel the keyboard is talking to.
                    Style::default()
                        .fg(theme.muted)
                        .add_modifier(Modifier::REVERSED)
                }
            } else {
                Style::default().fg(theme.text)
            };
            spans.push(Span::styled(format!("{day:>2}"), style));
            day += 1;
        }
        lines.push(Line::from(spans));
    }

    lines
}

/// How many months fit, and in what grid.
///
/// `months_across` caps the columns, which is what keeps the panel from
/// spreading sideways across a wide screen and starving its neighbours. Rows
/// then fill whatever height there is: `[calendar].months` is a floor on how
/// many months are shown, not a ceiling, so a tall panel stacks a second row
/// rather than leaving a void under the first.
///
/// Returns `(columns, rows)`, each at least one so there is always something to
/// draw; ratatui clips a block that genuinely does not fit.
fn grid_shape(area: Rect, months_across: usize) -> (usize, usize) {
    let fits_across = usize::from((area.width.saturating_add(GAP)) / (MONTH_WIDTH + GAP)).max(1);
    let columns = fits_across.min(months_across.max(1));
    let fits_down = usize::from(area.height / MONTH_HEIGHT).max(1);
    let rows = fits_down.min(MAX_MONTHS.div_ceil(columns).max(1));
    (columns, rows)
}

impl Panel for CalendarPanel {
    fn title(&self) -> String {
        "Calendar".to_string()
    }

    fn counter(&self) -> Option<String> {
        // Only worth the border space once the view has left today, where it
        // doubles as the way back: a non-empty counter means `t` will do
        // something.
        (self.offset != 0).then(|| format!("{:+} mo", self.offset))
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn max_width(&self) -> Option<u16> {
        // Months are laid out `months` across and no wider, so past this the
        // panel would be drawing the same grid with more blank around it. The
        // columns are better spent on the weather table or the task list.
        let months = u16::from(self.config.months.clamp(1, 12));
        Some(months * MONTH_WIDTH + (months - 1) * GAP + FRAME_WIDTH)
    }

    fn max_height(&self) -> Option<u16> {
        // Enough rows to reach a year at the configured width. Unlike the
        // width, this is a limit the panel rarely meets — extra height becomes
        // another row of months rather than a void, so there is little to
        // reclaim here and the figure mostly exists to stop the panel taking a
        // whole tall screen on its own.
        let columns = usize::from(self.config.months.clamp(1, 12));
        let rows = u16::try_from(MAX_MONTHS.div_ceil(columns)).unwrap_or(1);
        Some(rows * MONTH_HEIGHT + FRAME_HEIGHT)
    }

    fn refresh_interval(&self) -> std::time::Duration {
        // Nothing here changes faster than the date does. Polling every second
        // would spin the whole event loop for a panel that is idle all day.
        std::time::Duration::from_secs(30)
    }

    fn tick(&mut self) -> bool {
        // Re-read the date so the highlight crosses midnight on its own — but
        // report a change only when it actually crossed. This ran every 30
        // seconds and assigned unconditionally, so it repainted the dashboard
        // 2,880 times a day to move a highlight once.
        let today = jiff::Zoned::now().date();
        let moved = today != self.today;
        self.today = today;
        moved
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        match key.code {
            KeyCode::Char('n' | 'l') | KeyCode::Right => {
                self.scroll(1);
                KeyOutcome::Consumed
            }
            KeyCode::Char('p' | 'h') | KeyCode::Left => {
                self.scroll(-1);
                KeyOutcome::Consumed
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll(12);
                KeyOutcome::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll(-12);
                KeyOutcome::Consumed
            }
            KeyCode::Char('t') => {
                self.offset = 0;
                KeyOutcome::Consumed
            }
            _ => KeyOutcome::Ignored,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> KeyOutcome {
        match event.kind {
            MouseEventKind::ScrollDown => {
                self.scroll(1);
                KeyOutcome::Consumed
            }
            MouseEventKind::ScrollUp => {
                self.scroll(-1);
                KeyOutcome::Consumed
            }
            _ => KeyOutcome::Ignored,
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let across = usize::from(self.config.months.clamp(1, 12));
        let (columns, rows) = grid_shape(area, across);

        // A month is twenty cells and there is no honest way to squeeze one
        // into fewer, so a panel narrower than that shows fewer weekdays rather
        // than a narrower week. Below three cells not even one day fits and
        // there is nothing to draw; the frame still says which panel this is.
        let day_columns = if columns > 1 {
            7
        } else {
            day_columns_for(area.width)
        };
        if day_columns == 0 {
            return;
        }
        let week_start = self.week_start();
        let anchor = first_of_month(self.today);

        for row in 0..rows {
            // Each grid row is a strip of month blocks rendered line by line,
            // so the blocks stay aligned with each other horizontally.
            let mut strip: Vec<Line<'static>> = vec![Line::default(); usize::from(MONTH_HEIGHT)];
            let mut drew_any = false;

            for column in 0..columns {
                let index = row * columns + column;
                if index >= MAX_MONTHS {
                    break;
                }
                let Some(first) = shift_month(
                    anchor,
                    self.offset
                        .saturating_add(MonthOffset::try_from(index).unwrap_or(0)),
                ) else {
                    continue;
                };
                drew_any = true;

                let block = month_block(
                    first,
                    self.today,
                    week_start,
                    ctx.theme,
                    ctx.focused,
                    day_columns,
                );
                for (line_index, line) in block.into_iter().enumerate() {
                    let Some(target) = strip.get_mut(line_index) else {
                        continue;
                    };
                    if column > 0 {
                        target.spans.push(Span::raw(" ".repeat(usize::from(GAP))));
                    }
                    target.spans.extend(line.spans);
                }
            }

            if !drew_any {
                continue;
            }

            let y = area.y + u16::try_from(row).unwrap_or(0) * MONTH_HEIGHT;
            if y >= area.y + area.height {
                break;
            }
            let height = MONTH_HEIGHT.min(area.y + area.height - y);
            frame.render_widget(
                Paragraph::new(strip),
                Rect::new(area.x, y, area.width, height),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> CalendarPanel {
        CalendarPanel::new(CalendarConfig::default())
    }

    /// The reference output is real `cal` for July 2026, which is what the
    /// user asked this panel to look like.
    #[test]
    fn a_month_matches_what_cal_prints() {
        let july = Date::new(2026, 7, 1).unwrap();
        let theme = Theme::default();
        let block = month_block(july, july, Weekday::Sunday, &theme, true, 7);

        let text: Vec<String> = block
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();

        assert_eq!(text[0].trim(), "July 2026");
        assert_eq!(text[1], "Su Mo Tu We Th Fr Sa");
        assert_eq!(text[2], "          1  2  3  4");
        assert_eq!(text[3], " 5  6  7  8  9 10 11");
        assert_eq!(text[4], "12 13 14 15 16 17 18");
        assert_eq!(text[5], "19 20 21 22 23 24 25");
        assert_eq!(text[6], "26 27 28 29 30 31");
    }

    #[test]
    fn a_month_block_is_always_the_same_height() {
        let theme = Theme::default();
        // February 2026 needs five week rows; August 2026 starts on a Saturday
        // and needs six. Both must occupy the same space or the panel jumps as
        // it scrolls.
        for (year, month) in [(2026, 2), (2026, 8), (2026, 3)] {
            let first = Date::new(year, month, 1).unwrap();
            let block = month_block(first, first, Weekday::Sunday, &theme, true, 7);
            assert_eq!(
                block.len(),
                usize::from(MONTH_HEIGHT),
                "{month}/{year} was {} lines",
                block.len()
            );
        }
    }

    #[test]
    fn today_is_the_only_reversed_day() {
        let theme = Theme::default();
        let today = Date::new(2026, 7, 25).unwrap();
        let block = month_block(
            today.first_of_month(),
            today,
            Weekday::Sunday,
            &theme,
            true,
            7,
        );

        let reversed: Vec<String> = block
            .iter()
            .flat_map(|line| line.spans.iter())
            .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
            .map(|s| s.content.trim().to_string())
            .collect();

        assert_eq!(reversed, vec!["25".to_string()]);
    }

    #[test]
    fn today_is_not_marked_in_a_month_it_does_not_fall_in() {
        let theme = Theme::default();
        let today = Date::new(2026, 7, 25).unwrap();
        let august = Date::new(2026, 8, 1).unwrap();
        let block = month_block(august, today, Weekday::Sunday, &theme, true, 7);

        assert!(
            !block
                .iter()
                .flat_map(|line| line.spans.iter())
                .any(|s| s.style.add_modifier.contains(Modifier::REVERSED)),
            "a different month must not mark the 25th"
        );
    }

    #[test]
    fn a_monday_week_shifts_the_headings_and_the_blanks() {
        let theme = Theme::default();
        let july = Date::new(2026, 7, 1).unwrap();
        let block = month_block(july, july, Weekday::Monday, &theme, true, 7);
        let header: String = block[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "Mo Tu We Th Fr Sa Su");

        // 1 July 2026 is a Wednesday: two blanks from Monday, four from Sunday.
        assert_eq!(leading_blanks(july, Weekday::Monday), 2);
        assert_eq!(leading_blanks(july, Weekday::Sunday), 3);
    }

    #[test]
    fn shifting_months_crosses_year_boundaries_in_both_directions() {
        let december = Date::new(2026, 12, 1).unwrap();
        assert_eq!(
            shift_month(december, 1).unwrap(),
            Date::new(2027, 1, 1).unwrap()
        );
        assert_eq!(
            shift_month(december, 13).unwrap(),
            Date::new(2028, 1, 1).unwrap()
        );

        let january = Date::new(2026, 1, 1).unwrap();
        assert_eq!(
            shift_month(january, -1).unwrap(),
            Date::new(2025, 12, 1).unwrap()
        );
        assert_eq!(
            shift_month(january, -13).unwrap(),
            Date::new(2024, 12, 1).unwrap()
        );
    }

    #[test]
    fn shifting_off_the_end_of_the_calendar_is_refused_rather_than_wrapping() {
        let anchor = Date::new(2026, 7, 1).unwrap();
        assert!(shift_month(anchor, MonthOffset::MAX).is_none());
        assert!(shift_month(anchor, MonthOffset::MIN).is_none());
    }

    #[test]
    fn scrolling_saturates_instead_of_blanking_the_panel() {
        let mut p = panel();
        // Far past anything `Date` can represent; the offset must not move
        // somewhere that renders nothing.
        for _ in 0..40 {
            p.scroll(MonthOffset::MAX / 2);
        }
        assert!(
            shift_month(first_of_month(p.today), p.offset).is_some(),
            "offset {} does not resolve to a real month",
            p.offset
        );
    }

    #[test]
    fn t_returns_to_today_from_anywhere() {
        let mut p = panel();
        p.scroll(7);
        assert_ne!(p.offset, 0);
        p.handle_key(KeyEvent::from(KeyCode::Char('t')));
        assert_eq!(p.offset, 0);
        assert_eq!(p.counter(), None, "no counter while showing today");
    }

    #[test]
    fn the_wheel_moves_one_month_at_a_time() {
        use ratatui::crossterm::event::KeyModifiers;
        let mut p = panel();
        let wheel = |kind| MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        p.handle_mouse(wheel(MouseEventKind::ScrollDown), Rect::new(0, 0, 40, 10));
        assert_eq!(p.offset, 1);
        p.handle_mouse(wheel(MouseEventKind::ScrollUp), Rect::new(0, 0, 40, 10));
        assert_eq!(p.offset, 0);
    }

    #[test]
    fn the_counter_shows_which_way_the_view_moved() {
        let mut p = panel();
        p.scroll(2);
        assert_eq!(p.counter(), Some("+2 mo".to_string()));
        p.scroll(-4);
        assert_eq!(p.counter(), Some("-2 mo".to_string()));
    }

    #[test]
    fn every_month_title_is_centred_over_its_own_block() {
        // The bug this pins: `centred` left-padded without right-padding, so a
        // title line was short of the full block width and every month after
        // the first slid left by the shortfall.
        let theme = Theme::default();
        let july = Date::new(2026, 7, 1).unwrap();

        for (year, month) in [(2026, 7), (2026, 8), (2026, 9), (2026, 12)] {
            let first = Date::new(year, month, 1).unwrap();
            let block = month_block(first, july, Weekday::Sunday, &theme, true, 7);
            for (index, line) in block.iter().enumerate() {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(
                    text.chars().count(),
                    usize::from(MONTH_WIDTH),
                    "{month}/{year} line {index} is {text:?}, not {MONTH_WIDTH} cells"
                );
            }
        }
    }

    #[test]
    fn a_title_sits_over_the_weekday_header_it_belongs_to() {
        let theme = Theme::default();
        let text_of = |line: &Line<'static>| -> String {
            line.spans.iter().map(|s| s.content.as_ref()).collect()
        };

        for name in ["July 2026", "August 2026", "September 2026", "May 2026"] {
            let line = centred(name, usize::from(MONTH_WIDTH));
            let leading = line.len() - line.trim_start().len();
            let trailing = line.len() - line.trim_end().len();
            assert_eq!(line.chars().count(), usize::from(MONTH_WIDTH));
            assert!(
                trailing >= leading && trailing - leading <= 1,
                "`{name}` is off centre: {leading} left, {trailing} right"
            );
        }

        // And the composed block agrees: the weekday header is the full width,
        // so a correctly padded title cannot shift it.
        let block = month_block(
            Date::new(2026, 9, 1).unwrap(),
            Date::new(2026, 9, 1).unwrap(),
            Weekday::Sunday,
            &theme,
            true,
            7,
        );
        assert_eq!(text_of(&block[1]), "Su Mo Tu We Th Fr Sa");
    }

    #[test]
    fn narrow_panels_still_get_one_month() {
        // A panel too narrow for even a single block must still ask for one,
        // so the panel degrades to a clipped month rather than to nothing.
        assert_eq!(grid_shape(Rect::new(0, 0, 5, 20), 2).0, 1);
        assert_eq!(grid_shape(Rect::new(0, 0, 1, 1), 2), (1, 1));
    }

    #[test]
    fn months_sit_side_by_side_up_to_the_configured_width() {
        let wide = Rect::new(0, 0, MONTH_WIDTH * 2 + GAP, MONTH_HEIGHT);
        assert_eq!(grid_shape(wide, 2), (2, 1), "both months fit across");

        // Wider than two months still gives two columns: `months` is the width
        // budget, and spreading further would starve the neighbouring panels.
        let wider = Rect::new(0, 0, MONTH_WIDTH * 6, MONTH_HEIGHT);
        assert_eq!(grid_shape(wider, 2).0, 2, "the width cap holds");
    }

    #[test]
    fn a_tall_panel_stacks_another_row_of_months_rather_than_leaving_a_void() {
        let one_row = Rect::new(0, 0, MONTH_WIDTH * 2 + GAP, MONTH_HEIGHT);
        assert_eq!(grid_shape(one_row, 2), (2, 1));

        let two_rows = Rect::new(0, 0, MONTH_WIDTH * 2 + GAP, MONTH_HEIGHT * 2);
        assert_eq!(
            grid_shape(two_rows, 2),
            (2, 2),
            "twice the height is four months, not two and a gap"
        );

        let three_rows = Rect::new(0, 0, MONTH_WIDTH * 2 + GAP, MONTH_HEIGHT * 3);
        assert_eq!(grid_shape(three_rows, 2), (2, 3));
    }

    #[test]
    fn a_single_narrow_column_stacks_downwards() {
        let tall = Rect::new(0, 0, MONTH_WIDTH, MONTH_HEIGHT * 4);
        assert_eq!(grid_shape(tall, 2), (1, 4), "one across, four down");
    }

    #[test]
    fn a_year_is_the_most_that_is_ever_shown() {
        // Past twelve the grid stops being something you read.
        let enormous = Rect::new(0, 0, MONTH_WIDTH * 2 + GAP, MONTH_HEIGHT * 40);
        let (columns, rows) = grid_shape(enormous, 2);
        assert!(
            columns * rows <= MAX_MONTHS,
            "{columns}x{rows} is more than a year"
        );
        assert_eq!(rows, 6, "six rows of two is exactly a year");
    }

    #[test]
    fn the_declared_width_matches_what_the_months_actually_need() {
        // The layout hands the surplus to a neighbour based on this figure, so
        // a wrong one either starves the calendar or wastes the space anyway.
        let panel = CalendarPanel::new(CalendarConfig {
            months: 2,
            ..CalendarConfig::default()
        });
        let declared = panel.max_width().unwrap();
        let interior = declared - FRAME_WIDTH;
        assert_eq!(
            grid_shape(Rect::new(0, 0, interior, MONTH_HEIGHT), 2),
            (2, 1),
            "the declared width must fit exactly the months it claims"
        );
        assert_eq!(
            grid_shape(Rect::new(0, 0, interior - 1, MONTH_HEIGHT), 2).0,
            1,
            "and one column less must not"
        );
    }
}
