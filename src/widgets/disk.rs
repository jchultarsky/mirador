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
//!
//! Under each device, since 1.16.0, the network panel's face: a `↓ read
//! ↑ write` readout and two braille histories scaled to the device's own
//! peak — floored at a megabyte a second, so the trickle of background
//! writes every disk carries draws low instead of filling a graph that has
//! nothing bigger to show. `i` hides them for anyone who wants the panel as
//! it was.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::chart::{BrailleGraph, meter_line};
use crate::config::DiskConfig;
use crate::frame::{Binding, FRAME_HEIGHT};
use crate::keymap::{KeysConfig, Meta, PanelKeymap};
use crate::panel::{Panel, RenderContext};
use crate::widgets::network::format_rate;

/// One mounted volume as the platform reports it, before grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Volume {
    pub mount: String,
    pub file_system: String,
    pub total: u64,
    pub available: u64,
    pub read_only: bool,
    /// Bytes a second since the previous reading.
    pub read_rate: u64,
    pub write_rate: u64,
}

/// One row of the panel: a device, named by its friendliest mount point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Device {
    pub mount: String,
    pub total: u64,
    pub available: u64,
    /// Bytes a second since the previous reading.
    pub read_rate: u64,
    pub write_rate: u64,
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
                // The volumes of one device report one set of counters, so
                // the larger is the device's, not the sum.
                d.read_rate = d.read_rate.max(v.read_rate);
                d.write_rate = d.write_rate.max(v.write_rate);
                *writable |= !v.read_only;
            }
            None => devices.push((
                Device {
                    mount: v.mount,
                    total: v.total,
                    available: v.available,
                    read_rate: v.read_rate,
                    write_rate: v.write_rate,
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

/// The platform's disk list, kept between readings so the I/O counters
/// are deltas against the last one. Blocking; the thread's job.
struct Reader {
    disks: sysinfo::Disks,
    last: Instant,
    last_listing: Instant,
    /// How often to re-list volumes and re-read capacity, which is the
    /// expensive half; the I/O counters are read every time.
    relist_every: Duration,
}

impl Reader {
    fn new(relist_every: Duration) -> Self {
        use sysinfo::{DiskRefreshKind, Disks};
        let disks = Disks::new_with_refreshed_list_specifics(
            DiskRefreshKind::nothing().with_storage().with_io_usage(),
        );
        let now = Instant::now();
        Self {
            disks,
            last: now,
            last_listing: now,
            relist_every,
        }
    }

    fn read(&mut self) -> Vec<Volume> {
        use sysinfo::DiskRefreshKind;
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64().max(0.001);
        let relist = now.duration_since(self.last_listing) >= self.relist_every;
        let kind = if relist {
            DiskRefreshKind::nothing().with_storage().with_io_usage()
        } else {
            DiskRefreshKind::nothing().with_io_usage()
        };
        self.disks.refresh_specifics(relist, kind);
        if relist {
            self.last_listing = now;
        }
        self.last = now;
        let per_second = |bytes: u64| (bytes as f64 / elapsed) as u64;
        self.disks
            .list()
            .iter()
            .map(|d| Volume {
                mount: d.mount_point().to_string_lossy().into_owned(),
                file_system: d.file_system().to_string_lossy().into_owned(),
                total: d.total_space(),
                available: d.available_space(),
                read_only: d.is_read_only(),
                read_rate: per_second(d.usage().read_bytes),
                write_rate: per_second(d.usage().written_bytes),
            })
            .collect()
    }
}

/// The smallest ceiling a device's graphs are drawn against. Every disk
/// carries a trickle of background writes, and scaled to its own peak that
/// trickle would fill the graph; a megabyte a second is where I/O starts
/// to be worth a full-height mark.
const SCALE_FLOOR: u64 = 1 << 20;

/// The disk panel.
#[derive(Debug)]
pub struct DiskPanel {
    config: DiskConfig,
    /// `[disk.keys]` over the defaults.
    keys: PanelKeymap<DiskAction>,
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
    /// Read and write rate histories by mount, oldest first, fed one sample
    /// per reading by `tick`.
    histories: HashMap<String, (VecDeque<u64>, VecDeque<u64>)>,
    /// Cells a graph was last drawn into, so the histories can grow to fill it.
    graph_cells: usize,
}

impl DiskPanel {
    /// Start the reader thread and return immediately.
    pub fn new(config: DiskConfig) -> Self {
        let shared: Arc<Mutex<Option<Vec<Device>>>> = Arc::new(Mutex::new(None));
        let generation = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let interval = Duration::from_secs(config.io_sample_secs.max(1));
        let relist_every = Duration::from_secs(config.sample_secs.max(1));
        let (state, bump, halt) = (
            Arc::clone(&shared),
            Arc::clone(&generation),
            Arc::clone(&stop),
        );
        std::thread::Builder::new()
            .name("mirador-disk".into())
            .spawn(move || {
                let mut reader = Reader::new(relist_every);
                loop {
                    let devices = group(reader.read());
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
            keys: PanelKeymap::or_defaults("disk", ACTIONS, &config.keys),
            config,
            shared,
            generation,
            seen: 0,
            stop,
            cache: None,
            histories: HashMap::new(),
            graph_cells: 0,
        }
    }

    /// A panel holding `devices`, with no thread behind it, for tests.
    #[cfg(test)]
    pub(crate) fn with_devices(config: DiskConfig, devices: Vec<Device>) -> Self {
        let mut panel = Self {
            keys: PanelKeymap::or_defaults("disk", ACTIONS, &config.keys),
            config,
            shared: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            seen: 0,
            stop: Arc::new(AtomicBool::new(false)),
            cache: None,
            histories: HashMap::new(),
            graph_cells: 0,
        };
        panel.take(devices);
        panel
    }

    /// Adopt a fresh reading: replace the list, push one sample onto each
    /// device's histories, and forget any device that is gone.
    fn take(&mut self, devices: Vec<Device>) {
        let capacity = crate::samples::capacity(self.config.history, self.graph_cells);
        for device in &devices {
            let (reads, writes) = self.histories.entry(device.mount.clone()).or_default();
            crate::samples::push_bounded(reads, device.read_rate, capacity);
            crate::samples::push_bounded(writes, device.write_rate, capacity);
        }
        self.histories
            .retain(|mount, _| devices.iter().any(|d| &d.mount == mount));
        self.cache = Some(devices);
    }

    /// The ceiling both of a device's graphs are drawn against: its own
    /// peak in either direction, never below `SCALE_FLOOR`.
    fn scale(&self, mount: &str) -> u64 {
        self.histories
            .get(mount)
            .map_or(0, |(r, w)| {
                r.iter().chain(w.iter()).copied().max().unwrap_or(0)
            })
            .max(SCALE_FLOOR)
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
                    read_rate: 0,
                    write_rate: 327_680,
                },
                Device {
                    mount: "/Volumes/T7".into(),
                    total: 2_000_398_934_016,
                    available: 61_203_144_704,
                    read_rate: 52_428_800,
                    write_rate: 0,
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

/// What this panel's keys do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAction {
    Io,
}

/// This panel's keys, under `[disk.keys]`.
pub const ACTIONS: &[Meta<DiskAction>] = &[Meta {
    action: DiskAction::Io,
    name: "io",
    defaults: &[(KeyCode::Char('i'), KeyModifiers::NONE)],
    label: "i/o",
    primary: true,
    joins: false,
    about: "show or hide the I/O graphs",
}];

/// `[disk.keys]` laid over [`ACTIONS`], or why it cannot be.
pub fn keymap(keys: &KeysConfig) -> Result<PanelKeymap<DiskAction>, String> {
    PanelKeymap::new("disk", ACTIONS, keys)
}

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

    fn bindings(&self) -> &[Binding] {
        self.keys.bindings()
    }

    fn set_keys(&mut self, config: &crate::config::Config) {
        self.keys = PanelKeymap::or_defaults("disk", ACTIONS, &config.disk.keys);
    }

    fn max_height(&self) -> Option<u16> {
        // With the graphs on, more height is more history and the panel
        // scales like cpu. Without them: three rows a device with the last
        // blank line unneeded, and the frame — past that every row is space
        // under the last meter.
        if self.config.show_io {
            return None;
        }
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
        let fresh = self.shared.lock().ok().and_then(|slot| slot.clone());
        if let Some(devices) = fresh {
            self.take(devices);
        }
        true
    }

    fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> crate::panel::KeyOutcome {
        if self.keys.action(key) == Some(DiskAction::Io) {
            self.config.show_io = !self.config.show_io;
            return crate::panel::KeyOutcome::Consumed;
        }
        crate::panel::KeyOutcome::Ignored
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

        let count = u16::try_from(devices.len()).unwrap_or(u16::MAX);

        // With the graphs on, every device gets an equal share of the
        // height and the graphs take what its figures leave. Without them,
        // the blocks stack from the top with a blank line between when
        // there is room, and the rest of the panel is the row's to give away.
        let show_io = self.config.show_io && area.height >= 3;
        if show_io {
            let regions = Layout::vertical(vec![Constraint::Fill(1); devices.len()]).split(area);
            let devices = devices.to_vec();
            for (device, region) in devices.iter().zip(regions.iter()) {
                self.draw_device_with_io(frame, *region, device, ctx);
            }
            return;
        }

        let bottom = area.y + area.height;
        let per = rows_per_device(area.height, count);
        let mut y = area.y;

        for (index, device) in devices.iter().enumerate() {
            if y >= bottom {
                break;
            }
            let pct = device.used_pct();
            let colour = self.colour(pct, theme);
            frame.render_widget(
                Paragraph::new(crate::grid::assemble(
                    figures(device, colour, theme),
                    area.width,
                )),
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

/// The figures line as parts for `grid::assemble`: mount, how full, the
/// word, what is free, what it holds — dropped from the end, so a narrow
/// panel keeps the mount and the figure that says whether to worry. The
/// word is its own part, so the figure outlives its label rather than the
/// other way round. Each gap travels with the part it introduces.
fn figures(device: &Device, colour: Color, theme: &crate::theme::Theme) -> Vec<Vec<Span<'static>>> {
    let muted = Style::default().fg(theme.muted);
    let pct = device.used_pct();
    vec![
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
    ]
}

impl DiskPanel {
    /// One device with its graphs: figures, meter, the `↓ ↑` readout, and
    /// the rows left split between a read graph and a write graph — or one
    /// graph of both when only a row is left.
    fn draw_device_with_io(
        &mut self,
        frame: &mut Frame,
        region: Rect,
        device: &Device,
        ctx: RenderContext<'_>,
    ) {
        let theme = ctx.theme;
        if region.height == 0 {
            return;
        }
        let pct = device.used_pct();
        let colour = self.colour(pct, theme);
        let muted = Style::default().fg(theme.muted);
        let track = Style::default().fg(theme.track);
        let gradient = &ctx.gradients.cpu;

        let graph_rows = region.height.saturating_sub(3);
        let rows = Layout::vertical([
            Constraint::Length(1),                             // figures
            Constraint::Length(u16::from(region.height >= 2)), // meter
            Constraint::Length(u16::from(region.height >= 3)), // readout
            Constraint::Length(graph_rows.div_ceil(2)),        // reads
            Constraint::Length(graph_rows / 2),                // writes
        ])
        .split(region);

        frame.render_widget(
            Paragraph::new(crate::grid::assemble(
                figures(device, colour, theme),
                rows[0].width,
            )),
            rows[0],
        );
        if rows[1].height > 0 {
            frame.render_widget(
                Paragraph::new(Line::from(meter_line(
                    pct,
                    rows[1].width,
                    colour,
                    theme.track,
                ))),
                rows[1],
            );
        }
        if rows[2].height > 0 {
            // Two readings, each a whole part: an arrow left over nothing
            // reads as a rate of zero.
            let text = Style::default().fg(theme.text);
            frame.render_widget(
                Paragraph::new(crate::grid::assemble(
                    vec![
                        vec![
                            Span::styled("↓ ", muted),
                            Span::styled(format_rate(device.read_rate), text),
                        ],
                        vec![
                            Span::styled("   ↑ ", muted),
                            Span::styled(format_rate(device.write_rate), text),
                        ],
                    ],
                    rows[2].width,
                )),
                rows[2],
            );
        }

        self.graph_cells = usize::from(region.width);
        let scale = self.scale(&device.mount);
        let Some((reads, writes)) = self.histories.get(&device.mount) else {
            return;
        };
        if rows[4].height == 0 && rows[3].height > 0 {
            // One row: both directions in one graph, since a reader with a
            // row to spare wants to know whether the disk is busy at all.
            let both: Vec<u64> = reads
                .iter()
                .zip(writes.iter())
                .map(|(r, w)| r.saturating_add(*w))
                .collect();
            BrailleGraph::new(&both, scale.saturating_mul(2), gradient)
                .track_style(track)
                .render(rows[3], frame.buffer_mut());
            return;
        }
        for (data, rect) in [(reads, rows[3]), (writes, rows[4])] {
            if rect.height > 0 {
                let data: Vec<u64> = data.iter().copied().collect();
                BrailleGraph::new(&data, scale, gradient)
                    .track_style(track)
                    .render(rect, frame.buffer_mut());
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
            read_rate: 0,
            write_rate: 0,
        }
    }

    fn device(mount: &str, total: u64, available: u64) -> Device {
        Device {
            mount: mount.into(),
            total,
            available,
            read_rate: 0,
            write_rate: 0,
        }
    }

    /// The panel without its graphs, which is the face the older tests
    /// describe row by row.
    fn quiet() -> DiskConfig {
        DiskConfig {
            show_io: false,
            ..DiskConfig::default()
        }
    }

    fn panel(devices: Vec<Device>) -> DiskPanel {
        DiskPanel::with_devices(quiet(), devices)
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
        assert_eq!(panel(vec![]).max_height(), None);
        assert_eq!(
            DiskPanel::canned(quiet()).max_height(),
            Some(FRAME_HEIGHT + 5),
            "two devices: text, meter, blank, text, meter"
        );
        assert_eq!(
            DiskPanel::canned(DiskConfig::default()).max_height(),
            None,
            "with the graphs on, more height is more history"
        );
    }

    /// Each device is a line of figures over a meter, a blank line between
    /// devices when there is room, and the figures are dropped whole from
    /// the end at any width — never a fragment.
    #[test]
    fn each_device_is_figures_over_a_meter_and_never_a_fragment() {
        let mut p = DiskPanel::canned(quiet());
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
        let mut waiting = panel(vec![]);
        waiting.cache = None;
        let rows = screen(&mut waiting, 44, 3).join("\n");
        assert!(rows.contains("Reading disks"), "{rows}");
    }

    /// With the graphs on, each device is figures, meter, a `↓ ↑` readout
    /// and two graphs sharing the rows left — the network panel's face, per
    /// device — and no fragment of any figure at any size.
    #[test]
    fn with_io_each_device_is_figures_meter_readout_and_two_graphs() {
        let mut p = DiskPanel::canned(DiskConfig::default());
        let rows = screen(&mut p, 60, 10);
        assert_eq!(
            rows[0], "/   17% USED   1.6 TB free   of 2.0 TB",
            "{rows:?}"
        );
        assert!(rows[1].starts_with('■'), "{rows:?}");
        assert_eq!(rows[2], "↓ 0 B/s   ↑ 320.0 KB/s", "{rows:?}");
        assert!(
            rows[3].starts_with('⣀') && rows[4].starts_with('⣀'),
            "two graphs: {rows:?}"
        );
        assert!(rows[5].starts_with("/Volumes/T7"), "{rows:?}");
        assert_eq!(rows[7], "↓ 50.0 MB/s   ↑ 0 B/s", "{rows:?}");

        for width in 1..=40u16 {
            for height in 1..=12u16 {
                for row in screen(&mut p, width, height) {
                    let t = row.trim();
                    for bad in ["B/", "KB", "MB", "↑", "↓", "fre", "USE"] {
                        assert!(!t.ends_with(bad), "fragment {t:?} at {width}x{height}");
                    }
                }
            }
        }
    }

    /// A tick adopts the reading and pushes one sample per device onto its
    /// histories; a device that vanishes takes its history with it.
    #[test]
    fn each_reading_feeds_the_histories_and_a_gone_device_is_forgotten() {
        let mut p = DiskPanel::canned(DiskConfig::default());
        assert_eq!(p.histories["/"].1.len(), 1);
        let mut root = device("/", 100, 50);
        root.read_rate = 7;
        p.take(vec![root]);
        assert_eq!(
            p.histories["/"].0.iter().copied().collect::<Vec<_>>(),
            [0, 7]
        );
        assert!(
            !p.histories.contains_key("/Volumes/T7"),
            "gone: {:?}",
            p.histories.keys()
        );
    }

    /// The graphs' ceiling is the device's own peak in either direction,
    /// never below a megabyte a second, so a trickle of background writes
    /// does not fill the graph.
    #[test]
    fn the_scale_is_the_devices_peak_floored_at_a_megabyte() {
        let p = DiskPanel::canned(DiskConfig::default());
        assert_eq!(p.scale("/"), SCALE_FLOOR, "320 KB/s is under the floor");
        assert_eq!(p.scale("/Volumes/T7"), 52_428_800);
        assert_eq!(p.scale("/nowhere"), SCALE_FLOOR);
    }

    #[test]
    fn i_toggles_the_graphs_and_is_documented() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut p = DiskPanel::canned(DiskConfig::default());
        assert!(p.config.show_io);
        let outcome = p.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert_eq!(outcome, crate::panel::KeyOutcome::Consumed);
        assert!(!p.config.show_io);
        assert!(p.bindings().iter().any(|b| b.key == "i"));
        assert_eq!(
            p.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            crate::panel::KeyOutcome::Ignored,
            "every other key stays global"
        );
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
