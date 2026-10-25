//! The console (N7, M5): program output through a terminal emulator (alacritty_terminal), drawn
//! by an egui view of our own.
//!
//! A [`TermModel`] is one terminal: fed bytes from whichever thread receives a program's output
//! (the run worker), drawn by [`view::show`] on the UI thread. Pty output and pipe output go
//! through the same emulator. A bare `\n` starts a new line at the left, as it does in a terminal
//! (the model adds the `\r` a pty would), except while a program in a pty is attached: that
//! program gets the keys typed into the view and its size, and its pty adds the `\r` itself.

pub mod input;
pub mod palette;
pub mod session;
pub mod view;

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use alacritty_terminal::Term;
use launcher_core::engine::effects::Proc;

/// Lines kept above the screen (the full output is in the step's log on disk).
pub const SCROLLBACK: usize = 10_000;

/// Where the terminal's own replies (cursor position reports and the like) go.
#[derive(Clone, Default)]
pub struct Listener {
    process: Arc<Mutex<Option<Arc<dyn Proc>>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        if let TermEvent::PtyWrite(text) = event {
            if let Some(p) = self.process.lock().ok().and_then(|p| p.clone()) {
                p.write(text.as_bytes());
            }
        }
    }
}

pub struct TermModel {
    pub(crate) term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    size: (usize, usize),
    /// Keep the view at the newest output (off while the user reads back).
    pub(crate) follow: bool,
    /// Bytes fed so far.
    pub bytes: u64,
    /// The output isn't from a pty: `\n` gets a `\r`.
    add_cr: bool,
    /// Something visible has been printed (a pty session starts with escape sequences only).
    text_seen: bool,
}

pub type SharedTerm = Arc<Mutex<TermModel>>;

impl Default for TermModel {
    fn default() -> TermModel {
        TermModel::new()
    }
}

impl TermModel {
    pub fn new() -> TermModel {
        let size = (100usize, 24usize);
        let listener = Listener::default();
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        TermModel {
            term: Term::new(config, &TermSize::new(size.0, size.1), listener.clone()),
            parser: Processor::new(),
            listener,
            size,
            follow: true,
            bytes: 0,
            add_cr: true,
            text_seen: false,
        }
    }

    pub fn shared() -> SharedTerm {
        Arc::new(Mutex::new(TermModel::new()))
    }

    /// Program output.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len() as u64;
        if self.add_cr && bytes.contains(&b'\n') {
            let mut with_cr = Vec::with_capacity(bytes.len() + bytes.len() / 16);
            for &b in bytes {
                if b == b'\n' {
                    with_cr.push(b'\r');
                }
                with_cr.push(b);
            }
            self.parser.advance(&mut self.term, &with_cr);
        } else {
            self.parser.advance(&mut self.term, bytes);
        }
        if !self.text_seen {
            self.text_seen = self.scan_for_text();
        }
        if self.follow {
            self.show_newest();
        }
    }

    /// Whether anything visible has been printed.
    pub fn has_text(&self) -> bool {
        self.text_seen
    }

    fn scan_for_text(&self) -> bool {
        use alacritty_terminal::index::{Column, Line, Point};
        let grid = self.term.grid();
        let top = -(grid.history_size() as i32);
        (top..grid.screen_lines() as i32).any(|line| {
            (0..grid.columns())
                .any(|col| !grid[Point::new(Line(line), Column(col))].c.is_whitespace())
        })
    }

    /// Where following puts the view: the newest line at the bottom, with as much of the
    /// scrollback above it as fits. A pty session starts by clearing the screen (ConPTY sends
    /// `ESC[2J`), which moves what the step printed before into the scrollback; this keeps it in
    /// view, so a step's output reads top to bottom.
    pub fn newest_offset(&self) -> usize {
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            return 0;
        }
        let grid = self.term.grid();
        let cursor = grid.cursor.point.line.0.max(0) as usize;
        grid.screen_lines()
            .saturating_sub(cursor + 1)
            .min(grid.history_size())
    }

    /// Show the newest output.
    pub fn show_newest(&mut self) {
        let want = self.newest_offset();
        let have = self.term.grid().display_offset();
        if want != have {
            self.term
                .scroll_display(Scroll::Delta(want as i32 - have as i32));
        }
    }

    /// A line from the launcher itself (a step's command, a log step), dimmed. It starts on a
    /// fresh line even when the program's last output didn't end with one.
    pub fn note(&mut self, text: &str) {
        self.line("2", text);
    }

    /// A line of the launcher's own in a style (an SGR parameter list: "2" is dim, "92" bright
    /// green), on a fresh line.
    pub fn line(&mut self, style: &str, text: &str) {
        let fresh = if self.term.grid().cursor.point.column.0 > 0 {
            "\r\n"
        } else {
            ""
        };
        let line = format!(
            "{fresh}\x1b[0;{style}m{}\x1b[0m\r\n",
            text.replace('\n', "\r\n")
        );
        self.feed(line.as_bytes());
    }

    /// Show the newest output from now on (a new run starts, say), even if the view was
    /// scrolled back.
    pub fn follow_newest(&mut self) {
        self.follow = true;
        self.show_newest();
    }

    /// The program in a pty that typing goes to (and that gets resized with the view). A program
    /// without a pty is not attached.
    pub fn attach(&mut self, p: Arc<dyn Proc>) {
        if !p.is_pty() {
            return;
        }
        self.add_cr = false;
        p.resize(self.size.0 as u16, self.size.1 as u16);
        if let Ok(mut slot) = self.listener.process.lock() {
            *slot = Some(p);
        }
    }

    /// The program ended: typing goes nowhere.
    pub fn detach(&mut self) {
        let was = self
            .listener
            .process
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if was.is_some() {
            self.add_cr = true;
        }
    }

    pub fn attached(&self) -> Option<Arc<dyn Proc>> {
        self.listener.process.lock().ok().and_then(|p| p.clone())
    }

    pub fn size(&self) -> (usize, usize) {
        self.size
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if (cols, rows) == self.size {
            return;
        }
        self.size = (cols, rows);
        self.term.resize(TermSize::new(cols, rows));
        if let Some(p) = self.attached() {
            p.resize(cols as u16, rows as u16);
        }
        if self.follow {
            self.show_newest();
        }
    }

    /// Keys typed into the console.
    pub fn input(&mut self, bytes: &[u8]) {
        if let Some(p) = self.attached() {
            p.write(bytes);
            self.follow = true;
            self.show_newest();
        }
    }

    /// Lines in the scrollback plus the screen.
    #[cfg(test)]
    pub fn total_lines(&self) -> usize {
        self.term.total_lines()
    }

    /// Everything in the scrollback and on the screen, as text (Copy all).
    pub fn text(&self) -> String {
        use alacritty_terminal::index::{Column, Line, Point};
        let grid = self.term.grid();
        let top = Line(-(grid.history_size() as i32));
        let bottom = Line(grid.screen_lines() as i32 - 1);
        let last = Column(grid.columns() - 1);
        let mut s = self
            .term
            .bounds_to_string(Point::new(top, Column(0)), Point::new(bottom, last));
        let trimmed = s.trim_end().len();
        s.truncate(trimmed);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line, Point};

    fn row(m: &TermModel, line: i32) -> String {
        let mut s = String::new();
        for c in 0..m.size.0 {
            s.push(m.term.grid()[Point::new(Line(line), Column(c))].c);
        }
        s.trim_end().to_string()
    }

    #[test]
    fn pipe_output_starts_each_line_at_the_left() {
        let mut m = TermModel::new();
        m.feed(b"first\nsecond\n");
        assert_eq!(row(&m, 0), "first");
        assert_eq!(row(&m, 1), "second");
    }

    #[test]
    fn progress_lines_overwrite_themselves() {
        let mut m = TermModel::new();
        m.feed(b"Receiving objects:  10%\rReceiving objects: 100%, done.\n");
        assert_eq!(row(&m, 0), "Receiving objects: 100%, done.");
    }

    #[test]
    fn notes_start_on_a_fresh_line() {
        let mut m = TermModel::new();
        m.feed(b"no newline");
        m.note("> next");
        assert_eq!(row(&m, 0), "no newline");
        assert_eq!(row(&m, 1), "> next");
        assert_eq!(m.text(), "no newline\n> next");
    }

    #[test]
    fn a_new_pty_session_keeps_earlier_output_in_view() {
        let mut m = TermModel::new();
        m.note("Ordering for Ada");
        assert!(m.has_text());
        // What ConPTY sends first: clear the screen, cursor home.
        m.feed(b"\x1b[?25l\x1b[2J\x1b[m\x1b[Hpaying\r\n");
        let offset = m.term.grid().display_offset() as i32;
        assert_eq!(row(&m, -offset), "Ordering for Ada");
        assert_eq!(row(&m, 1 - offset), "paying");
        let fresh = TermModel::new();
        assert!(!fresh.has_text());
        let mut escapes = TermModel::new();
        escapes.feed(b"\x1b[?25l\x1b[2J\x1b[m\x1b[H\x1b]0;title\x07");
        assert!(!escapes.has_text());
    }

    #[test]
    fn ten_megabytes_of_output_is_quick_and_bounded() {
        let mut m = TermModel::new();
        let line = "\x1b[32m   Compiling\x1b[0m some-crate v1.2.3 (C:\\builds\\project\\crates\\some-crate)\n";
        let chunk: String = line.repeat(1000);
        let target = if cfg!(debug_assertions) {
            2 << 20
        } else {
            10 << 20
        };
        let start = std::time::Instant::now();
        while (m.bytes as usize) < target {
            m.feed(chunk.as_bytes());
        }
        let elapsed = start.elapsed();
        // At least 4 MB/s even in a debug build; the scrollback stays capped.
        let mb_per_s = m.bytes as f64 / 1e6 / elapsed.as_secs_f64();
        assert!(mb_per_s > 4.0, "{mb_per_s:.1} MB/s");
        assert!(m.total_lines() <= SCROLLBACK + m.size().1);
    }
}
