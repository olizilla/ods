use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Formats a byte size using base-1024 units without a space before the unit.
/// Under 1 KiB -> "512B"; under 1 MiB -> "640KB"; under 1 GiB -> "36MB"; above -> "2.7GB".
pub fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;

    if bytes < KIB {
        format!("{}B", bytes)
    } else if bytes < MIB {
        let kb = (bytes as f64 / KIB as f64).round() as u64;
        format!("{}KB", kb)
    } else if bytes < GIB {
        let mb = (bytes as f64 / MIB as f64).round() as u64;
        format!("{}MB", mb)
    } else {
        let gb = bytes as f64 / GIB as f64;
        format!("{:.1}GB", gb)
    }
}

/// Formats a transfer rate (bytes per second) without a space before the unit.
/// Example: "9.1MB/s", "512.0KB/s".
pub fn format_rate(bytes_per_sec: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;

    if bytes_per_sec < MIB {
        format!("{:.1}KB/s", bytes_per_sec / KIB)
    } else {
        format!("{:.1}MB/s", bytes_per_sec / MIB)
    }
}

/// Formats a duration into compact human-readable form.
/// Examples: "16s", "3m12s", "1h04m".
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    if total_secs < 60 {
        format!("{}s", total_secs)
    } else if total_secs < 3600 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{}m{:02}s", mins, secs)
    } else {
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        format!("{}h{:02}m", hours, mins)
    }
}

/// Formats an elapsed duration: one decimal place under a minute (e.g. "3.1s", "59.9s"),
/// then `format_duration`'s form ("1m00s", "3m12s").
pub fn format_elapsed(duration: Duration) -> String {
    let secs_f64 = duration.as_secs_f64();
    if secs_f64 < 60.0 {
        let s = format!("{:.1}s", secs_f64);
        if s == "60.0s" {
            "1m00s".to_string()
        } else {
            s
        }
    } else {
        format_duration(duration)
    }
}

#[derive(Debug, Clone)]
pub enum ReleaseBlockState {
    Downloading {
        bytes_done: u64,
        rate: Option<f64>,
        eta: Option<Duration>,
    },
    Done {
        elapsed: Duration,
    },
    Cached,
}

#[derive(Debug, Clone, Copy)]
pub struct ReleaseBlockLink<'a> {
    pub target: &'a str,
    pub unchanged: bool,
}

/// Parameters for rendering a release report block.
#[derive(Debug, Clone)]
pub struct ReleaseBlockParams<'a> {
    pub date: &'a str,
    pub archive_size: u64,
    pub file_count: usize,
    pub state: &'a ReleaseBlockState,
    pub verified: &'a str,
    pub linked: Option<ReleaseBlockLink<'a>>,
    pub hash: Option<&'a str>,
    pub color: bool,
}

/// Renders a release report as a multi-line block (2 to 4 lines).
/// Given release block parameters, returns the block's lines.
pub fn render_release_block(params: &ReleaseBlockParams) -> Vec<String> {
    let mut lines = Vec::with_capacity(4);

    let (filled, empty, size_bytes, tail) = match params.state {
        ReleaseBlockState::Downloading { bytes_done, rate, eta } => {
            let done = (*bytes_done).min(params.archive_size);
            let filled = if params.archive_size > 0 {
                ((done as f64 / params.archive_size as f64) * 20.0).floor() as usize
            } else {
                0
            }.min(20);
            let empty = 20 - filled;
            let mut parts = Vec::new();
            if let Some(r) = rate {
                parts.push(format_rate(*r));
            }
            if let Some(e) = eta {
                parts.push(format!("eta {}", format_duration(*e)));
            }
            (filled, empty, *bytes_done, parts.join("  "))
        }
        ReleaseBlockState::Done { elapsed } => {
            (20, 0, params.archive_size, format!("in {}", format_elapsed(*elapsed)))
        }
        ReleaseBlockState::Cached => {
            (20, 0, params.archive_size, "cached".to_string())
        }
    };

    let bar = if params.color {
        let mut b = String::new();
        if filled > 0 {
            b.push_str(crate::ansi::ANSI_CYAN);
            b.push_str(&"█".repeat(filled));
            b.push_str(crate::ansi::ANSI_RESET);
        }
        if empty > 0 {
            b.push_str(crate::ansi::ANSI_MUTED);
            b.push_str(&"░".repeat(empty));
            b.push_str(crate::ansi::ANSI_RESET);
        }
        b
    } else {
        format!("{}{}", "█".repeat(filled), "░".repeat(empty))
    };

    let files_str = if params.file_count == 1 {
        "1 file".to_string()
    } else {
        format!("{} files", params.file_count)
    };
    let size_str = format_size(size_bytes);

    let stats_text = format!("  {:>4}   {}  {}", size_str, files_str, tail);
    let stats = if params.color {
        format!("{}{}{}", crate::ansi::ANSI_MUTED, stats_text, crate::ansi::ANSI_RESET)
    } else {
        stats_text
    };

    lines.push(format!("  {:<14}{}{}", params.date, bar, stats));

    // Row 2: verified
    let verified_val = match params.state {
        ReleaseBlockState::Downloading { .. } => {
            if params.color {
                format!("{}-{}", crate::ansi::ANSI_MUTED, crate::ansi::ANSI_RESET)
            } else {
                "-".to_string()
            }
        }
        _ => params.verified.to_string(),
    };
    lines.push(format!("  {:<14}{}", "verified", verified_val));

    // Row 3: linked (if present)
    if let Some(link) = params.linked {
        let linked_val = match params.state {
            ReleaseBlockState::Downloading { .. } => {
                if params.color {
                    format!("{}-{}", crate::ansi::ANSI_MUTED, crate::ansi::ANSI_RESET)
                } else {
                    "-".to_string()
                }
            }
            _ => {
                let arrow = if params.color {
                    format!("{}→{}", crate::ansi::ANSI_CYAN, crate::ansi::ANSI_RESET)
                } else {
                    "→".to_string()
                };
                let unchanged_str = if link.unchanged {
                    if params.color {
                        format!("{} (unchanged){}", crate::ansi::ANSI_MUTED, crate::ansi::ANSI_RESET)
                    } else {
                        " (unchanged)".to_string()
                    }
                } else {
                    String::new()
                };
                format!("current {} {}{}", arrow, link.target, unchanged_str)
            }
        };
        lines.push(format!("  {:<14}{}", "linked", linked_val));
    }

    // Row 4: sha256 (if verbose / hash present)
    if let Some(h) = params.hash {
        lines.push(format!("  {:<14}{}", "sha256", h));
    }

    lines
}

#[derive(Debug, Clone, Copy)]
pub struct ProgressCaps {
    pub is_tty: bool,
    pub no_color: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub width: usize,
}

impl ProgressCaps {
    pub fn detect(quiet: bool, verbose: bool, no_progress: bool) -> Self {
        let is_terminal = std::io::stderr().is_terminal();
        let term = std::env::var("TERM").unwrap_or_default();
        let is_dumb = term == "dumb";
        let is_tty = is_terminal && !no_progress && !is_dumb;
        let no_color = std::env::var("NO_COLOR").is_ok();

        let width = if is_tty {
            crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80)
        } else {
            80
        };

        Self {
            is_tty,
            no_color,
            quiet,
            verbose,
            width,
        }
    }
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_char(start: Instant) -> &'static str {
    let idx = (start.elapsed().as_millis() / 80) as usize % SPINNER_FRAMES.len();
    SPINNER_FRAMES[idx]
}

#[derive(Clone)]
struct InFlightRow {
    text: String,
    last_updated: Instant,
}

struct ProgressInner {
    writer: Box<dyn Write + Send>,
    caps: ProgressCaps,
    live_step: Option<String>,
    live_block: Option<Vec<String>>,
    in_flight: HashMap<String, InFlightRow>,
    in_flight_order: Vec<String>,
    batch_bar: Option<String>,
    start_time: Instant,
    last_rendered_lines_count: usize,
    last_rendered_text: Vec<String>,
    last_render_time: Instant,
    last_plain_heartbeat: Instant,
}

impl ProgressInner {
    fn erase_live_unlocked(&mut self) {
        if !self.caps.is_tty || self.last_rendered_lines_count == 0 {
            return;
        }
        let mut buf = String::with_capacity(128);
        if self.last_rendered_lines_count > 1 {
            buf.push_str(&format!("\x1b[{}A\r", self.last_rendered_lines_count - 1));
        } else {
            buf.push('\r');
        }
        for i in 0..self.last_rendered_lines_count {
            buf.push_str("\x1b[2K");
            if i + 1 < self.last_rendered_lines_count {
                buf.push('\n');
            }
        }
        if self.last_rendered_lines_count > 1 {
            buf.push_str(&format!("\x1b[{}A\r", self.last_rendered_lines_count - 1));
        } else {
            buf.push('\r');
        }
        let _ = write!(self.writer, "{}", buf);
        let _ = self.writer.flush();
        self.last_rendered_lines_count = 0;
        self.last_rendered_text.clear();
    }

    fn clear_live_unlocked(&mut self) {
        self.erase_live_unlocked();
        self.live_step = None;
        self.live_block = None;
        self.in_flight.clear();
        self.in_flight_order.clear();
        self.batch_bar = None;
    }

    fn tick_unlocked(&mut self) {
        let now = Instant::now();
        if self.caps.quiet {
            return;
        }

        if self.caps.is_tty {
            let has_live = self.live_block.is_some() || self.live_step.is_some() || !self.in_flight.is_empty() || self.batch_bar.is_some();
            if has_live {
                self.repaint_live_unlocked(false);
            }
        } else {
            // Plain mode rate-limited heartbeat: emit 1 line every 30s while batch is running
            if let Some(ref bar) = self.batch_bar {
                if !self.in_flight.is_empty() && now.duration_since(self.last_plain_heartbeat) >= Duration::from_secs(30) {
                    self.last_plain_heartbeat = now;
                    let _ = writeln!(self.writer, "… {}", bar);
                    let _ = self.writer.flush();
                }
            }
        }
    }

    fn repaint_live_unlocked(&mut self, force: bool) {
        if !self.caps.is_tty || self.caps.quiet {
            return;
        }

        let now = Instant::now();
        if !force && now.duration_since(self.last_render_time) < Duration::from_millis(33) {
            return;
        }

        let mut lines_to_draw = Vec::new();

        if let Some(ref block) = self.live_block {
            lines_to_draw.extend(block.clone());
        } else {
            let spin = spinner_char(self.start_time);

            if let Some(ref step) = self.live_step {
                lines_to_draw.push(format!("{} {}", spin, step));
            }

            for id in &self.in_flight_order {
                if let Some(row) = self.in_flight.get(id) {
                    let elapsed = now.duration_since(row.last_updated);
                    if elapsed > Duration::from_secs(10) {
                        lines_to_draw.push(format!("  {} (stalled {}s)", row.text, elapsed.as_secs()));
                    } else {
                        lines_to_draw.push(format!("  {}", row.text));
                    }
                }
            }

            if let Some(ref bar) = self.batch_bar {
                lines_to_draw.push(format!("{} {}", spin, bar));
            }
        }

        if lines_to_draw.is_empty() {
            self.erase_live_unlocked();
            return;
        }

        let mut truncated_lines = Vec::new();
        for line in &lines_to_draw {
            let mut t = line.clone();
            if t.chars().count() > self.caps.width && self.caps.width > 3 {
                t = t.chars().take(self.caps.width - 3).collect::<String>() + "…";
            }
            truncated_lines.push(t);
        }

        if !force && truncated_lines == self.last_rendered_text && now.duration_since(self.last_render_time) < Duration::from_millis(80) {
            return;
        }

        // Build the entire ANSI frame in memory to avoid screen flicker
        let mut buf = String::with_capacity(512);

        if self.last_rendered_lines_count > 1 {
            buf.push_str(&format!("\x1b[{}A\r", self.last_rendered_lines_count - 1));
        } else if self.last_rendered_lines_count == 1 {
            buf.push('\r');
        }

        let new_count = truncated_lines.len();
        for (i, line) in truncated_lines.iter().enumerate() {
            buf.push_str("\x1b[2K");
            buf.push_str(line);
            if i + 1 < new_count {
                buf.push('\n');
            }
        }

        if self.last_rendered_lines_count > new_count {
            let extra = self.last_rendered_lines_count - new_count;
            for _ in 0..extra {
                buf.push_str("\n\x1b[2K");
            }
            buf.push_str(&format!("\x1b[{}A", extra));
        }

        let _ = write!(self.writer, "{}", buf);
        let _ = self.writer.flush();

        self.last_rendered_lines_count = new_count;
        self.last_rendered_text = truncated_lines;
        self.last_render_time = now;
    }
}

#[derive(Clone)]
pub struct Progress {
    inner: Arc<Mutex<ProgressInner>>,
    ticker_stop: Arc<AtomicBool>,
}

impl Progress {
    pub fn new(caps: ProgressCaps, writer: Box<dyn Write + Send>) -> Self {
        let inner = Arc::new(Mutex::new(ProgressInner {
            writer,
            caps,
            live_step: None,
            live_block: None,
            in_flight: HashMap::new(),
            in_flight_order: Vec::new(),
            batch_bar: None,
            start_time: Instant::now(),
            last_rendered_lines_count: 0,
            last_rendered_text: Vec::new(),
            last_render_time: Instant::now() - Duration::from_secs(1),
            last_plain_heartbeat: Instant::now(),
        }));

        let ticker_stop = Arc::new(AtomicBool::new(false));

        let inner_weak = Arc::downgrade(&inner);
        let stop_flag = ticker_stop.clone();

        if !caps.quiet {
            std::thread::Builder::new()
                .name("ods-render-tick".to_string())
                .spawn(move || {
                    while !stop_flag.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                        if let Some(arc_inner) = inner_weak.upgrade() {
                            let mut guard = arc_inner.lock().unwrap();
                            guard.tick_unlocked();
                        } else {
                            break;
                        }
                    }
                })
                .ok();
        }

        Self { inner, ticker_stop }
    }

    pub fn stderr(caps: ProgressCaps) -> Self {
        Self::new(caps, Box::new(std::io::stderr()))
    }

    pub fn caps(&self) -> ProgressCaps {
        self.inner.lock().unwrap().caps
    }

    /// Transient progress step, overwritten by the next call in TTY mode.
    pub fn step(&self, text: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.caps.verbose {
            inner.erase_live_unlocked();
            let _ = writeln!(inner.writer, "* {}", text);
            let _ = inner.writer.flush();
            return;
        }
        if !inner.caps.is_tty || inner.caps.quiet {
            return;
        }
        inner.live_step = Some(text.to_string());
        inner.repaint_live_unlocked(true);
    }

    /// Live progress bar with percentage, bytes, rate, and ETA.
    pub fn bar(&self, label: &str, done: u64, total: u64, rate: Option<f64>, eta: Option<Duration>) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.caps.is_tty || inner.caps.quiet {
            return;
        }

        let pct = if total > 0 {
            ((done as f64 / total as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        const BAR_WIDTH: usize = 15;
        let filled = ((pct / 100.0) * BAR_WIDTH as f64).round() as usize;
        let empty = BAR_WIDTH.saturating_sub(filled);
        let bar_str = format!("{}{}", "█".repeat(filled), "░".repeat(empty));

        let mut parts = vec![
            label.to_string(),
            bar_str,
            if total <= 1000 && rate.is_none() {
                format!("{}/{}", done, total)
            } else {
                format!("{}/{}", format_size(done), format_size(total))
            },
        ];

        if let Some(r) = rate {
            parts.push(format_rate(r));
        }

        if let Some(e) = eta {
            parts.push(format!("eta {}", format_duration(e)));
        }

        inner.live_step = Some(parts.join("  "));
        inner.repaint_live_unlocked(false);
    }

    /// Batch progress bar with item count (e.g. "5 of 12", bytes, rate, eta).
    pub fn batch_bar(
        &self,
        current_item: usize,
        total_items: usize,
        done_bytes: u64,
        total_bytes: u64,
        rate: Option<f64>,
        eta: Option<Duration>,
    ) {
        let mut inner = self.inner.lock().unwrap();

        let pct = if total_bytes > 0 {
            ((done_bytes as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        const BAR_WIDTH: usize = 15;
        let filled = ((pct / 100.0) * BAR_WIDTH as f64).round() as usize;
        let empty = BAR_WIDTH.saturating_sub(filled);
        let bar_str = format!("{}{}", "█".repeat(filled), "░".repeat(empty));

        let mut parts = vec![
            format!("{} of {}", current_item, total_items),
            bar_str,
            format!("{}/{}", format_size(done_bytes), format_size(total_bytes)),
        ];

        if let Some(r) = rate {
            parts.push(format_rate(r));
        }

        if let Some(e) = eta {
            parts.push(format!("eta {}", format_duration(e)));
        }

        inner.batch_bar = Some(parts.join("  "));
        if inner.caps.is_tty && !inner.caps.quiet {
            inner.repaint_live_unlocked(false);
        }
    }

    /// Sets or updates an active in-flight task line (shown without tick prefix).
    pub fn set_in_flight(&self, id: &str, text: &str) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.in_flight.contains_key(id) {
            inner.in_flight_order.push(id.to_string());
        }
        inner.in_flight.insert(id.to_string(), InFlightRow {
            text: text.to_string(),
            last_updated: Instant::now(),
        });
        if inner.caps.is_tty && !inner.caps.quiet {
            inner.repaint_live_unlocked(true);
        }
    }

    /// Removes an in-flight task line when it finishes or fails.
    pub fn remove_in_flight(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.in_flight.remove(id);
        inner.in_flight_order.retain(|k| k != id);
        if inner.caps.is_tty && !inner.caps.quiet {
            inner.repaint_live_unlocked(true);
        }
    }

    /// Permanent line that scrolls above the live line.
    pub fn settle(&self, line: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.caps.quiet && !line.starts_with('✖') {
            return;
        }
        inner.erase_live_unlocked();
        let _ = writeln!(inner.writer, "{}", line);
        let _ = inner.writer.flush();
        inner.repaint_live_unlocked(true);
    }

    /// Indented continuation under the settled line (preserved even in quiet mode).
    pub fn settle_detail(&self, line: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.erase_live_unlocked();
        let _ = writeln!(inner.writer, "  {}", line);
        let _ = inner.writer.flush();
        inner.repaint_live_unlocked(true);
    }

    /// Final batch summary line (preserved even in quiet mode).
    pub fn settle_summary(&self, line: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.erase_live_unlocked();
        let _ = writeln!(inner.writer, "{}", line);
        let _ = inner.writer.flush();
        inner.repaint_live_unlocked(true);
    }

    /// Error block: clears live line and prints headline + details.
    pub fn error(&self, headline: &str, details: &[&str]) {
        let mut inner = self.inner.lock().unwrap();
        inner.live_block = None;
        inner.erase_live_unlocked();
        let _ = writeln!(inner.writer, "✖ {}", headline);
        for detail in details {
            let _ = writeln!(inner.writer, "  {}", detail);
        }
        let _ = inner.writer.flush();
        inner.repaint_live_unlocked(true);
    }

    /// Finish: clears live region and writes the summary line.
    pub fn finish(&self, summary: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.clear_live_unlocked();
        if !summary.is_empty() {
            let _ = writeln!(inner.writer, "{}", summary);
            let _ = inner.writer.flush();
        }
    }

    /// Clears any live transient line without writing further output.
    pub fn clear_live(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.clear_live_unlocked();
    }

    /// Repaints a multi-row live block in TTY mode.
    pub fn update_live_block(&self, lines: Vec<String>) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.caps.is_tty || inner.caps.quiet {
            return;
        }
        inner.live_block = Some(lines);
        inner.repaint_live_unlocked(false);
    }

    /// Emits a completed release block: clears live lines and writes block lines to stderr.
    /// Preserved even in quiet mode.
    pub fn finish_block(&self, lines: &[String]) {
        let mut inner = self.inner.lock().unwrap();
        inner.clear_live_unlocked();
        for line in lines {
            let _ = writeln!(inner.writer, "{}", line);
        }
        let _ = inner.writer.flush();
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if Arc::strong_count(&self.ticker_stop) <= 2 {
            self.ticker_stop.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(1024), "1KB");
        assert_eq!(format_size(3624), "4KB");
        assert_eq!(format_size(500 * 1024), "500KB");
        assert_eq!(format_size(36 * 1024 * 1024), "36MB");
        assert_eq!(format_size(246 * 1024 * 1024), "246MB");
        assert_eq!(format_size((2.7 * 1024.0 * 1024.0 * 1024.0) as u64), "2.7GB");
    }

    #[test]
    fn test_format_rate() {
        assert_eq!(format_rate(512.0 * 1024.0), "512.0KB/s");
        assert_eq!(format_rate(9.1 * 1024.0 * 1024.0), "9.1MB/s");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(16)), "16s");
        assert_eq!(format_duration(Duration::from_secs(192)), "3m12s");
        assert_eq!(format_duration(Duration::from_secs(3840)), "1h04m");
    }

    #[test]
    fn test_plain_mode_contains_no_control_chars() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        struct BufferWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufferWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let caps = ProgressCaps {
            is_tty: false,
            no_color: false,
            quiet: false,
            verbose: false,
            width: 80,
        };

        let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));
        progress.step("Querying TRUD...");
        progress.bar("Downloading", 50, 100, Some(1024.0 * 1024.0), Some(Duration::from_secs(5)));
        progress.settle("✓ 2026-07-31  36MB  downloaded");
        progress.finish("Done!");

        let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(!output.contains('\r'), "Plain mode must not contain \\r");
        assert!(!output.contains("\x1b"), "Plain mode must not contain escape codes");
        assert_eq!(
            output,
            "✓ 2026-07-31  36MB  downloaded\nDone!\n"
        );
    }

    #[test]
    fn test_concurrent_settle_calls_thread_safe() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        struct BufferWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufferWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let caps = ProgressCaps {
            is_tty: false,
            no_color: false,
            quiet: false,
            verbose: false,
            width: 80,
        };

        let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let p = progress.clone();
                thread::spawn(move || {
                    for j in 0..10 {
                        p.settle(&format!("✓ Worker {} Release {}", i, j));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 80);
        for line in lines {
            assert!(line.starts_with("✓ Worker "));
        }
    }

    #[test]
    fn test_bar_byte_vs_item_formatting() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        struct BufferWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufferWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let caps = ProgressCaps {
            is_tty: true,
            no_color: true,
            quiet: false,
            verbose: false,
            width: 80,
        };

        let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));
        // Large byte download
        progress.bar("2021-07-30   downloading", 12543416, 30133679, Some(5.5 * 1024.0 * 1024.0), Some(Duration::from_secs(3)));
        let out = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(out.contains("12MB/29MB") || out.contains("12MB/30MB") || out.contains("11MB/28MB") || out.contains("MB/"), "Must format bytes as MB: {}", out);
        assert!(!out.contains("12543416/30133679"), "Must not format raw byte numbers: {}", out);
    }

    #[test]
    fn test_verbose_mode_uses_neutral_marker() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        struct BufferWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufferWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let caps = ProgressCaps {
            is_tty: false,
            no_color: true,
            quiet: false,
            verbose: true,
            width: 80,
        };

        let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));
        progress.step("reading publication metadata…");

        let out = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(out.starts_with("* reading publication metadata…"), "Verbose step must use * marker: {}", out);
    }

    #[test]
    fn test_quiet_mode_preserves_summary_and_errors() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        struct BufferWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufferWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let caps = ProgressCaps {
            is_tty: false,
            no_color: true,
            quiet: true,
            verbose: false,
            width: 80,
        };

        let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));
        progress.step("Ignored step");
        progress.settle("✓ 2026-07-31  36MB  downloaded"); // suppressed
        progress.settle_summary("✓ 1 downloaded · 0 cached · 0 failed · 36MB total  in 1s"); // preserved
        progress.settle_detail("current → releases/2026-07-31"); // preserved
        progress.error("Some failure", &["details"]); // preserved

        let out = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(!out.contains("Ignored step"));
        assert!(!out.contains("2026-07-31  36MB  downloaded"));
        assert!(out.contains("✓ 1 downloaded · 0 cached · 0 failed · 36MB total  in 1s"));
        assert!(out.contains("current → releases/2026-07-31"));
        assert!(out.contains("✖ Some failure"));
    }

    #[test]
    fn test_plain_mode_heartbeat_at_30s() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        struct BufferWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufferWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let caps = ProgressCaps {
            is_tty: false,
            no_color: true,
            quiet: false,
            verbose: false,
            width: 80,
        };

        let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));
        progress.set_in_flight("2026-07-31", "2026-07-31  36MB  downloading…");
        progress.batch_bar(3, 12, 61 * 1024 * 1024, 246 * 1024 * 1024, Some(9.1 * 1024.0 * 1024.0), Some(Duration::from_secs(20)));

        // Simulate 31 seconds passing
        {
            let mut guard = progress.inner.lock().unwrap();
            guard.last_plain_heartbeat = Instant::now() - Duration::from_secs(31);
            guard.tick_unlocked();
        }

        let out = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(out.contains("… 3 of 12"), "Plain mode heartbeat must write '…' line: {}", out);
        assert!(out.contains("61MB/246MB") || out.contains("61MB"));
    }

    #[test]
    fn test_format_elapsed() {
        assert_eq!(format_elapsed(Duration::from_millis(3100)), "3.1s");
        assert_eq!(format_elapsed(Duration::from_millis(59900)), "59.9s");
        assert_eq!(format_elapsed(Duration::from_secs(60)), "1m00s");
        assert_eq!(format_elapsed(Duration::from_secs(192)), "3m12s");
    }

    #[test]
    fn test_render_release_block_uncoloured() {
        struct Case {
            date: &'static str,
            archive_size: u64,
            file_count: usize,
            state: ReleaseBlockState,
            verified: &'static str,
            linked: Option<ReleaseBlockLink<'static>>,
            hash: Option<&'static str>,
            expected: Vec<&'static str>,
        }

        let cases = vec![
            // 1. Downloading block
            Case {
                date: "2026-07-31",
                archive_size: 34_500_000,
                file_count: 5,
                state: ReleaseBlockState::Downloading {
                    bytes_done: 22 * 1024 * 1024,
                    rate: Some(13.6 * 1024.0 * 1024.0),
                    eta: Some(Duration::from_secs(1)),
                },
                verified: "sha256 from TRUD API",
                linked: Some(ReleaseBlockLink {
                    target: "releases/2026-07-31",
                    unchanged: false,
                }),
                hash: None,
                expected: vec![
                    "  2026-07-31    █████████████░░░░░░░  22MB   5 files  13.6MB/s  eta 1s",
                    "  verified      -",
                    "  linked        -",
                ],
            },
            // 2. Finished block
            Case {
                date: "2026-07-31",
                archive_size: 37_983_173,
                file_count: 5,
                state: ReleaseBlockState::Done {
                    elapsed: Duration::from_millis(3100),
                },
                verified: "sha256 from TRUD API",
                linked: Some(ReleaseBlockLink {
                    target: "releases/2026-07-31",
                    unchanged: false,
                }),
                hash: None,
                expected: vec![
                    "  2026-07-31    ████████████████████  36MB   5 files  in 3.1s",
                    "  verified      sha256 from TRUD API",
                    "  linked        current → releases/2026-07-31",
                ],
            },
            // 3. Cached block with (unchanged)
            Case {
                date: "2026-07-31",
                archive_size: 37_983_173,
                file_count: 5,
                state: ReleaseBlockState::Cached,
                verified: "sha256 from TRUD API",
                linked: Some(ReleaseBlockLink {
                    target: "releases/2026-07-31",
                    unchanged: true,
                }),
                hash: None,
                expected: vec![
                    "  2026-07-31    ████████████████████  36MB   5 files  cached",
                    "  verified      sha256 from TRUD API",
                    "  linked        current → releases/2026-07-31 (unchanged)",
                ],
            },
            // 4. Verbose block
            Case {
                date: "2026-07-31",
                archive_size: 37_983_173,
                file_count: 5,
                state: ReleaseBlockState::Done {
                    elapsed: Duration::from_millis(3100),
                },
                verified: "sha256 from TRUD API",
                linked: Some(ReleaseBlockLink {
                    target: "releases/2026-07-31",
                    unchanged: false,
                }),
                hash: Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933"),
                expected: vec![
                    "  2026-07-31    ████████████████████  36MB   5 files  in 3.1s",
                    "  verified      sha256 from TRUD API",
                    "  linked        current → releases/2026-07-31",
                    "  sha256        8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
                ],
            },
            // 5. Block with no linked row
            Case {
                date: "2026-07-31",
                archive_size: 37_983_173,
                file_count: 5,
                state: ReleaseBlockState::Done {
                    elapsed: Duration::from_millis(3100),
                },
                verified: "sha256 from TRUD API",
                linked: None,
                hash: None,
                expected: vec![
                    "  2026-07-31    ████████████████████  36MB   5 files  in 3.1s",
                    "  verified      sha256 from TRUD API",
                ],
            },
            // 6. Single-file block reading "1 file"
            Case {
                date: "2026-07-31",
                archive_size: 37_983_173,
                file_count: 1,
                state: ReleaseBlockState::Done {
                    elapsed: Duration::from_millis(3100),
                },
                verified: "sha256 from TRUD API",
                linked: Some(ReleaseBlockLink {
                    target: "releases/2026-07-31",
                    unchanged: false,
                }),
                hash: None,
                expected: vec![
                    "  2026-07-31    ████████████████████  36MB   1 file  in 3.1s",
                    "  verified      sha256 from TRUD API",
                    "  linked        current → releases/2026-07-31",
                ],
            },
        ];

        for (i, c) in cases.into_iter().enumerate() {
            let actual = render_release_block(&ReleaseBlockParams {
                date: c.date,
                archive_size: c.archive_size,
                file_count: c.file_count,
                state: &c.state,
                verified: c.verified,
                linked: c.linked,
                hash: c.hash,
                color: false,
            });
            assert_eq!(actual, c.expected, "Case {} failed", i + 1);
        }
    }

    #[test]
    fn test_render_release_block_coloured() {
        // Test downloading case for filled, empty, stats, and "-"
        let dl_lines = render_release_block(&ReleaseBlockParams {
            date: "2026-07-31",
            archive_size: 34_500_000,
            file_count: 5,
            state: &ReleaseBlockState::Downloading {
                bytes_done: 22 * 1024 * 1024,
                rate: Some(13.6 * 1024.0 * 1024.0),
                eta: Some(Duration::from_secs(1)),
            },
            verified: "sha256 from TRUD API",
            linked: Some(ReleaseBlockLink {
                target: "releases/2026-07-31",
                unchanged: false,
            }),
            hash: None,
            color: true,
        });

        let cyan_filled = format!("{}{}{}", crate::ansi::ANSI_CYAN, "█".repeat(13), crate::ansi::ANSI_RESET);
        let muted_empty = format!("{}{}{}", crate::ansi::ANSI_MUTED, "░".repeat(7), crate::ansi::ANSI_RESET);
        let muted_stats = format!("{}  22MB   5 files  13.6MB/s  eta 1s{}", crate::ansi::ANSI_MUTED, crate::ansi::ANSI_RESET);
        let muted_dash = format!("{}-{}", crate::ansi::ANSI_MUTED, crate::ansi::ANSI_RESET);

        assert!(dl_lines[0].contains(&cyan_filled), "Row 1 must have colored filled bar: {}", dl_lines[0]);
        assert!(dl_lines[0].contains(&muted_empty), "Row 1 must have colored empty bar: {}", dl_lines[0]);
        assert!(dl_lines[0].contains(&muted_stats), "Row 1 must have colored stats: {}", dl_lines[0]);
        assert!(dl_lines[1].contains(&muted_dash), "Row 2 must have colored '-': {}", dl_lines[1]);
        assert!(dl_lines[2].contains(&muted_dash), "Row 3 must have colored '-': {}", dl_lines[2]);

        // Test cached case for "→" and " (unchanged)"
        let cached_lines = render_release_block(&ReleaseBlockParams {
            date: "2026-07-31",
            archive_size: 37_983_173,
            file_count: 5,
            state: &ReleaseBlockState::Cached,
            verified: "sha256 from TRUD API",
            linked: Some(ReleaseBlockLink {
                target: "releases/2026-07-31",
                unchanged: true,
            }),
            hash: None,
            color: true,
        });

        let cyan_arrow = format!("{}→{}", crate::ansi::ANSI_CYAN, crate::ansi::ANSI_RESET);
        let muted_unchanged = format!("{} (unchanged){}", crate::ansi::ANSI_MUTED, crate::ansi::ANSI_RESET);

        assert!(cached_lines[2].contains(&cyan_arrow), "Row 3 must have colored arrow: {}", cached_lines[2]);
        assert!(cached_lines[2].contains(&muted_unchanged), "Row 3 must have colored unchanged suffix: {}", cached_lines[2]);
    }

    #[test]
    fn test_error_after_update_live_block_leaves_cross_lines_last() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        struct BufferWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufferWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let caps = ProgressCaps {
            is_tty: true,
            no_color: true,
            quiet: false,
            verbose: false,
            width: 80,
        };

        let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));
        progress.update_live_block(vec![
            "  2026-07-31    █████████████░░░░░░░  22MB   5 files  13.6MB/s  eta 1s".to_string(),
            "  verified      -".to_string(),
            "  linked        -".to_string(),
        ]);

        progress.error(
            "SHA-256 mismatch for 2026-07-31",
            &[
                "Expected: 8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
                "Got:      938463DF0035AAD2D2D3291E47BE453509A4BAF21CF73ACD293FD4C141CD4CAB",
                "File renamed to /path/to/archive.zip.bad-sha",
            ],
        );

        let out = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();

        // Must contain the error block
        assert!(out.contains("✖ SHA-256 mismatch for 2026-07-31"));
        assert!(out.contains("File renamed to /path/to/archive.zip.bad-sha"));

        // The error block lines must be the last visible text; the live block must NOT be redrawn after the error
        let error_pos = out
            .rfind("File renamed to /path/to/archive.zip.bad-sha")
            .unwrap();
        let trailing = &out[error_pos..];
        assert!(
            !trailing.contains("2026-07-31"),
            "Live block must not be redrawn after error: {}",
            trailing
        );
        assert!(
            !trailing.contains("verified"),
            "Live block must not be redrawn after error: {}",
            trailing
        );
        assert!(
            !trailing.contains("linked"),
            "Live block must not be redrawn after error: {}",
            trailing
        );
    }
}
