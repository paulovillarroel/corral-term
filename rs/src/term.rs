//! The terminal for a full-screen viewer: alternate screen, keys one at a
//! time, mouse, size. Straight to the C library (termios, poll), no crates.

use std::fs::File;
use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;

#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[repr(C)]
#[derive(Default)]
struct Winsize {
    rows: u16,
    cols: u16,
    xpixel: u16,
    ypixel: u16,
}

extern "C" {
    fn tcgetattr(fd: i32, t: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, action: i32, t: *const Termios) -> i32;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn poll(fds: *mut PollFd, n: u64, timeout: i32) -> i32;
}

const ICANON: u32 = 0o2;
const ECHO: u32 = 0o10;
const VTIME: usize = 5;
const VMIN: usize = 6;
const TCSADRAIN: i32 = 1;
const TIOCGWINSZ: u64 = 0x5413;
const POLLIN: i16 = 1;

pub struct Screen {
    saved: Termios,
    mouse: bool,
    pending: std::cell::RefCell<std::collections::VecDeque<String>>,
}

pub fn out(s: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(s.as_bytes());
    let _ = stdout.flush();
}

impl Screen {
    pub fn enter(mouse: bool) -> Option<Screen> {
        let mut t = Termios { c_iflag: 0, c_oflag: 0, c_cflag: 0, c_lflag: 0, c_line: 0, c_cc: [0; 32], c_ispeed: 0, c_ospeed: 0 };
        // SAFETY: plain C calls on stdin with a correctly laid out struct.
        if unsafe { tcgetattr(0, &mut t) } != 0 {
            return None;
        }
        let saved = t;
        t.c_lflag &= !(ICANON | ECHO);
        t.c_cc[VMIN] = 1;
        t.c_cc[VTIME] = 0;
        unsafe { tcsetattr(0, TCSADRAIN, &t) };
        out(&format!("\x1b[?1049h\x1b[?25l{}", if mouse { "\x1b[?1000h\x1b[?1006h" } else { "" }));
        Some(Screen { saved, mouse, pending: Default::default() })
    }

    pub fn size(&self) -> (usize, usize) {
        size()
    }

    /// Is there input waiting (queued keys, or bytes on the terminal)?
    pub fn has_input(&self) -> bool {
        if !self.pending.borrow().is_empty() {
            return true;
        }
        let mut fds = PollFd { fd: 0, events: POLLIN, revents: 0 };
        // SAFETY: one pollfd for stdin, no wait.
        unsafe { poll(&mut fds, 1, 0) > 0 }
    }

    /// The next key, or None after `timeout_ms`. Mouse clicks come as "mouse:b:x:y".
    /// One read can hold several keys (key repeat, pasted text): they are queued.
    pub fn key(&self, timeout_ms: i32) -> Option<String> {
        if let Some(k) = self.pending.borrow_mut().pop_front() {
            return Some(k);
        }
        let mut fds = PollFd { fd: 0, events: POLLIN, revents: 0 };
        // SAFETY: one pollfd for stdin.
        if unsafe { poll(&mut fds, 1, timeout_ms) } <= 0 {
            return None;
        }
        // Read the terminal directly: Stdin's buffer could keep keys that poll() no longer sees.
        // SAFETY: fd 0 stays open; ManuallyDrop keeps File from closing it.
        let mut tty = ManuallyDrop::new(unsafe { File::from_raw_fd(0) });
        let mut buf = [0u8; 4096];
        let n = match tty.read(&mut buf) {
            Ok(n) if n > 0 => n,
            _ => return Some("quit".into()),
        };
        let mut raw = buf[..n].to_vec();
        // A burst can end in the middle of an escape sequence (a mouse report cut
        // in two): wait a moment for the rest instead of reading the pieces as keys.
        for _ in 0..10 {
            if !incomplete_tail(&raw) {
                break;
            }
            let mut fds = PollFd { fd: 0, events: POLLIN, revents: 0 };
            // SAFETY: one pollfd for stdin.
            if unsafe { poll(&mut fds, 1, 30) } <= 0 {
                break;
            }
            match tty.read(&mut buf) {
                Ok(n) if n > 0 => raw.extend_from_slice(&buf[..n]),
                _ => break,
            }
        }
        let data = String::from_utf8_lossy(&raw).to_string();
        let mut pending = self.pending.borrow_mut();
        pending.extend(tokens(&data).into_iter().map(|t| name(&t)).filter(|k| !k.is_empty()));
        Some(pending.pop_front().unwrap_or_default())
    }
}

/// Does the input end inside an escape sequence (ESC, ESC [ params, ESC [ < mouse, ESC O)?
fn incomplete_tail(raw: &[u8]) -> bool {
    let Some(start) = raw.iter().rposition(|&b| b == 0x1b) else { return false };
    let seq = &raw[start..];
    match seq.get(1) {
        None => true, // a lone ESC may be the Esc key, or the start of a sequence: wait briefly
        Some(b'[') => {
            if seq.get(2) == Some(&b'<') {
                !seq[3..].iter().any(|&b| b == b'M' || b == b'm')
            } else {
                !seq[2..].iter().any(|&b| (b'@'..=b'~').contains(&b))
            }
        }
        Some(b'O') => seq.len() < 3,
        _ => false,
    }
}

/// Split raw input into keys: escape sequences, mouse reports, single characters.
fn tokens(data: &str) -> Vec<String> {
    let chars: Vec<char> = data.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '[' {
            let mut j = i + 2;
            if j < chars.len() && chars[j] == '<' {
                while j < chars.len() && chars[j] != 'M' && chars[j] != 'm' {
                    j += 1;
                }
            } else {
                while j < chars.len() && !('@'..='~').contains(&chars[j]) {
                    j += 1;
                }
            }
            let end = (j + 1).min(chars.len());
            out.push(chars[i..end].iter().collect());
            i = end;
        } else if chars[i] == '\x1b' && i + 2 < chars.len() && chars[i + 1] == 'O' {
            out.push(chars[i..i + 3].iter().collect());
            i += 3;
        } else {
            out.push(chars[i].to_string());
            i += 1;
        }
    }
    out
}

/// A key's name ("up", "enter", "mouse:0:12:3", or the character itself).
fn name(t: &str) -> String {
    if let Some(rest) = t.strip_prefix("\x1b[<") {
        // SGR mouse: ESC [ < button ; x ; y M (press) or m (release)
        if let Some(nums) = rest.strip_suffix('M') {
            let nums: Vec<&str> = nums.split(';').collect();
            if nums.len() == 3 {
                return format!("mouse:{}:{}:{}", nums[0], nums[1], nums[2]);
            }
        }
        return String::new();
    }
    let named = match t {
        "\x1b[A" | "\x1bOA" => "up",
        "\x1b[B" | "\x1bOB" => "down",
        "\x1b[C" | "\x1bOC" => "right",
        "\x1b[D" | "\x1bOD" => "left",
        "\x1b[H" | "\x1bOH" | "\x1b[1~" => "home",
        "\x1b[F" | "\x1bOF" | "\x1b[4~" => "end",
        "\x1b[5~" => "pgup",
        "\x1b[6~" => "pgdn",
        "\x1b[Z" => "backtab",
        "\r" | "\n" => "enter",
        "\x1b" => "esc",
        "\x7f" | "\x08" => "backspace",
        "\x03" => "quit",
        _ => "",
    };
    if !named.is_empty() {
        named.to_string()
    } else if t.chars().count() == 1 {
        t.to_string()
    } else {
        String::new()
    }
}

/// A console's line-editing mode: keys one by one, no echo, and Ctrl+C, Ctrl+Z
/// and Ctrl+S as plain keys (the console interrupts the kernel itself). Output
/// processing stays on, so "\n" still starts a new line.
pub struct Raw {
    saved: Termios,
}

const ISIG: u32 = 0o1;
const IEXTEN: u32 = 0o100000;
const IXON: u32 = 0o2000;
const ICRNL: u32 = 0o400;

impl Raw {
    pub fn enter() -> Option<Raw> {
        let mut t = Termios { c_iflag: 0, c_oflag: 0, c_cflag: 0, c_lflag: 0, c_line: 0, c_cc: [0; 32], c_ispeed: 0, c_ospeed: 0 };
        // SAFETY: plain C calls on stdin with a correctly laid out struct.
        if unsafe { tcgetattr(0, &mut t) } != 0 {
            return None;
        }
        let saved = t;
        t.c_lflag &= !(ICANON | ECHO | ISIG | IEXTEN);
        t.c_iflag &= !(IXON | ICRNL);
        t.c_cc[VMIN] = 1;
        t.c_cc[VTIME] = 0;
        unsafe { tcsetattr(0, TCSADRAIN, &t) };
        Some(Raw { saved })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        // SAFETY: restore the settings read in enter().
        unsafe { tcsetattr(0, TCSADRAIN, &self.saved) };
    }
}

/// Columns and rows of the terminal on stdout.
pub fn size() -> (usize, usize) {
    let mut ws = Winsize::default();
    // SAFETY: TIOCGWINSZ fills a winsize struct.
    if unsafe { ioctl(1, TIOCGWINSZ, &mut ws as *mut Winsize) } == 0 && ws.cols > 0 {
        (ws.cols as usize, ws.rows as usize)
    } else {
        (80, 24)
    }
}

/// Width and height of the terminal in pixels, when the terminal reports them (else 0, 0).
pub fn pixels() -> (usize, usize) {
    let mut ws = Winsize::default();
    // SAFETY: TIOCGWINSZ fills a winsize struct.
    if unsafe { ioctl(1, TIOCGWINSZ, &mut ws as *mut Winsize) } == 0 {
        (ws.xpixel as usize, ws.ypixel as usize)
    } else {
        (0, 0)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        out(&format!("{}\x1b[0m\x1b[2J\x1b[?25h\x1b[?1049l", if self.mouse { "\x1b[?1006l\x1b[?1000l" } else { "" }));
        // SAFETY: restore the settings read in enter().
        unsafe { tcsetattr(0, TCSADRAIN, &self.saved) };
    }
}

/// Cut to `width` characters, with an ellipsis.
pub fn clip(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        s.to_string()
    } else if width == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(width - 1).collect();
        t.push('…');
        t
    }
}

pub fn width(s: &str) -> usize {
    s.chars().count()
}
