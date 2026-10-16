//! Warp-style block engine driven by OSC 133 shell integration markers.
//!
//! Tracks prompt → input → output → exit transitions and produces bounded
//! [`Block`] history. Command text is reconstructed from the echoed input
//! between the B and C markers; output is captured between C and D with
//! head/tail truncation so a single runaway command cannot exhaust memory.

use std::{sync::Arc, time::Instant};

use crate::model::Block;
use crate::text::{Marker, TextEvent};

/// Keep at most this many finished blocks.
const MAX_BLOCKS: usize = 200;
/// Keep at most this many bytes of output per block (head + tail).
const MAX_BLOCK_OUTPUT: usize = 16 * 1024;

#[derive(Debug)]
enum Phase {
    /// No prompt seen yet (before shell integration kicks in).
    Idle,
    /// Between A and B: the prompt is being drawn.
    Prompt,
    /// Between B and C: the user is typing / recalling a command.
    Input { segments: Vec<String> },
    /// Between C and D: the command is running.
    Running {
        command: String,
        started: Instant,
        output: BoundedOutput,
    },
}

#[derive(Clone, Debug, Default)]
struct BoundedOutput {
    head: String,
    tail: String,
    line: String,
    tail_cap: usize,
    dropped: u64,
}

impl BoundedOutput {
    fn new() -> Self {
        Self {
            head: String::new(),
            tail: String::new(),
            line: String::new(),
            tail_cap: MAX_BLOCK_OUTPUT / 2,
            dropped: 0,
        }
    }

    fn push_str(&mut self, text: &str) {
        let mut cut = (MAX_BLOCK_OUTPUT / 2 - self.head.len()).min(text.len());
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        self.head.push_str(&text[..cut]);
        self.tail.push_str(&text[cut..]);
        // Trim in chunks rather than moving the tail for every output byte.
        if self.tail.len() > self.tail_cap + 2048 {
            let mut boundary = self.tail.len() - self.tail_cap;
            while !self.tail.is_char_boundary(boundary) {
                boundary += 1;
            }
            let _ = self.tail.drain(..boundary);
            self.dropped += boundary as u64;
        }
    }

    fn text(&mut self, event: &TextEvent) {
        match event {
            TextEvent::Print(c) => {
                self.line.push(*c);
                // A redrawable line is capped; very long lines spill into bounded history.
                if self.line.len() >= 8192 {
                    let line = std::mem::take(&mut self.line);
                    self.push_str(&line);
                }
            }
            TextEvent::Newline => {
                let line = std::mem::take(&mut self.line);
                self.push_str(&line);
                self.push_str("\n");
            }
            TextEvent::Backspace => {
                self.line.pop();
            }
            TextEvent::EraseLine | TextEvent::LineReset => self.line.clear(),
            _ => {}
        }
    }

    fn finish(mut self) -> String {
        let line = std::mem::take(&mut self.line);
        self.push_str(line.trim_end());
        if self.dropped == 0 {
            return self.head + &self.tail;
        }
        let mut out = self.head;
        out.push_str("\n…[terminal output truncated: ");
        out.push_str(&self.dropped.to_string());
        out.push_str(" bytes dropped]…\n");
        out.push_str(&self.tail);
        out
    }
}

/// Tracks the OSC 133 lifecycle and finished block history.
#[derive(Debug)]
pub struct BlockEngine {
    blocks: Vec<Arc<Block>>,
    next_id: u64,
    phase: Phase,
    dirty: bool,
}

impl Default for BlockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockEngine {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            next_id: 0,
            phase: Phase::Idle,
            dirty: false,
        }
    }

    pub fn blocks(&self) -> &[Arc<Block>] {
        &self.blocks
    }

    pub fn running(&self) -> Option<Block> {
        if let Phase::Running {
            command,
            started,
            output,
        } = &self.phase
        {
            Some(Block {
                id: self.next_id,
                command: command.clone(),
                output: output.clone().finish(),
                exit_code: None,
                duration: Some(started.elapsed()),
            })
        } else {
            None
        }
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// True while a command is between its C and D markers.
    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    pub fn reset(&mut self) {
        self.blocks.clear();
        self.phase = Phase::Idle;
        self.dirty = true;
    }

    pub fn marker(&mut self, marker: &Marker) {
        match marker {
            Marker::PromptStart => {
                self.close_running(None);
                self.phase = Phase::Prompt;
                self.dirty = true;
            }
            Marker::InputStart => {
                if !matches!(self.phase, Phase::Running { .. }) {
                    self.phase = Phase::Input {
                        segments: vec![String::new()],
                    };
                    self.dirty = true;
                }
            }
            Marker::OutputStart => {
                if matches!(self.phase, Phase::Running { .. }) {
                    return;
                }
                let previous = std::mem::replace(&mut self.phase, Phase::Idle);
                if let Phase::Input { segments } = previous {
                    let command = join_segments(segments);
                    self.phase = Phase::Running {
                        command,
                        started: Instant::now(),
                        output: BoundedOutput::new(),
                    };
                    self.dirty = true;
                } else if matches!(previous, Phase::Prompt) {
                    // Integration without a B marker: still time the command.
                    self.phase = Phase::Running {
                        command: String::new(),
                        started: Instant::now(),
                        output: BoundedOutput::new(),
                    };
                    self.dirty = true;
                }
            }
            Marker::CommandExit(code) => {
                if self.close_running(*code) {
                    self.phase = Phase::Prompt;
                    self.dirty = true;
                }
            }
            Marker::Command(command) => {
                if !self.is_running() {
                    self.phase = Phase::Input {
                        segments: vec![command.clone()],
                    };
                }
            }
            Marker::Title(_) => {}
        }
    }

    pub fn text(&mut self, event: &TextEvent) {
        match &mut self.phase {
            Phase::Input { segments } => {
                let segment_count = segments.len();
                let last = match segments.last_mut() {
                    Some(last) => last,
                    None => return,
                };
                match event {
                    TextEvent::Print(c) if last.len() < 8192 => last.push(*c),
                    TextEvent::Print(_) => {}
                    TextEvent::Newline if segment_count < 128 => segments.push(String::new()),
                    TextEvent::Newline => {}
                    TextEvent::Backspace => {
                        last.pop();
                    }
                    TextEvent::EraseLine | TextEvent::LineReset => last.clear(),
                    TextEvent::Marker(_) | TextEvent::AltScreenCompat(_) => {}
                }
            }
            Phase::Running { output, .. } => output.text(event),
            Phase::Idle | Phase::Prompt => {}
        }
    }

    fn close_running(&mut self, exit_code: Option<i32>) -> bool {
        if let Phase::Running {
            command,
            started,
            output,
        } = std::mem::replace(&mut self.phase, Phase::Idle)
        {
            let block = Block {
                id: self.next_id,
                command,
                output: output.finish(),
                exit_code,
                duration: Some(started.elapsed()),
            };
            self.next_id += 1;
            self.blocks.push(Arc::new(block));
            if self.blocks.len() > MAX_BLOCKS {
                self.blocks.drain(..self.blocks.len() - MAX_BLOCKS);
            }
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn force_finish(&mut self, exit_code: Option<i32>) {
        self.close_running(exit_code);
        self.phase = Phase::Idle;
    }
}

fn join_segments(segments: Vec<String>) -> String {
    let trimmed: Vec<String> = segments.into_iter().map(|s| s.trim().to_owned()).collect();
    let mut command = trimmed.join(" ").trim().to_owned();
    if command.len() > 8192 {
        let mut end = 8192;
        while !command.is_char_boundary(end) {
            end -= 1;
        }
        command.truncate(end);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::PlainTextParser;

    fn feed(engine: &mut BlockEngine, parser: &mut PlainTextParser, bytes: &[u8]) {
        let mut events = Vec::new();
        parser.feed(bytes, &mut events);
        for event in &events {
            match event {
                TextEvent::Marker(m) => engine.marker(m),
                other => engine.text(other),
            }
        }
    }

    #[test]
    fn full_command_lifecycle() {
        let mut engine = BlockEngine::new();
        let mut parser = PlainTextParser::new();
        feed(&mut engine, &mut parser, b"\x1b]133;D;0\x07\x1b]133;A\x07");
        feed(&mut engine, &mut parser, b"user@host $ \x1b]133;B\x07");
        feed(&mut engine, &mut parser, b"echo hi");
        feed(
            &mut engine,
            &mut parser,
            b"\x1b]133;C\x07hi\r\n\x1b]133;D;0\x07",
        );
        assert_eq!(engine.blocks().len(), 1);
        let block = &engine.blocks()[0];
        assert_eq!(block.command, "echo hi");
        assert_eq!(block.output, "hi\n");
        assert_eq!(block.exit_code, Some(0));
        assert!(block.duration.is_some());
    }

    #[test]
    fn exit_code_failure_propagates() {
        let mut engine = BlockEngine::new();
        let mut parser = PlainTextParser::new();
        feed(
            &mut engine,
            &mut parser,
            b"\x1b]133;A\x07\x1b]133;B\x07false\x1b]133;C\x07\x1b]133;D;1\x07",
        );
        assert_eq!(engine.blocks()[0].exit_code, Some(1));
    }

    #[test]
    fn backspace_and_redraw_shape_command() {
        let mut engine = BlockEngine::new();
        let mut parser = PlainTextParser::new();
        feed(&mut engine, &mut parser, b"\x1b]133;A\x07\x1b]133;B\x07");
        feed(&mut engine, &mut parser, b"ekk\x08\x08cho hi");
        feed(&mut engine, &mut parser, b"\r\x1b[Kls -la");
        feed(
            &mut engine,
            &mut parser,
            b"\x1b]133;C\x07out\x1b]133;D;0\x07",
        );
        assert_eq!(engine.blocks()[0].command, "ls -la");
    }

    #[test]
    fn output_is_bounded_head_and_tail() {
        let mut engine = BlockEngine::new();
        let mut parser = PlainTextParser::new();
        feed(
            &mut engine,
            &mut parser,
            b"\x1b]133;A\x07\x1b]133;B\x07yes\x1b]133;C\x07",
        );
        let big_line = "x".repeat(64 * 1024);
        feed(
            &mut engine,
            &mut parser,
            format!("{big_line}\r\n").as_bytes(),
        );
        feed(&mut engine, &mut parser, b"END\r\n\x1b]133;D;0\x07");
        let block = &engine.blocks()[0];
        assert!(
            block.output.len() < 20 * 1024,
            "output should stay bounded: {}",
            block.output.len()
        );
        assert!(block.output.contains("truncated"));
        assert!(block.output.ends_with("END\n"));
    }

    #[test]
    fn block_count_is_bounded() {
        let mut engine = BlockEngine::new();
        let mut parser = PlainTextParser::new();
        for i in 0..(MAX_BLOCKS + 50) {
            feed(
                &mut engine,
                &mut parser,
                format!("\x1b]133;A\x07\x1b]133;B\x07c{i}\x1b]133;C\x07o\x1b]133;D;0\x07")
                    .as_bytes(),
            );
        }
        assert_eq!(engine.blocks().len(), MAX_BLOCKS);
        assert_eq!(
            engine.blocks().last().map(|b| b.command.as_str()),
            Some("c249")
        );
    }

    #[test]
    fn stray_markers_without_block_are_ignored() {
        let mut engine = BlockEngine::new();
        engine.marker(&Marker::CommandExit(Some(3)));
        engine.marker(&Marker::OutputStart);
        assert!(engine.blocks().is_empty());
        assert!(!engine.is_running());
    }

    #[test]
    fn restart_force_finishes_running_command() {
        let mut engine = BlockEngine::new();
        let mut parser = PlainTextParser::new();
        feed(
            &mut engine,
            &mut parser,
            b"\x1b]133;A\x07\x1b]133;B\x07sleep\x1b]133;C\x07",
        );
        assert!(engine.is_running());
        engine.force_finish(Some(9));
        assert_eq!(engine.blocks().len(), 1);
        assert_eq!(engine.blocks()[0].exit_code, Some(9));
        assert!(!engine.is_running());
    }

    #[test]
    fn multiline_command_joins_segments() {
        let mut engine = BlockEngine::new();
        let mut parser = PlainTextParser::new();
        feed(
            &mut engine,
            &mut parser,
            b"\x1b]133;A\x07\x1b]133;B\x07echo one\necho two\x1b]133;C\x07\x1b]133;D;0\x07",
        );
        assert_eq!(engine.blocks()[0].command, "echo one echo two");
    }
}
