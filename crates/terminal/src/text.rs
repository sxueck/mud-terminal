//! Minimal VTE-lite stream parser.
//!
//! Runs alongside the full `alacritty_terminal` parser and extracts:
//! * plain text (used for the transcript and block output),
//! * line editing side effects (backspace, erase-line),
//! * OSC 133 shell-integration markers (prompt/input/output/exit),
//!   so block timing never depends on the emulator's internals.
//!
//! The parser is a byte-level state machine, so sequences may be split across
//! arbitrary chunk boundaries.

/// Shell integration markers (OSC 133) and window title changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Marker {
    /// OSC 133;A - prompt start.
    PromptStart,
    /// OSC 133;B - command input start.
    InputStart,
    /// OSC 133;C - command output start.
    OutputStart,
    /// Private OSC 133;E extension: authoritative command from the shell hook.
    Command(String),
    /// OSC 133;D[;exit] - command finished.
    CommandExit(Option<i32>),
    /// OSC 0/2 - window title.
    Title(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextEvent {
    Print(char),
    Newline,
    /// A lone `\r` not followed by `\n`: the line is being redrawn.
    LineReset,
    Backspace,
    /// CSI K / CSI J: the current line is being redrawn.
    EraseLine,
    /// `CSI ? 47 h/l` or `CSI ? 1047 h/l`: alternate screen requests that
    /// alacritty_terminal's vte version does not decode natively; the
    /// backend translates them to the 1049 equivalent.
    AltScreenCompat(bool),
    Marker(Marker),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Escape,
    /// After ESC ( : one more byte selects a charset then back to ground.
    Charset,
    Csi,
    Osc,
    /// ESC seen inside an OSC string: only `ST` (ESC \) is valid.
    OscEsc,
    /// DCS / SOS / PM / APC payload, skipped until ST.
    SkipString,
}

const OSC_MAX: usize = 8192;

#[derive(Debug)]
pub struct PlainTextParser {
    state: State,
    osc: Vec<u8>,
    osc_overflow: bool,
    utf8_bytes: [u8; 4],
    utf8_len: usize,
    utf8_expect: usize,
    /// A byte that was consumed but must be reprocessed in Ground state.
    pending: Option<u8>,
    /// Buffered CSI parameter/intermediate bytes (bounded).
    csi: Vec<u8>,
    /// A `\r` whose meaning (newline vs redraw) depends on the next byte.
    pending_cr: bool,
    /// Set after `\r` so `\r\n` collapses into a single newline.
    last_was_cr: bool,
    concealed: bool,
}

impl Default for PlainTextParser {
    fn default() -> Self {
        Self::new()
    }
}

impl PlainTextParser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            osc: Vec::new(),
            osc_overflow: false,
            utf8_bytes: [0; 4],
            utf8_len: 0,
            utf8_expect: 0,
            pending: None,
            csi: Vec::new(),
            pending_cr: false,
            last_was_cr: false,
            concealed: false,
        }
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<TextEvent>) {
        for &byte in bytes {
            if let Some(replay) = self.pending.take() {
                self.step(replay, out);
            }
            self.step(byte, out);
        }
        // Keep a trailing CR pending: its LF may arrive in the next PTY read.
    }

    /// Resolve a deferred `\r`. `followed_by_lf` picks newline vs redraw.
    fn flush_cr(&mut self, out: &mut Vec<TextEvent>, followed_by_lf: bool) {
        self.pending_cr = false;
        self.last_was_cr = followed_by_lf;
        out.push(if followed_by_lf {
            TextEvent::Newline
        } else {
            TextEvent::LineReset
        });
    }

    fn step(&mut self, byte: u8, out: &mut Vec<TextEvent>) {
        match self.state {
            State::Ground => self.step_ground(byte, out),
            State::Escape => self.step_escape(byte),
            State::Charset => {
                self.state = State::Ground;
            }
            State::Csi => self.step_csi(byte, out),
            State::Osc => self.step_osc(byte, out),
            State::OscEsc => {
                self.state = State::Ground;
                let payload = std::mem::take(&mut self.osc);
                let overflow = self.osc_overflow;
                self.osc_overflow = false;
                if byte == b'\\' && !overflow {
                    if let Some(marker) = decode_osc(&payload) {
                        out.push(TextEvent::Marker(marker));
                    }
                } else {
                    // Invalid terminator; drop the sequence but reprocess the
                    // byte since it may itself start an escape.
                    self.pending = Some(byte);
                }
            }
            State::SkipString => {
                if byte == 0x1b {
                    self.state = State::Escape;
                }
            }
        }
    }

    fn step_ground(&mut self, byte: u8, out: &mut Vec<TextEvent>) {
        if byte < 0x80 {
            self.utf8_len = 0;
        }
        if self.pending_cr {
            self.flush_cr(out, byte == b'\n');
        }
        match byte {
            0x1b => self.state = State::Escape,
            b'\r' => {
                self.pending_cr = true;
            }
            b'\n' => {
                if self.pending_cr {
                    self.flush_cr(out, true);
                } else if !self.last_was_cr {
                    out.push(TextEvent::Newline);
                }
                self.last_was_cr = false;
            }
            0x08 => {
                self.last_was_cr = false;
                out.push(TextEvent::Backspace);
            }
            b'\t' => {
                self.last_was_cr = false;
                if !self.concealed {
                    out.push(TextEvent::Print('\t'));
                }
            }
            0x00..=0x1f | 0x7f => {
                // Other C0 controls: no textual meaning for us.
                self.last_was_cr = false;
            }
            0x20..=0x7e => {
                self.last_was_cr = false;
                if !self.concealed {
                    out.push(TextEvent::Print(byte as char));
                }
            }
            _ => {
                self.last_was_cr = false;
                self.step_utf8(byte, out);
            }
        }
    }

    fn step_utf8(&mut self, byte: u8, out: &mut Vec<TextEvent>) {
        if self.utf8_len == 0 {
            self.utf8_expect = utf8_length(byte);
            if self.utf8_expect == 0 {
                return; // Invalid lead byte: drop.
            }
            self.utf8_bytes[0] = byte;
            self.utf8_len = 1;
            return;
        }
        if byte & 0xc0 != 0x80 {
            // Invalid continuation: reset and try the byte as a new lead.
            self.utf8_len = 0;
            self.utf8_expect = 0;
            self.step_utf8(byte, out);
            return;
        }
        self.utf8_bytes[self.utf8_len] = byte;
        self.utf8_len += 1;
        if self.utf8_len < self.utf8_expect {
            return;
        }
        let sequence: &[u8] = &self.utf8_bytes[..self.utf8_len];
        if let Ok(text) = std::str::from_utf8(sequence)
            && let Some(ch) = text.chars().next()
            && !self.concealed
        {
            out.push(TextEvent::Print(ch));
        }
        self.utf8_len = 0;
        self.utf8_expect = 0;
    }

    fn step_escape(&mut self, byte: u8) {
        self.state = State::Ground;
        match byte {
            b'[' => {
                self.csi.clear();
                self.state = State::Csi;
            }
            b']' => {
                self.osc.clear();
                self.osc_overflow = false;
                self.state = State::Osc;
            }
            b'c' => self.concealed = false,
            b'P' | b'X' | b'^' | b'_' => self.state = State::SkipString,
            b'(' | b')' | b'*' | b'+' => self.state = State::Charset,
            _ => {}
        }
    }

    fn step_csi(&mut self, byte: u8, out: &mut Vec<TextEvent>) {
        match byte {
            0x40..=0x7e => {
                self.state = State::Ground;
                if byte == b'm' {
                    self.sgr();
                }
                // Erase-in-line / erase-in-display mean the shell or an
                // application is redrawing the current line.
                if byte == b'K' || byte == b'J' {
                    out.push(TextEvent::EraseLine);
                }
                if (byte == b'h' || byte == b'l')
                    && let Some(enable) = alt_screen_compat(&self.csi, byte == b'h')
                {
                    out.push(TextEvent::AltScreenCompat(enable));
                }
                self.csi.clear();
            }
            0x20..=0x3f => {
                if self.csi.len() < 32 {
                    self.csi.push(byte);
                }
            }
            _ => self.state = State::Ground,
        }
    }

    fn sgr(&mut self) {
        let text = String::from_utf8_lossy(&self.csi);
        let mut params = text.split(';').map(|part| {
            if part.is_empty() {
                Some(0)
            } else {
                part.parse::<u16>().ok()
            }
        });
        while let Some(code) = params.next() {
            match code {
                Some(0 | 28) => self.concealed = false,
                Some(8) => self.concealed = true,
                Some(38 | 48 | 58) => {
                    // RGB/index components are not independent SGR attributes.
                    let count = match params.next() {
                        Some(Some(2)) => 3,
                        Some(Some(5)) => 1,
                        _ => 0,
                    };
                    for _ in 0..count {
                        let _ = params.next();
                    }
                }
                _ => {}
            }
        }
    }

    fn step_osc(&mut self, byte: u8, out: &mut Vec<TextEvent>) {
        match byte {
            0x07 => {
                self.state = State::Ground;
                let payload = std::mem::take(&mut self.osc);
                let overflow = self.osc_overflow;
                self.osc_overflow = false;
                if !overflow && let Some(marker) = decode_osc(&payload) {
                    out.push(TextEvent::Marker(marker));
                }
            }
            0x1b => self.state = State::OscEsc,
            _ => {
                if self.osc.len() < OSC_MAX {
                    self.osc.push(byte);
                } else {
                    self.osc_overflow = true;
                }
            }
        }
    }
}

/// Recognize `CSI ? 47 h/l` and `CSI ? 1047 h/l` alternate screen requests.
fn alt_screen_compat(params: &[u8], set: bool) -> Option<bool> {
    match params {
        b"?47" | b"?1047" => Some(set),
        _ => None, // 1049 has native support in alacritty_terminal
    }
}

/// Rewrites `CSI ? 47 h/l` and `CSI ? 1047 h/l` into the `?1049` equivalent
/// alacritty_terminal understands, preserving stream order even when a
/// sequence is split across PTY read chunks.
#[derive(Debug, Default)]
pub(crate) struct AltCompatTranslator {
    pending: Vec<u8>,
}

impl AltCompatTranslator {
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut out: Vec<u8> = Vec::with_capacity(input.len());
        let mut flush = 0;
        let mut i = 0;
        let mut split = input.len();
        while i < input.len() {
            if input[i] == 0x1b && i + 1 == input.len() {
                split = i;
                break;
            }
            if input[i] == 0x1b && i + 1 < input.len() && input[i + 1] == b'[' {
                let mut j = i + 2;
                while j < input.len() && !(0x40..=0x7e).contains(&input[j]) {
                    j += 1;
                }
                if j >= input.len() {
                    // No final byte yet: retain the partial sequence.
                    if j - i <= 16 {
                        split = i;
                        break;
                    }
                } else {
                    let replacement: &[u8] = match &input[i..=j] {
                        b"\x1b[?47h" | b"\x1b[?1047h" => b"\x1b[?1049h",
                        b"\x1b[?47l" | b"\x1b[?1047l" => b"\x1b[?1049l",
                        _ => &[],
                    };
                    if !replacement.is_empty() {
                        out.extend_from_slice(&input[flush..i]);
                        out.extend_from_slice(replacement);
                        flush = j + 1;
                    }
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
        out.extend_from_slice(&input[flush..split]);
        self.pending = input[split..].to_vec();
        out
    }
}

fn utf8_length(byte: u8) -> usize {
    match byte {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 0,
    }
}

/// Decode a completed OSC payload into a [`Marker`], if interesting.
fn decode_osc(payload: &[u8]) -> Option<Marker> {
    let payload = String::from_utf8_lossy(payload);
    let mut parts = payload.splitn(3, ';');
    let code = parts.next()?;
    match code {
        "133" => {
            let sub = parts.next().unwrap_or("");
            match sub {
                "A" => Some(Marker::PromptStart),
                "B" => Some(Marker::InputStart),
                "C" => Some(Marker::OutputStart),
                "E" => parts
                    .next()
                    .map(|command| Marker::Command(command.to_owned())),
                "D" => {
                    let exit = match parts.next() {
                        None => Some(0),
                        Some(raw) => raw.parse::<i32>().ok(),
                    };
                    Some(Marker::CommandExit(exit))
                }
                _ => None,
            }
        }
        "0" | "2" => parts.next().map(|title| Marker::Title(title.to_owned())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bytes: &[u8]) -> Vec<TextEvent> {
        let mut parser = PlainTextParser::new();
        let mut events = Vec::new();
        parser.feed(bytes, &mut events);
        events
    }

    #[test]
    fn plain_text_and_newlines() {
        let events = run(b"hello\r\nworld\n");
        assert_eq!(
            events,
            vec![
                TextEvent::Print('h'),
                TextEvent::Print('e'),
                TextEvent::Print('l'),
                TextEvent::Print('l'),
                TextEvent::Print('o'),
                TextEvent::Newline,
                TextEvent::Print('w'),
                TextEvent::Print('o'),
                TextEvent::Print('r'),
                TextEvent::Print('l'),
                TextEvent::Print('d'),
                TextEvent::Newline,
            ]
        );
    }

    #[test]
    fn csi_sequences_are_swallowed_and_el_reported() {
        let events = run(b"\x1b[?1049h\x1b[2Kabc\x1b[0J");
        assert_eq!(
            events,
            vec![
                TextEvent::EraseLine,
                TextEvent::Print('a'),
                TextEvent::Print('b'),
                TextEvent::Print('c'),
                TextEvent::EraseLine,
            ]
        );
    }

    #[test]
    fn osc133_markers_with_bel_and_st() {
        let events = run(b"\x1b]133;A\x07x\x1b]133;B\x1b\\y\x1b]133;C\x07\x1b]133;D;42\x07");
        assert_eq!(
            events,
            vec![
                TextEvent::Marker(Marker::PromptStart),
                TextEvent::Print('x'),
                TextEvent::Marker(Marker::InputStart),
                TextEvent::Print('y'),
                TextEvent::Marker(Marker::OutputStart),
                TextEvent::Marker(Marker::CommandExit(Some(42))),
            ]
        );
    }

    #[test]
    fn osc133_without_exit_code_defaults_to_zero() {
        let events = run(b"\x1b]133;D\x07");
        assert_eq!(
            events,
            vec![TextEvent::Marker(Marker::CommandExit(Some(0)))]
        );
    }

    #[test]
    fn sequences_split_across_chunks() {
        let mut parser = PlainTextParser::new();
        let mut events = Vec::new();
        for chunk in [b"\x1b]13".as_slice(), b"3;A\x07", b"ab\x1b[", b"2K"] {
            parser.feed(chunk, &mut events);
        }
        assert_eq!(
            events,
            vec![
                TextEvent::Marker(Marker::PromptStart),
                TextEvent::Print('a'),
                TextEvent::Print('b'),
                TextEvent::EraseLine,
            ]
        );
    }

    #[test]
    fn utf8_split_across_chunks() {
        let mut parser = PlainTextParser::new();
        let mut events = Vec::new();
        let bytes = "héllo → ✓".as_bytes();
        for byte in bytes {
            parser.feed(&[*byte], &mut events);
        }
        let text: String = events
            .into_iter()
            .filter_map(|e| match e {
                TextEvent::Print(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(text, "héllo → ✓");
    }

    #[test]
    fn title_marker_from_osc0() {
        let events = run(b"\x1b]0;my title\x07");
        assert_eq!(
            events,
            vec![TextEvent::Marker(Marker::Title("my title".into()))]
        );
    }

    #[test]
    fn alt_screen_compat_47_and_1047_detected() {
        let events = run(b"\x1b[?47h\x1b[?1047h\x1b[?1047l\x1b[?47l\x1b[?1049h");
        assert_eq!(
            events,
            vec![
                TextEvent::AltScreenCompat(true),
                TextEvent::AltScreenCompat(true),
                TextEvent::AltScreenCompat(false),
                TextEvent::AltScreenCompat(false),
            ]
        );
    }

    #[test]
    fn charset_and_dcs_sequences_do_not_leak_text() {
        let events = run(b"\x1b(B\x1bP1;2q data \x1b\\ok\x1bM\x1b7");
        let text: String = events
            .into_iter()
            .filter_map(|e| match e {
                TextEvent::Print(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(text, "ok");
    }

    #[test]
    fn backspace_and_carriage_return_redraw() {
        let events = run(b"ab\x08c\rXY");
        assert_eq!(
            events,
            vec![
                TextEvent::Print('a'),
                TextEvent::Print('b'),
                TextEvent::Backspace,
                TextEvent::Print('c'),
                TextEvent::LineReset,
                TextEvent::Print('X'),
                TextEvent::Print('Y'),
            ]
        );
    }

    #[test]
    fn crlf_is_single_newline_and_trailing_cr_resolves() {
        let events = run(b"a\r\nb\r");
        assert_eq!(
            events,
            vec![
                TextEvent::Print('a'),
                TextEvent::Newline,
                TextEvent::Print('b'),
            ]
        );
    }

    #[test]
    fn split_crlf_is_not_a_line_erase_and_hidden_text_is_not_exported() {
        let mut parser = PlainTextParser::new();
        let mut events = Vec::new();
        parser.feed(b"kept\r", &mut events);
        parser.feed(b"\n\x1b[8mconcealed", &mut events);
        parser.feed("密文".as_bytes(), &mut events);
        parser.feed(b"\x1b[28m\x1b[38;2;8;0;0mvisible", &mut events);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, TextEvent::Newline))
                .count(),
            1
        );
        assert!(!events.iter().any(|e| matches!(e, TextEvent::LineReset)));
        let text: String = events
            .iter()
            .filter_map(|e| {
                if let TextEvent::Print(c) = e {
                    Some(*c)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(text, "keptvisible");
    }
}
