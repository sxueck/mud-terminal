//! The Phase 2 terminal backend loop.
//!
//! Owns a portable-pty child, feeds its output through the alacritty VTE
//! parser plus the block engine, and publishes immutable snapshots over a
//! bounded tokio channel. All blocking PTY work (spawn/read/write/resize/
//! kill/wait) happens on `spawn_blocking`/background threads; nothing here
//! ever touches a GUI.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::index::Line;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::{Child, ChildKiller, MasterPty, PtySize};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::oneshot;
use tokio::time::MissedTickBehavior;

use crate::blocks::BlockEngine;
use crate::model::{PtyEvent, PtyHandle, Request, Snapshot};
use crate::screen::{TermSize, build_screen};
use crate::shell::{build_launch, detect_shell};
use crate::text::{AltCompatTranslator, PlainTextParser, TextEvent};

/// Initial terminal size until the first `Request::Resize` arrives.
const DEFAULT_COLUMNS: usize = 100;
const DEFAULT_ROWS: usize = 30;
/// Scrolling history kept by the VTE grid (bounded).
const SCROLLING_HISTORY: usize = 10_000;
/// Transcript budget (bytes).
const TRANSCRIPT_CAP: usize = 128 * 1024;
/// Trim slack: only cut the front once we exceed cap + slack, so trimming is
/// amortized O(1) per byte rather than per-byte O(history).
const TRANSCRIPT_SLACK: usize = 16 * 1024;
/// Minimum interval between snapshot rebuilds.
const SNAPSHOT_MIN_INTERVAL: Duration = Duration::from_millis(8);
/// Housekeeping tick: shutdown checks + pending snapshot flushes.
const TICK: Duration = Duration::from_millis(16);
/// Batch limits for one loop iteration.
const MAX_BATCH_MESSAGES: usize = 128;
const MAX_BATCH_BYTES: usize = 512 * 1024;

type WriterSlot = crossbeam_channel::Sender<Vec<u8>>;
type KillerSlot = Arc<Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>>;

/// Commands for the master-owner thread. The PTY master is only `Send`, so a
/// dedicated background thread owns it exclusively; resize calls go through
/// this bounded channel and are answered asynchronously.
enum MasterCommand {
    Resize(PtySize, crossbeam_channel::Sender<Result<(), String>>),
    Stop,
}

#[derive(Clone, Debug)]
struct MasterHandle {
    tx: crossbeam_channel::Sender<MasterCommand>,
}

fn spawn_master_service(master: Box<dyn MasterPty + Send>) -> MasterHandle {
    let (tx, rx) = crossbeam_channel::bounded::<MasterCommand>(16);
    // If the thread fails to spawn, the closure (and thus `rx`) is dropped,
    // which disconnects `tx` so all resize attempts fail cleanly.
    let _spawn_result = std::thread::Builder::new()
        .name("mud-pty-master".to_owned())
        .spawn(move || {
            let master = master;
            while let Ok(command) = rx.recv() {
                match command {
                    MasterCommand::Resize(size, reply) => {
                        let _ = reply.send(master.resize(size).map_err(|err| err.to_string()));
                    }
                    MasterCommand::Stop => {
                        #[cfg(unix)]
                        if let Some(group) = master.process_group_leader().filter(|id| *id > 0) {
                            let _ = nix::sys::signal::killpg(
                                nix::unistd::Pid::from_raw(group),
                                nix::sys::signal::Signal::SIGKILL,
                            );
                        }
                        break;
                    }
                }
            }
            // Dropping `master` here closes the pty once the loop is gone.
        });
    MasterHandle { tx }
}

impl MasterHandle {
    fn stop(&self) {
        let _ = self.tx.try_send(MasterCommand::Stop);
    }

    async fn resize(&self, size: PtySize) -> Result<(), String> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded::<Result<(), String>>(1);
        self.tx
            .send(MasterCommand::Resize(size, reply_tx))
            .map_err(|_| "pty master service stopped".to_owned())?;
        match tokio::task::spawn_blocking(move || reply_rx.recv_timeout(Duration::from_secs(5)))
            .await
        {
            Ok(Ok(result)) => result.map_err(|err| format!("pty resize failed: {err}")),
            Ok(Err(timeout)) => Err(format!("pty resize timed out: {timeout}")),
            Err(err) => Err(format!("pty resize task failed: {err}")),
        }
    }
}

/// Messages delivered to the loop from helper threads/tasks. Payloads carry
/// the stream generation they came from so stale readers/waiters of a killed
/// shell cannot interleave with the respawned one.
enum LoopMsg {
    Bytes(Vec<u8>, u64),
    Request(Request),
    Exited(Option<i32>, u64),
    Eof(u64),
    IoError(String, u64),
    /// Shutdown requested (last handle dropped or request channel closed).
    Shutdown,
}

/// Collects `PtyWrite` responses (DA/CPR/DECRPM/OSC color queries) generated
/// by the alacritty parser while feeding, so the loop can write them back to
/// the PTY afterwards.
#[derive(Clone, Debug, Default)]
struct ResponseCollector {
    pending: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ResponseCollector {
    fn drain(&self) -> Vec<u8> {
        self.pending
            .lock()
            .map(|mut guard| std::mem::take(&mut *guard).concat())
            .unwrap_or_default()
    }
}

impl EventListener for ResponseCollector {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event
            && let Ok(mut pending) = self.pending.lock()
        {
            pending.push(text.into_bytes());
        }
    }
}

struct Spawned {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    reader: Box<dyn std::io::Read + Send>,
    child: Box<dyn Child + Send + Sync>,
    warning: Option<String>,
    /// Keeps the integration temp dir alive until dropped.
    _temp: Option<tempfile::TempDir>,
}

fn spawn_shell(
    directory: &Path,
    shell_override: Option<PathBuf>,
    columns: usize,
    rows: usize,
) -> Result<Spawned, String> {
    let program = detect_shell(shell_override.as_deref());
    let mut launch = build_launch(&program, directory);
    launch.command.env("TERM", "xterm-256color");
    launch.command.env("COLORTERM", "truecolor");
    let pair = portable_pty::native_pty_system()
        .openpty(PtySize {
            rows: rows.clamp(1, u16::MAX as usize) as u16,
            cols: columns.clamp(2, u16::MAX as usize) as u16,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| format!("openpty failed: {err}"))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|err| format!("pty reader failed: {err}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|err| format!("pty writer failed: {err}"))?;
    // Acquire fallible I/O handles before starting a child that would need cleanup.
    let child = pair
        .slave
        .spawn_command(launch.command)
        .map_err(|err| format!("spawning shell '{}' failed: {err}", program.display()))?;
    // The parent's slave handle must not prevent EOF after child exit.
    drop(pair.slave);
    Ok(Spawned {
        master: pair.master,
        writer,
        reader,
        child,
        warning: launch.warning,
        _temp: launch._temp,
    })
}

/// PTY reader thread: blocking reads pushed into the bounded loop channel.
fn spawn_reader(
    reader: Box<dyn std::io::Read + Send>,
    msg_tx: mpsc::Sender<LoopMsg>,
    generation: u64,
) {
    let fallback_tx = msg_tx.clone();
    let result = std::thread::Builder::new()
        .name("mud-pty-reader".to_owned())
        .spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 16384];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(read) => {
                        let chunk = buf[..read].to_vec();
                        if msg_tx
                            .blocking_send(LoopMsg::Bytes(chunk, generation))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = msg_tx.blocking_send(LoopMsg::Eof(generation));
        });
    if result.is_err() {
        // Without a reader the shell output is dead; treat as exit so the UI
        // is not left waiting forever.
        let _ = fallback_tx.try_send(LoopMsg::IoError(
            "PTY reader thread could not start".into(),
            generation,
        ));
        let _ = fallback_tx.try_send(LoopMsg::Eof(generation));
    }
}

/// Child waiter: reports the exit code once, then finishes.
fn spawn_waiter(
    mut child: Box<dyn Child + Send + Sync>,
    msg_tx: mpsc::Sender<LoopMsg>,
    generation: u64,
) -> Box<dyn ChildKiller + Send + Sync> {
    let killer = child.clone_killer();
    tokio::task::spawn_blocking(move || {
        let code = child
            .wait()
            .ok()
            .map(|status| status.exit_code().min(i32::MAX as u32) as i32);
        let _ = msg_tx.blocking_send(LoopMsg::Exited(code, generation));
    });
    killer
}

/// Forwards UI requests (crossbeam, sync side) into the async loop channel.
fn spawn_forwarder(
    req_rx: crossbeam_channel::Receiver<Request>,
    msg_tx: mpsc::Sender<LoopMsg>,
    shutdown: Arc<AtomicBool>,
) {
    let fallback_tx = msg_tx.clone();
    let result = std::thread::Builder::new()
        .name("mud-pty-requests".to_owned())
        .spawn(move || {
            loop {
                crossbeam_channel::select! {
                    recv(req_rx) -> msg => match msg {
                        Ok(request) => {
                            if msg_tx.blocking_send(LoopMsg::Request(request)).is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            // All handles dropped.
                            let _ = msg_tx.blocking_send(LoopMsg::Shutdown);
                            break;
                        }
                    },
                    default(Duration::from_millis(100)) => {
                        if shutdown.load(Ordering::Acquire) {
                            let _ = msg_tx.blocking_send(LoopMsg::Shutdown);
                            break;
                        }
                    }
                }
            }
        });
    if result.is_err() {
        // Without the forwarder no request can ever arrive; shut down.
        let _ = fallback_tx.try_send(LoopMsg::Shutdown);
    }
}

fn spawn_writer(
    mut writer: Box<dyn std::io::Write + Send>,
    msg_tx: mpsc::Sender<LoopMsg>,
    generation: u64,
) -> WriterSlot {
    let (tx, rx) = crossbeam_channel::bounded::<Vec<u8>>(16);
    let _ = std::thread::Builder::new()
        .name("mud-pty-writer".into())
        .spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if let Err(error) = writer.write_all(&bytes) {
                    let _ = msg_tx.blocking_send(LoopMsg::IoError(
                        format!("PTY write failed: {error}"),
                        generation,
                    ));
                    break;
                }
            }
        });
    tx
}

fn kill_child(killer: &KillerSlot) {
    let slot = Arc::clone(killer);
    tokio::task::spawn_blocking(move || {
        if let Ok(mut guard) = slot.lock()
            && let Some(killer) = guard.as_mut()
        {
            let _ = killer.kill();
        }
    });
}

struct LoopState {
    term: Term<ResponseCollector>,
    processor: Processor,
    parser: PlainTextParser,
    alt: AltCompatTranslator,
    engine: BlockEngine,
    listener: ResponseCollector,
    transcript: String,
    warning: Option<String>,
    exited: Option<i32>,
    prompt_ready: bool,
    /// Absolute grid line of the last OSC 133;A prompt, used to crop the live
    /// tail so it never repeats a finished block's output.
    prompt_line: Option<Line>,
    restarting: bool,
    dirty: bool,
    columns: usize,
    rows: usize,
    last_snapshot: Instant,
}

impl LoopState {
    fn new(warning: Option<String>, columns: usize, rows: usize) -> Self {
        let listener = ResponseCollector::default();
        let config = TermConfig {
            scrolling_history: SCROLLING_HISTORY,
            ..TermConfig::default()
        };
        let term = Term::new(config, &TermSize { columns, rows }, listener.clone());
        Self {
            term,
            processor: Processor::new(),
            parser: PlainTextParser::new(),
            alt: AltCompatTranslator::default(),
            engine: BlockEngine::new(),
            listener,
            transcript: String::new(),
            warning,
            exited: None,
            prompt_ready: false,
            prompt_line: None,
            restarting: false,
            dirty: true,
            columns,
            rows,
            last_snapshot: Instant::now(),
        }
    }

    fn reset_for_restart(&mut self, warning: Option<String>) {
        let (columns, rows) = (self.columns, self.rows);
        *self = Self::new(warning, columns, rows);
    }

    fn feed(&mut self, bytes: &[u8]) {
        let bytes = self.alt.feed(bytes);
        self.processor.advance(&mut self.term, &bytes);
        let mut events = Vec::new();
        self.parser.feed(&bytes, &mut events);
        let mut utf8 = [0u8; 4];
        for event in &events {
            match event {
                TextEvent::Marker(marker) => {
                    match marker {
                        crate::text::Marker::PromptStart => {
                            self.prompt_line = Some(self.term.grid().cursor.point.line);
                        }
                        crate::text::Marker::InputStart => self.prompt_ready = true,
                        crate::text::Marker::OutputStart => self.prompt_ready = false,
                        _ => {}
                    }
                    self.engine.marker(marker);
                }
                TextEvent::Print(ch) => {
                    push_transcript(&mut self.transcript, ch.encode_utf8(&mut utf8));
                    self.engine.text(event);
                }
                TextEvent::Newline => {
                    push_transcript(&mut self.transcript, "\n");
                    self.engine.text(event);
                }
                TextEvent::Backspace => {
                    self.transcript.pop();
                    self.engine.text(event);
                }
                TextEvent::EraseLine | TextEvent::LineReset => {
                    if let Some(newline) = self.transcript.rfind('\n') {
                        self.transcript.truncate(newline + 1);
                    } else {
                        self.transcript.clear();
                    }
                    self.engine.text(event);
                }
                TextEvent::AltScreenCompat(_) => {}
            }
        }
        self.dirty = true;
    }

    fn build_snapshot(&self) -> Snapshot {
        Snapshot {
            screen: build_screen(&self.term, self.prompt_line),
            blocks: self.engine.blocks().to_vec(),
            running: self.engine.running(),
            transcript: self.transcript.clone(),
            warning: self.warning.clone(),
            exited: self.exited.is_some(),
            prompt_ready: self.prompt_ready,
        }
    }
}

fn push_transcript(transcript: &mut String, text: &str) {
    transcript.push_str(text);
    if transcript.len() > TRANSCRIPT_CAP + TRANSCRIPT_SLACK {
        let excess = transcript.len() - TRANSCRIPT_CAP;
        let mut cut = excess;
        while cut < transcript.len() && !transcript.is_char_boundary(cut) {
            cut += 1;
        }
        let _ = transcript.drain(..cut);
    }
}

struct Backend {
    state: LoopState,
    master: MasterHandle,
    killer: KillerSlot,
    writer: WriterSlot,
    msg_tx: mpsc::Sender<LoopMsg>,
    events: mpsc::Sender<PtyEvent>,
    pending: Option<PtyEvent>,
    pending_error: Option<String>,
    child_exit: Option<Option<i32>>,
    reader_done: bool,
    directory: PathBuf,
    shell: Option<PathBuf>,
    /// Incremented on every respawn; stale stream messages are ignored.
    generation: u64,
    _integration: Option<tempfile::TempDir>,
}

impl Backend {
    /// Publish a fresh snapshot if needed, then try to flush the pending one.
    /// Returns false when the UI event receiver is gone.
    fn publish(&mut self, force: bool) -> bool {
        if let Some(error) = self.pending_error.take() {
            match self.events.try_send(PtyEvent::Error(error)) {
                Ok(()) => {}
                Err(TrySendError::Full(PtyEvent::Error(error))) => self.pending_error = Some(error),
                Err(TrySendError::Closed(_)) => return false,
                Err(_) => {}
            }
        }
        let due = force || self.state.last_snapshot.elapsed() >= SNAPSHOT_MIN_INTERVAL;
        if self.state.dirty && due {
            let snapshot = Arc::new(self.state.build_snapshot());
            self.state.dirty = false;
            self.state.last_snapshot = Instant::now();
            // Coalescing: a brand-new snapshot replaces any unsent older one.
            self.pending = Some(PtyEvent::Snapshot(snapshot));
        }
        if let Some(event) = self.pending.take() {
            match self.events.try_send(event) {
                Ok(()) => {}
                Err(TrySendError::Full(event)) => self.pending = Some(event),
                Err(TrySendError::Closed(_)) => return false,
            }
        }
        true
    }

    fn queue_error(&mut self, message: String) {
        self.state.warning = Some(message.clone());
        self.state.dirty = true;
        self.pending_error = Some(message);
    }

    fn write_input(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() && self.writer.try_send(bytes).is_err() {
            self.queue_error("PTY input queue is full or closed; input was not delivered".into());
        }
    }

    async fn resize(&mut self, columns: u16, rows: u16) {
        let columns = columns.clamp(2, 1000) as usize;
        let rows = rows.clamp(1, 500) as usize;
        let result = self
            .master
            .resize(PtySize {
                rows: rows.min(u16::MAX as usize) as u16,
                cols: columns.min(u16::MAX as usize) as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .await;
        match result {
            Ok(()) => {
                self.state.term.resize(TermSize { columns, rows });
                self.state.columns = columns;
                self.state.rows = rows;
                self.state.dirty = true;
            }
            Err(err) => self.queue_error(err),
        }
    }

    /// Kill the current shell; the respawn happens once the waiter reports
    /// the exit, keeping kill/wait off the async loop. If the shell already
    /// exited, respawn immediately.
    async fn restart(&mut self) {
        if self.state.restarting {
            return;
        }
        self.state.restarting = true;
        if self.state.exited.is_some() {
            self.state.exited = None;
            self.respawn().await;
        } else {
            self.master.stop();
            kill_child(&self.killer);
        }
    }

    async fn handle_exit(&mut self, code: Option<i32>) {
        if self.state.exited.is_some() && !self.state.restarting {
            return;
        }
        self.state.exited = Some(code.unwrap_or(0));
        self.state.engine.force_finish(code);
        self.state.dirty = true;
        if self.state.restarting {
            self.state.restarting = false;
            self.state.exited = None;
            self.respawn().await;
        }
    }

    async fn respawn(&mut self) {
        self.generation += 1;
        let generation = self.generation;
        let spawn_result = tokio::task::spawn_blocking({
            let directory = self.directory.clone();
            let shell = self.shell.clone();
            let columns = self.state.columns;
            let rows = self.state.rows;
            move || spawn_shell(&directory, shell, columns, rows)
        })
        .await;
        match spawn_result {
            Ok(Ok(spawned)) => {
                spawn_reader(spawned.reader, self.msg_tx.clone(), generation);
                let waiter = spawn_waiter(spawned.child, self.msg_tx.clone(), generation);
                self.killer = Arc::new(Mutex::new(Some(waiter)));
                self.writer = spawn_writer(spawned.writer, self.msg_tx.clone(), generation);
                self.child_exit = None;
                self.reader_done = false;
                self.master = spawn_master_service(spawned.master);
                self._integration = spawned._temp;
                self.state.reset_for_restart(spawned.warning);
                self.state.dirty = true;
            }
            Ok(Err(err)) => {
                self.state.exited = Some(-1);
                self.state.restarting = false;
                self.queue_error(format!("shell restart failed: {err}"));
            }
            Err(err) => {
                self.state.exited = Some(-1);
                self.state.restarting = false;
                self.queue_error(format!("shell restart task failed: {err}"));
            }
        }
    }

    /// Drain a batch of pending messages. Returns false to stop the loop.
    async fn process(&mut self, first: LoopMsg, msg_rx: &mut mpsc::Receiver<LoopMsg>) -> bool {
        let mut output = Vec::new();
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        let mut controls: Vec<LoopMsg> = Vec::new();
        let mut current = Some(first);
        for _ in 0..MAX_BATCH_MESSAGES {
            let msg = match current.take().or_else(|| msg_rx.try_recv().ok()) {
                Some(msg) => msg,
                None => break,
            };
            match msg {
                LoopMsg::Bytes(bytes, generation) => {
                    if generation == self.generation {
                        output.extend_from_slice(&bytes);
                        if output.len() >= MAX_BATCH_BYTES {
                            break;
                        }
                    }
                }
                LoopMsg::Request(Request::Input(bytes)) => inputs.push(bytes),
                other => controls.push(other),
            }
        }

        if !output.is_empty() {
            self.state.feed(&output);
            let responses = self.state.listener.drain();
            if !responses.is_empty() {
                inputs.push(responses);
            }
        }

        for input in inputs {
            self.write_input(input);
        }

        for control in controls {
            match control {
                LoopMsg::Request(Request::Resize { columns, rows }) => {
                    self.resize(columns, rows).await;
                }
                LoopMsg::Request(Request::Clear) => {
                    // Erase display + scrollback + home cursor through the
                    // VTE parser so grid and damage stay consistent, and drop
                    // block history and transcript as well.
                    self.state.feed(b"\x1b[3J\x1b[2J\x1b[H");
                    self.state.transcript.clear();
                    self.state.prompt_line = None;
                    self.state.engine.reset();
                    self.state.dirty = true;
                }
                LoopMsg::Request(Request::Restart) => self.restart().await,
                LoopMsg::Exited(code, generation) if generation == self.generation => {
                    self.child_exit = Some(code)
                }
                LoopMsg::Eof(generation) if generation == self.generation => {
                    self.reader_done = true
                }
                LoopMsg::IoError(error, generation)
                    if generation == self.generation
                        && !self.state.restarting
                        && self.state.exited.is_none() =>
                {
                    self.queue_error(error)
                }
                LoopMsg::Exited(..) | LoopMsg::Eof(_) | LoopMsg::IoError(..) => {}
                // Inputs were diverted to `inputs` above; never panics.
                LoopMsg::Request(Request::Input(_)) | LoopMsg::Bytes(..) => {}
                LoopMsg::Shutdown => return false,
            }
        }
        if self.reader_done
            && let Some(code) = self.child_exit
        {
            self.handle_exit(code).await;
        }
        true
    }
}

/// Run a local terminal backend until the UI drops the event receiver or the
/// last [`PtyHandle`] clone.
///
/// * Spawn errors are reported through `ready` as `Err`.
/// * Runtime errors are delivered as [`PtyEvent::Error`].
/// * Snapshots are coalesced under backpressure: only the newest is kept.
pub async fn run_local(
    directory: PathBuf,
    events: mpsc::Sender<PtyEvent>,
    ready: oneshot::Sender<Result<PtyHandle, String>>,
) {
    run_local_with_shell(directory, None, events, ready).await;
}

/// Like [`run_local`] but with an explicit shell program (tests/tools).
pub async fn run_local_with_shell(
    directory: PathBuf,
    shell: Option<PathBuf>,
    events: mpsc::Sender<PtyEvent>,
    ready: oneshot::Sender<Result<PtyHandle, String>>,
) {
    let (msg_tx, mut msg_rx) = mpsc::channel::<LoopMsg>(1024);
    let (req_tx, req_rx) = crossbeam_channel::bounded::<Request>(64);
    let shutdown = Arc::new(AtomicBool::new(false));
    let handle = PtyHandle::new(req_tx, Arc::clone(&shutdown));

    let spawn_result = tokio::task::spawn_blocking({
        let directory = directory.clone();
        let shell = shell.clone();
        move || spawn_shell(&directory, shell, DEFAULT_COLUMNS, DEFAULT_ROWS)
    })
    .await;

    let spawned = match spawn_result {
        Ok(Ok(spawned)) => spawned,
        Ok(Err(err)) => {
            let _ = ready.send(Err(err));
            return;
        }
        Err(err) => {
            let _ = ready.send(Err(format!("shell spawn task failed: {err}")));
            return;
        }
    };

    spawn_forwarder(req_rx, msg_tx.clone(), Arc::clone(&shutdown));
    spawn_reader(spawned.reader, msg_tx.clone(), 0);
    let waiter = spawn_waiter(spawned.child, msg_tx.clone(), 0);
    let master = spawn_master_service(spawned.master);
    let mut backend = Backend {
        state: LoopState::new(spawned.warning.clone(), DEFAULT_COLUMNS, DEFAULT_ROWS),
        master,
        killer: Arc::new(Mutex::new(Some(waiter))),
        writer: spawn_writer(spawned.writer, msg_tx.clone(), 0),
        msg_tx,
        events,
        pending: None,
        pending_error: None,
        child_exit: None,
        reader_done: false,
        directory,
        shell,
        generation: 0,
        _integration: spawned._temp,
    };

    if ready.send(Ok(handle)).is_err() {
        // UI vanished before the backend started.
        shutdown.store(true, Ordering::Release);
        backend.master.stop();
        kill_child(&backend.killer);
        return;
    }

    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    'event_loop: loop {
        let msg = tokio::select! {
            biased;
            msg = msg_rx.recv() => match msg {
                Some(msg) => msg,
                None => break 'event_loop,
            },
            _ = ticker.tick() => {
                if shutdown.load(Ordering::Acquire) || backend.events.is_closed() {
                    break 'event_loop;
                }
                if !backend.publish(false) {
                    break 'event_loop;
                }
                continue 'event_loop;
            }
        };
        match msg {
            LoopMsg::Shutdown => break 'event_loop,
            first => {
                if !backend.process(first, &mut msg_rx).await {
                    break 'event_loop;
                }
            }
        }
        if !backend.publish(false) || shutdown.load(Ordering::Acquire) {
            break 'event_loop;
        }
    }

    // Teardown: kill the child asynchronously (no join, no GUI); integration
    // temp files are removed when the ShellLaunch handles drop.
    shutdown.store(true, Ordering::Release);
    backend.master.stop();
    if backend.child_exit.is_none() {
        kill_child(&backend.killer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_stays_bounded_amortized() {
        let mut transcript = String::new();
        for i in 0..(TRANSCRIPT_CAP / 64 + 8) {
            push_transcript(&mut transcript, &"a".repeat(60));
            transcript.push('\n');
            let _ = i;
        }
        assert!(transcript.len() <= TRANSCRIPT_CAP + TRANSCRIPT_SLACK);
        assert!(transcript.starts_with('a'));
        assert!(transcript.ends_with('\n'));
    }

    #[test]
    fn transcript_trim_respects_char_boundaries() {
        let mut transcript = String::new();
        let filler = "é".repeat(TRANSCRIPT_CAP); // multi-byte chars
        push_transcript(&mut transcript, &filler);
        assert!(transcript.len() <= TRANSCRIPT_CAP + TRANSCRIPT_SLACK);
        assert!(transcript.chars().all(|c| c == 'é'));
    }

    #[test]
    fn response_collector_drains_in_order() {
        let collector = ResponseCollector::default();
        collector.send_event(Event::PtyWrite("first".to_owned()));
        collector.send_event(Event::PtyWrite("second".to_owned()));
        assert_eq!(collector.drain(), b"firstsecond");
        assert!(collector.drain().is_empty());
    }

    #[test]
    fn loop_state_snapshot_shape() {
        let state = LoopState::new(None, 80, 24);
        let snapshot = state.build_snapshot();
        assert_eq!(snapshot.screen.columns, 80);
        assert_eq!(snapshot.screen.rows, 24);
        assert_eq!(snapshot.screen.cells.len(), 80 * 24);
        assert_eq!(snapshot.screen.mode, RenderMode::Blocks);
        assert!(!snapshot.exited);
        assert!(snapshot.blocks.is_empty());
    }

    use crate::model::RenderMode;
}
