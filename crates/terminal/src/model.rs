//! Phase 2 backend data model exposed to the UI layer.
//!
//! These types are the coordinate interface consumed by the app crate. The
//! terminal backend produces [`Snapshot`] values behind an `Arc` and the UI
//! only ever clones the `Arc`.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// Warp-style blocks: prompt, command, output, exit code.
    Blocks,
    /// Alternate-screen full TUI takeover.
    Fullscreen,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub row: usize,
    pub column: usize,
    pub text: String,
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub wide_spacer: bool,
}

#[derive(Clone, Debug)]
pub struct Screen {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    /// `(column, row)` when the cursor is visible.
    pub cursor: Option<(usize, usize)>,
    /// Rows at the top of [`Screen::cells`] that a completed block already
    /// renders. The Blocks view crops them so the live tail never repeats
    /// block output; always `0` in [`RenderMode::Fullscreen`].
    pub hidden_rows: usize,
    /// Rows from [`Screen::hidden_rows`] down to the last row that holds text
    /// or the cursor. The Blocks view sizes the live tail with this so empty
    /// screen rows cannot push the prompt out of a scrolling document.
    pub live_rows: usize,
    pub mode: RenderMode,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub mouse_tracking: bool,
    pub mouse_motion: bool,
    pub mouse_drag: bool,
    pub mouse_sgr: bool,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub id: u64,
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub duration: Option<Duration>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub screen: Screen,
    pub blocks: Vec<Arc<Block>>,
    pub running: Option<Block>,
    pub transcript: String,
    pub warning: Option<String>,
    pub exited: bool,
    pub prompt_ready: bool,
}

#[derive(Clone)]
pub enum Request {
    Input(Vec<u8>),
    Resize {
        columns: u16,
        rows: u16,
    },
    /// Clear screen, scrollback, transcript and blocks.
    Clear,
    Restart,
}

#[derive(Clone, Debug)]
pub enum PtyEvent {
    Snapshot(Arc<Snapshot>),
    Error(String),
}

#[derive(Debug)]
struct HandleState {
    requests: crossbeam_channel::Sender<Request>,
    shutdown: Arc<AtomicBool>,
}

impl Drop for HandleState {
    fn drop(&mut self) {
        // Last handle clone gone: ask the loop to shut the process down.
        self.shutdown.store(true, Ordering::Release);
    }
}

#[derive(Clone, Debug)]
pub struct PtyHandle {
    state: Arc<HandleState>,
}

impl PtyHandle {
    pub(crate) fn new(
        requests: crossbeam_channel::Sender<Request>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            state: Arc::new(HandleState { requests, shutdown }),
        }
    }

    /// Queue a request for the backend without ever blocking the caller.
    pub fn send(&self, request: Request) -> Result<(), String> {
        use crossbeam_channel::TrySendError;
        if matches!(&request, Request::Input(bytes) if bytes.len() > 1024 * 1024) {
            return Err("Paste exceeds the 1 MiB input limit; nothing was sent".into());
        }
        self.state
            .requests
            .try_send(request)
            .map_err(|err| match err {
                TrySendError::Full(_) => "terminal request queue is full".to_owned(),
                TrySendError::Disconnected(_) => "terminal backend unavailable".to_owned(),
            })
    }

    pub fn is_dropped(&self) -> bool {
        self.state.shutdown.load(Ordering::Acquire)
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Input(bytes) => write!(f, "Input({} bytes)", bytes.len()),
            Request::Resize { columns, rows } => {
                write!(f, "Resize {{ columns: {columns}, rows: {rows} }}")
            }
            Request::Clear => f.write_str("Clear"),
            Request::Restart => f.write_str("Restart"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturated_request_queue_returns_without_waiting() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let handle = PtyHandle::new(tx, Arc::new(AtomicBool::new(false)));
        handle.send(Request::Clear).expect("first request");
        assert_eq!(
            handle
                .send(Request::Restart)
                .expect_err("must not wait for space"),
            "terminal request queue is full"
        );
    }

    #[test]
    fn oversized_paste_is_rejected_atomically_and_debug_redacts_input() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let handle = PtyHandle::new(tx, Arc::new(AtomicBool::new(false)));
        assert!(
            handle
                .send(Request::Input(vec![b'x'; 1024 * 1024 + 1]))
                .is_err()
        );
        assert!(rx.is_empty());
        assert_eq!(
            format!("{:?}", Request::Input(b"private".to_vec())),
            "Input(7 bytes)"
        );
    }
}
