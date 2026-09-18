//! Disk space: every volume worth watching, how full it is, and what is left.
//!
//! The last quarter of "what compute is available", and the one every monitor
//! this program borrows from shows under memory. It exists to say one thing
//! before it matters: this volume is filling up. So the figure that leads is
//! how full, the one beside it is what is free, and the colour changes only
//! at the thresholds — brass while there is room, amber past
//! `warn_above_pct`, red past `alert_above_pct`, where the status bar says so
//! too. A disk at 60% is not warm the way a die at 60° is, which is why this
//! panel does not share the cpu ramp.
//!
//! **Reading a volume is a blocking call**, and on a network mount that has
//! gone to sleep it can block for a minute — so the reading is done on a
//! thread and polled in `tick`, as weather's is, even though it is local.
//! Every read costs about a tenth of a second on this `MacBook` for two
//! volumes, which would be a visible stall on the main thread anyway.
//!
//! **The panel's real work is `group`.** The platform lists *volumes*, and a
//! person thinks in *devices*: macOS reports `/` and `/System/Volumes/Data`
//! as two entries of one APFS container, with one pool of free space between
//! them, and listing both would show the same disk twice. Linux lists every
//! snap as a read-only squashfs loop that is full by construction. Volumes
//! that share a device are folded into one row named by the shortest mount
//! point, read-only volumes are dropped because nothing can fill them, and
//! what is left is what a person would ask about.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::chart::meter_line;
use crate::config::DiskConfig;
use crate::frame::{Binding, FRAME_HEIGHT};
use crate::panel::{Panel, RenderContext};

/// One mounted volume as the platform reports it, before grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Volume {
    pub mount: String,
    pub file_system: String,
    pub total: u64,
    pub available: u64,
    pub read_only: bool,
}

/// One row of the panel: a device, named by its friendliest mount point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Device {
    pub mount: String,
    pub total: u64,
    pub available: u64,
}

impl Device {
    fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    /// Whole percent used, saturating at 100 and reading 0 for no capacity.
    fn used_pct(&self) -> u16 {
        percent(self.used(), self.total)
    }
}

/// `part` as a whole percentage of `whole`, saturating at 100 and reading `0`
/// for a whole of zero.
fn percent(part: u64, whole: u64) -> u16 {
    if whole == 0 {
        return 0;
    }
    ((u128::from(part.min(whole)) * 100) / u128::from(whole)) as u16
}

/// Bytes the way a disk is sold and Finder states it: decimal units, one
/// decimal above a terabyte and below ten gigabytes, whole figures between.
pub(crate) fn human(bytes: u64) -> String {
    const TB: f64 = 1e12;
    const GB: f64 = 1e9;
    const MB: f64 = 1e6;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.1} TB", b / TB)
    } else if b >= 10.0 * GB {
        format!("{:.0} GB", b / GB)
    } else if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{} kB", bytes / 1000)
    }
}

/// File systems that are never a disk filling up: a snap's squashfs image is
/// full the day it is written, an optical disc cannot be written at all, and
/// the memory-backed ones are memory.
const NOT_A_DISK: &[&str] = &[
    "squashfs", "iso9660", "udf", "tmpfs", "devtmpfs", "ramfs", "autofs", "overlay",
];

/// Fold the platform's volumes into the devices a person would list.
///
/// Two volumes are one device when they report the same capacity and the
/// same free space to within a percent — an APFS container's volumes share
/// one pool and drift by megabytes between two reads. The device takes the
/// shortest mount point as its name, the smallest free figure as its free
/// space, and counts as writable if any of its volumes is. Read-only devices
/// are dropped while there is a writable one to show, because a volume
/// nothing can write to is a volume that never fills. Root first, then by
/// mount point, so the list holds still between reads.
pub(crate) fn group(volumes: Vec<Volume>) -> Vec<Device> {
    let mut devices: Vec<(Device, bool)> = Vec::new();
    for v in volumes {
        if v.total == 0 || NOT_A_DISK.contains(&v.file_system.to_ascii_lowercase().as_str()) {
            continue;
        }
        let tolerance = v.total / 100;
        match devices
            .iter_mut()
            .find(|(d, _)| d.total == v.total && d.available.abs_diff(v.available) <= tolerance)
        {
            Some((d, writable)) => {
                if shorter(&v.mount, &d.mount) {
                    d.mount = v.mount;
                }
                d.available = d.available.min(v.available);
                *writable |= !v.read_only;
            }
            None => devices.push((
                Device {
                    mount: v.mount,
                    total: v.total,
                    available: v.available,
                },
                !v.read_only,
            )),
        }
    }
    if devices.iter().any(|(_, writable)| *writable) {
        devices.retain(|(_, writable)| *writable);
    }
    let mut out: Vec<Device> = devices.into_iter().map(|(d, _)| d).collect();
    out.sort_by(|a, b| shorter_order(&a.mount, &b.mount));
    out
}

fn shorter(a: &str, b: &str) -> bool {
    shorter_order(a, b) == std::cmp::Ordering::Less
}

fn shorter_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.chars()
        .count()
        .cmp(&b.chars().count())
        .then_with(|| a.cmp(b))
}

/// Ask the platform for every mounted volume. Blocking; the thread's job.
fn read() -> Vec<Volume> {
    use sysinfo::{DiskRefreshKind, Disks};
    let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage());
    disks
        .list()
        .iter()
        .map(|d| Volume {
            mount: d.mount_point().to_string_lossy().into_owned(),
            file_system: d.file_system().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
            read_only: d.is_read_only(),
        })
        .collect()
}

/// The disk panel.
#[derive(Debug)]
pub struct DiskPanel {
    config: DiskConfig,
    /// The reader's latest list; `None` until the first read lands.
    shared: Arc<Mutex<Option<Vec<Device>>>>,
    /// Bumped by the reader each time it replaces the list.
    generation: Arc<AtomicU64>,
    /// The generation the last tick copied out.
    seen: u64,
    stop: Arc<AtomicBool>,
    /// What the panel draws from: copied out of `shared` once per change, so
    /// `render`, `counter` and `alert` never take the mutex.
    cache: Option<Vec<Device>>,
}

impl DiskPanel {
    /// Start the reader thread and return immediately.
    pub fn new(config: DiskConfig) -> Self {
        let shared: Arc<Mutex<Option<Vec<Device>>>> = Arc::new(Mutex::new(None));
        let generation = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let interval = Duration::from_secs(config.sample_secs.max(1));
        let (state, bump, halt) = (
            Arc::clone(&shared),
            Arc::clone(&generation),
            Arc::clone(&stop),
        );
        std::thread::Builder::new()
            .name("mirador-disk".into())
            .spawn(move || {
                loop {
                    let devices = group(read());
                    if let Ok(mut slot) = state.lock() {
                        *slot = Some(devices);
                    }
                    bump.fetch_add(1, Ordering::Release);
                    if matches!(
                        crate::poll::wait(interval, &halt, || false),
                        crate::poll::Wake::Stop
                    ) {
                        break;
                    }
                }
            })
            .expect("spawning the disk thread");
        Self {
            config,
            shared,
            generation,
            seen: 0,
            stop,
            cache: None,
        }
    }

    /// A panel holding `devices`, with no thread behind it, for tests.
    #[cfg(test)]
    pub(crate) fn with_devices(config: DiskConfig, devices: Vec<Device>) -> Self {
        Self {
            config,
            shared: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            seen: 0,
            stop: Arc::new(AtomicBool::new(false)),
            cache: Some(devices),
        }
    }

    /// An internal drive with room and an external one nearly full, for the
    /// sweep and the dump.
    #[cfg(test)]
    pub(crate) fn canned(config: DiskConfig) -> Self {
        Self::with_devices(
            config,
            vec![
                Device {
                    mount: "/".into(),
                    total: 1_995_165_736_960,
                    available: 1_637_809_513_970,
                },
                Device {
                    mount: "/Volumes/T7".into(),
                    total: 2_000_398_934_016,
                    available: 61_203_144_704,
                },
            ],
        )
    }

    /// Brass while there is room, amber past the warning line, red past the
    /// alert line. Thresholds rather than a ramp: a disk two-thirds full is
    /// not "warm", it is fine, and a ramp would say otherwise.
    fn colour(&self, pct: u16, theme: &crate::theme::Theme) -> Color {
        if self.config.alert_above_pct > 0 && pct >= self.config.alert_above_pct {
            theme.error
        } else if self.config.warn_above_pct > 0 && pct >= self.config.warn_above_pct {
            theme.warning
        } else {
            theme.accent
        }
    }

    fn devices(&self) -> &[Device] {
        self.cache.as_deref().unwrap_or(&[])
    }
}

impl Drop for DiskPanel {
    /// See `Drop for StocksPanel`: a panel can be dropped without `shutdown`,
    /// and the picker rebuilding the dashboard does exactly that.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Keys this panel responds to: none. It has nothing to set.
const BINDINGS: &[Binding] = &[];

/// Rows a device takes at each spacing: text and meter with a blank between
/// devices, text and meter, or text alone.
fn rows_per_device(height: u16, devices: u16) -> u16 {
    if devices == 0 {
        return 1;
    }
    if height >= devices * 3 - 1 {
        3
    } else if height >= devices * 2 {
        2
    } else {
        1
    }
}

impl Panel for DiskPanel {
    fn title(&self) -> String {
        "Disk".to_string()
    }

    fn counter(&self) -> Option<String> {
        // What is left on the system's own volume: the one figure a glance
        // at the frame is for. The rows inside say the rest.
        let first = self.devices().first()?;
        Some(format!("{} free", human(first.available)))
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn max_height(&self) -> Option<u16> {
        // Three rows a device with the last blank line unneeded, and the
        // frame. Past that every row is space under the last meter.
        let devices = u16::try_from(self.devices().len()).unwrap_or(u16::MAX);
        (devices > 0).then(|| FRAME_HEIGHT + devices * 3 - 1)
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn tick(&mut self) -> bool {
        let now = self.generation.load(Ordering::Acquire);
        if now == self.seen {
            return false;
        }
        self.seen = now;
        if let Ok(slot) = self.shared.lock() {
            self.cache.clone_from(&slot);
        }
        true
    }

    fn alert(&self) -> Option<crate::panel::Alert> {
        // A volume past the alert line gets worse with every file written,
        // and what it breaks — a save, a build, a backup — is not always
        // something that tells you why. The fullest one speaks.
        if self.config.alert_above_pct == 0 {
            return None;
        }
        let fullest = self
            .devices()
            .iter()
            .filter(|d| d.used_pct() >= self.config.alert_above_pct)
            .max_by_key(|d| d.used_pct())?;
        Some(crate::panel::Alert::soon(format!(
            "Disk {} at {}% — {} free",
            fullest.mount,
            fullest.used_pct(),
            human(fullest.available)
        )))
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }
        let Some(devices) = self.cache.as_deref() else {
            draw_message(frame, area, theme, "Reading disks", "");
            return;
        };
        if devices.is_empty() {
            draw_message(
                frame,
                area,
                theme,
                "No disks readable",
                "Nothing listed can fill up",
            );
            return;
        }

        let muted = Style::default().fg(theme.muted);
        let bottom = area.y + area.height;
        let count = u16::try_from(devices.len()).unwrap_or(u16::MAX);
        let per = rows_per_device(area.height, count);
        let mut y = area.y;

        for (index, device) in devices.iter().enumerate() {
            if y >= bottom {
                break;
            }
            let pct = device.used_pct();
            let colour = self.colour(pct, theme);

            // Mount, how full, the word, what is free, what it holds — dropped
            // from the end, so a narrow panel keeps the mount and the figure
            // that says whether to worry. Each gap travels with the part it
            // introduces.
            let parts = vec![
                vec![Span::styled(
                    device.mount.clone(),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                )],
                vec![
                    Span::styled(
                        format!("   {pct}"),
                        Style::default().fg(colour).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("%", muted),
                ],
                // The word is its own part, so a narrow panel keeps the
                // figure and loses the label rather than the other way round.
                vec![Span::styled(
                    format!(" {}", crate::glyphs::utility("used")),
                    Style::default()
                        .fg(theme.label)
                        .add_modifier(Modifier::BOLD),
                )],
                vec![Span::styled(
                    format!("   {} free", human(device.available)),
                    muted,
                )],
                vec![Span::styled(
                    format!("   of {}", human(device.total)),
                    muted,
                )],
            ];
            frame.render_widget(
                Paragraph::new(crate::grid::assemble(parts, area.width)),
                Rect::new(area.x, y, area.width, 1),
            );
            y += 1;

            if per >= 2 && y < bottom {
                frame.render_widget(
                    Paragraph::new(Line::from(meter_line(pct, area.width, colour, theme.track))),
                    Rect::new(area.x, y, area.width, 1),
                );
                y += 1;
            }
            if per == 3 && index + 1 < devices.len() {
                y += 1;
            }
        }
    }
}

/// Two centred muted lines, for a panel with nothing to draw.
fn draw_message(
    frame: &mut Frame,
    area: Rect,
    theme: &crate::theme::Theme,
    first: &str,
    second: &str,
) {
    let top = area.y + area.height.saturating_sub(2) / 2;
    for (i, text) in [first, second].into_iter().enumerate() {
        let y = top + u16::try_from(i).unwrap_or(0);
        if y < area.y + area.height && !text.is_empty() {
            let text = crate::grid::truncate(text, usize::from(area.width));
            frame.render_widget(
                Paragraph::new(Span::styled(text, Style::default().fg(theme.muted))).centered(),
                Rect::new(area.x, y, area.width, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(mount: &str, fs: &str, total: u64, available: u64, read_only: bool) -> Volume {
        Volume {
            mount: mount.into(),
            file_system: fs.into(),
            total,
            available,
            read_only,
        }
    }

    fn device(mount: &str, total: u64, available: u64) -> Device {
        Device {
            mount: mount.into(),
            total,
            available,
        }
    }

    fn panel(devices: Vec<Device>) -> DiskPanel {
        DiskPanel::with_devices(DiskConfig::default(), devices)
    }

    fn screen(panel: &mut DiskPanel, width: u16, height: u16) -> Vec<String> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                panel.render(
                    frame,
                    frame.area(),
                    RenderContext {
                        theme: &config.theme,
                        gradients: &gradients,
                        focused: true,
                        watch: &crate::watch::WatchLog::default(),
                    },
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn bytes_read_the_way_a_disk_is_sold() {
        assert_eq!(human(1_995_165_736_960), "2.0 TB");
        assert_eq!(human(1_637_809_513_970), "1.6 TB");
        assert_eq!(human(357_356_222_990), "357 GB");
        assert_eq!(human(61_203_144_704), "61 GB");
        assert_eq!(human(8_267_366_400), "8.3 GB");
        assert_eq!(human(512_000_000), "512 MB");
        assert_eq!(human(4_096), "4 kB");
        assert_eq!(human(0), "0 kB");
    }

    #[test]
    fn a_percentage_saturates_and_never_divides_by_zero() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(1, 4), 25);
        assert_eq!(percent(9, 4), 100);
        assert_eq!(percent(u64::MAX, u64::MAX), 100);
        assert_eq!(device("/", 0, 0).used_pct(), 0);
        assert_eq!(
            device("/", 100, 150).used(),
            0,
            "free past total reads as nothing used, not as underflow"
        );
    }

    /// The macOS pair — `/` sealed and read-only, `/System/Volumes/Data`
    /// writable, one container between them — is one device named `/`,
    /// with the smaller free figure, and writable because one of them is.
    #[test]
    fn the_volumes_of_one_container_are_one_device_named_by_its_root() {
        let devices = group(vec![
            volume("/", "apfs", 1_995_165_736_960, 1_637_885_797_874, true),
            volume(
                "/System/Volumes/Data",
                "apfs",
                1_995_165_736_960,
                1_637_809_513_970,
                false,
            ),
        ]);
        assert_eq!(
            devices,
            vec![device("/", 1_995_165_736_960, 1_637_809_513_970)]
        );
    }

    /// Two disks that happen to be the same size are two disks: what folds
    /// them is the shared free space, not the capacity.
    #[test]
    fn two_disks_of_one_size_with_different_fill_stay_separate() {
        let devices = group(vec![
            volume("/", "ext4", 1_000_000_000_000, 400_000_000_000, false),
            volume("/home", "ext4", 1_000_000_000_000, 900_000_000_000, false),
        ]);
        assert_eq!(devices.len(), 2, "{devices:?}");
        assert_eq!(devices[0].mount, "/", "root first");
        assert_eq!(devices[1].mount, "/home");
    }

    /// A snap's squashfs, a DVD and the memory-backed pseudo file systems
    /// are not disks that fill; a read-only volume is dropped while there is
    /// a writable one to show, and nothing with no capacity is listed.
    #[test]
    fn what_cannot_fill_up_is_not_listed() {
        let devices = group(vec![
            volume("/", "ext4", 500_000_000_000, 200_000_000_000, false),
            volume("/snap/core/1234", "squashfs", 60_000_000, 0, true),
            volume("/media/dvd", "iso9660", 4_700_000_000, 0, true),
            volume("/run", "tmpfs", 8_000_000_000, 7_900_000_000, false),
            volume("/mnt/ro", "ext4", 100_000_000_000, 10_000_000_000, true),
            volume("/mnt/empty", "ext4", 0, 0, false),
        ]);
        assert_eq!(devices, vec![device("/", 500_000_000_000, 200_000_000_000)]);
    }

    /// A machine with only read-only volumes still shows them rather than
    /// nothing; the rule drops read-only devices only in favour of a writable one.
    #[test]
    fn only_read_only_volumes_are_still_shown_rather_than_nothing() {
        let devices = group(vec![volume(
            "/",
            "ext4",
            500_000_000_000,
            200_000_000_000,
            true,
        )]);
        assert_eq!(devices.len(), 1);
    }

    /// Windows drive letters and a long external mount sort root first and
    /// then by name, so the list holds still between reads.
    #[test]
    fn devices_are_listed_root_first_then_by_mount() {
        let devices = group(vec![
            volume(
                "/Volumes/Time Machine",
                "apfs",
                4_000_000_000_000,
                1_000_000_000_000,
                false,
            ),
            volume(
                "/Volumes/T7",
                "exfat",
                2_000_000_000_000,
                61_000_000_000,
                false,
            ),
            volume("/", "apfs", 1_995_165_736_960, 1_637_809_513_970, false),
        ]);
        let mounts: Vec<&str> = devices.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/Volumes/T7", "/Volumes/Time Machine"]);
        let devices = group(vec![
            volume("D:\\", "NTFS", 2_000_000_000_000, 1_000_000_000_000, false),
            volume("C:\\", "NTFS", 1_000_000_000_000, 100_000_000_000, false),
        ]);
        let mounts: Vec<&str> = devices.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(mounts, ["C:\\", "D:\\"]);
    }

    /// Thresholds, not a ramp: brass to the warning line, amber to the alert
    /// line, red past it — and both lines inclusive, since "above 80%" is
    /// what the config says and 80 is not above 80.
    #[test]
    fn the_colour_changes_only_at_the_thresholds() {
        let theme = crate::theme::Theme::default();
        let p = panel(vec![]);
        assert_eq!(p.colour(0, &theme), theme.accent);
        assert_eq!(p.colour(79, &theme), theme.accent);
        assert_eq!(p.colour(80, &theme), theme.warning);
        assert_eq!(p.colour(94, &theme), theme.warning);
        assert_eq!(p.colour(95, &theme), theme.error);
        assert_eq!(p.colour(100, &theme), theme.error);
        let off = DiskPanel::with_devices(
            DiskConfig {
                warn_above_pct: 0,
                alert_above_pct: 0,
                ..DiskConfig::default()
            },
            vec![],
        );
        assert_eq!(off.colour(100, &theme), theme.accent, "zero disables");
    }

    /// The status bar names the fullest volume past the alert line, in the
    /// terms someone would use, and nothing below it.
    #[test]
    fn the_alert_names_the_fullest_volume_past_the_line() {
        let p = panel(vec![
            device("/", 1_000_000_000_000, 40_000_000_000),
            device("/Volumes/T7", 2_000_000_000_000, 20_000_000_000),
        ]);
        assert_eq!(
            p.alert().map(|a| a.text),
            Some("Disk /Volumes/T7 at 99% — 20 GB free".to_string())
        );
        assert!(
            panel(vec![device("/", 1_000_000_000_000, 60_000_000_000)])
                .alert()
                .is_none(),
            "94% is below the line"
        );
        assert!(
            DiskPanel::with_devices(
                DiskConfig {
                    alert_above_pct: 0,
                    ..DiskConfig::default()
                },
                vec![device("/", 100, 0)],
            )
            .alert()
            .is_none(),
            "zero disables"
        );
        assert!(panel(vec![]).alert().is_none());
    }

    #[test]
    fn the_border_carries_what_is_free_on_the_first_volume() {
        assert_eq!(
            DiskPanel::canned(DiskConfig::default())
                .counter()
                .as_deref(),
            Some("1.6 TB free")
        );
        assert_eq!(panel(vec![]).counter(), None);
        assert_eq!(
            DiskPanel::with_devices(DiskConfig::default(), vec![]).max_height(),
            None
        );
        assert_eq!(
            DiskPanel::canned(DiskConfig::default()).max_height(),
            Some(FRAME_HEIGHT + 5),
            "two devices: text, meter, blank, text, meter"
        );
    }

    /// Each device is a line of figures over a meter, a blank line between
    /// devices when there is room, and the figures are dropped whole from
    /// the end at any width — never a fragment.
    #[test]
    fn each_device_is_figures_over_a_meter_and_never_a_fragment() {
        let mut p = DiskPanel::canned(DiskConfig::default());
        let rows = screen(&mut p, 60, 5);
        assert_eq!(
            rows[0], "/   17% USED   1.6 TB free   of 2.0 TB",
            "{rows:?}"
        );
        assert!(
            rows[1].starts_with("■■■") && rows[1].chars().count() == 60,
            "{rows:?}"
        );
        assert_eq!(rows[2], "", "a blank line between devices: {rows:?}");
        assert_eq!(
            rows[3], "/Volumes/T7   96% USED   61 GB free   of 2.0 TB",
            "{rows:?}"
        );
        assert!(rows[4].starts_with('■'), "{rows:?}");

        let tight = screen(&mut p, 60, 4);
        assert_eq!(
            tight[2], "/Volumes/T7   96% USED   61 GB free   of 2.0 TB",
            "no blank when there is no room: {tight:?}"
        );
        let tighter = screen(&mut p, 60, 2);
        assert!(
            tighter[1].starts_with("/Volumes/T7"),
            "one row each: {tighter:?}"
        );

        for width in 1..=40u16 {
            for height in 1..=6u16 {
                for row in screen(&mut p, width, height) {
                    let t = row.trim();
                    for bad in ["fre", "of 2", "of 2.", "USE", "1.6 T", "61 G", " 1", " 9"] {
                        assert!(!t.ends_with(bad), "fragment {t:?} at {width}x{height}");
                    }
                    assert!(
                        !t.contains("17") || t.contains("17%"),
                        "a figure without its unit: {t:?} at {width}x{height}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_machine_with_nothing_readable_says_so_and_before_the_first_read_says_it_is_reading() {
        let mut none = panel(vec![]);
        let rows = screen(&mut none, 44, 3).join("\n");
        assert!(rows.contains("No disks readable"), "{rows}");
        assert!(!rows.contains('■'), "no meter for nothing: {rows}");
        let mut waiting = DiskPanel::with_devices(DiskConfig::default(), vec![]);
        waiting.cache = None;
        let rows = screen(&mut waiting, 44, 3).join("\n");
        assert!(rows.contains("Reading disks"), "{rows}");
    }

    /// The rows a device gets: three with a blank between, two, or one, and
    /// a device count of zero cannot divide anything.
    #[test]
    fn rows_per_device_gives_way_from_the_blank_line_down() {
        assert_eq!(rows_per_device(5, 2), 3);
        assert_eq!(rows_per_device(4, 2), 2);
        assert_eq!(rows_per_device(3, 2), 1);
        assert_eq!(
            rows_per_device(2, 1),
            3,
            "one device needs two rows for three-spacing"
        );
        assert_eq!(rows_per_device(1, 1), 1);
        assert_eq!(rows_per_device(0, 0), 1);
    }
}
