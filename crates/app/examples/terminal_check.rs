#[path = "../src/files.rs"]
mod files;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/shell.rs"]
mod shell;
#[path = "../src/terminal_view.rs"]
mod terminal_view;
#[path = "../src/terminals.rs"]
mod terminals;
#[path = "../src/workspace.rs"]
mod workspace;

use iced::{Element, Subscription, Task};
use mud_terminal::{RenderMode, Snapshot};
use shell::{Command, Message as AppMessage};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

static FAILED: AtomicBool = AtomicBool::new(false);

fn main() -> iced::Result {
    iced::application(Check::boot, Check::update, Check::view)
        .title("mud — Phase 2 PTY/TUI check")
        .theme(mud_ui::theme::theme())
        .subscription(Check::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1200.0, 800.0),
            min_size: Some(iced::Size::new(900.0, 640.0)),
            decorations: false,
            transparent: true,
            ..Default::default()
        })
        .run()?;
    assert!(
        !FAILED.load(Ordering::Relaxed),
        "native terminal check failed"
    );
    Ok(())
}

struct Check {
    app: shell::App,
    session: u64,
    tab: u64,
    stage: usize,
    entered: Instant,
    output: PathBuf,
    busy: bool,
}

#[derive(Debug, Clone)]
enum Message {
    App(AppMessage),
    Tick,
    Captured(iced::window::Screenshot),
    Saved(Result<(), String>),
}

impl Check {
    fn boot() -> (Self, Task<Message>) {
        let (app, task) = shell::App::boot();
        let session = terminals::shell_ids(&app.workspace)
            .next()
            .map(|(id, _)| id)
            .expect("initial local shell");
        let tab = app.workspace.tab().id;
        let output = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| "target/terminal-check".into());
        (
            Self {
                app,
                session,
                tab,
                stage: 0,
                entered: Instant::now(),
                output,
                busy: false,
            },
            task.map(Message::App),
        )
    }

    fn view(&self) -> Element<'_, Message> {
        self.app.view().map(Message::App)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            self.app.subscription().map(Message::App),
            iced::time::every(Duration::from_millis(150)).map(|_| Message::Tick),
        ])
    }

    fn snapshot(&self) -> Option<Arc<Snapshot>> {
        self.app
            .terminals
            .live
            .get(&self.session)
            .and_then(|live| live.snapshot.clone())
    }

    fn app(&mut self, message: AppMessage) -> Task<Message> {
        self.app.update(message).map(Message::App)
    }

    fn input(&mut self, bytes: &[u8]) -> Task<Message> {
        self.app(AppMessage::Terminal(
            self.session,
            terminals::Event::Grid(terminal_view::Action::Input(bytes.to_vec())),
        ))
    }

    fn advance(&mut self) {
        self.stage += 1;
        self.entered = Instant::now();
    }

    fn fail(&self, error: impl std::fmt::Display) -> Task<Message> {
        eprintln!("TERMINAL_CHECK_FAILED stage {}: {error}", self.stage);
        if let Some(snapshot) = self.snapshot() {
            eprintln!(
                "mode={:?} blocks={:?} transcript={:?} warning={:?}",
                snapshot.screen.mode, snapshot.blocks, snapshot.transcript, snapshot.warning
            );
        }
        FAILED.store(true, Ordering::Relaxed);
        iced::exit()
    }

    fn capture(&mut self) -> Task<Message> {
        self.busy = true;
        iced::window::latest()
            .and_then(iced::window::screenshot)
            .map(Message::Captured)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // Keep the scripted check deterministic if the foreground window receives typing.
            Message::App(AppMessage::Terminal(
                _,
                terminals::Event::Grid(terminal_view::Action::Input(_)),
            )) => Task::none(),
            Message::App(message) => self.app(message),
            Message::Captured(screenshot) => {
                let path = self.output.join(format!("{:02}.png", self.stage));
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || save_png(&path, &screenshot))
                            .await
                            .map_err(|e| e.to_string())?
                    },
                    Message::Saved,
                )
            }
            Message::Saved(Err(error)) => self.fail(error),
            Message::Saved(Ok(())) => {
                self.busy = false;
                self.advance();
                Task::none()
            }
            Message::Tick if self.busy => Task::none(),
            Message::Tick => {
                if self.entered.elapsed() > Duration::from_secs(25) {
                    return self.fail("timed out waiting for real PTY state");
                }
                let Some(snapshot) = self.snapshot() else {
                    return Task::none();
                };
                if let Some(error) = self
                    .app
                    .terminals
                    .live
                    .get(&self.session)
                    .and_then(|s| s.error.as_ref())
                {
                    return self.fail(error);
                }
                match self.stage {
                    0 if snapshot.prompt_ready => {
                        self.advance();
                        self.input(b"printf '\\033[38;2;10;100;160mPHASE2-BLOCK\\033[0m\\n'\r")
                    }
                    1 if snapshot
                        .blocks
                        .iter()
                        .any(|b| b.exit_code == Some(0) && b.output.contains("PHASE2-BLOCK")) =>
                    {
                        self.capture()
                    }
                    2 => {
                        self.advance();
                        let open = self.app(AppMessage::Command(Command::FindScrollback));
                        let query = self.app(AppMessage::Terminal(
                            self.session,
                            terminals::Event::Query("PHASE2-BLOCK".into()),
                        ));
                        Task::batch([open, query])
                    }
                    3 if self.entered.elapsed() > Duration::from_millis(250) => self.capture(),
                    4 => {
                        self.advance();
                        let dismiss = self.app(AppMessage::Dismiss);
                        let clear = self.app(AppMessage::Command(Command::ClearTerminal));
                        Task::batch([dismiss, clear])
                    }
                    5 if snapshot.blocks.is_empty()
                        && !snapshot.transcript.contains("PHASE2-BLOCK") =>
                    {
                        self.advance();
                        self.input(b"echo PHASE2-SCROLLBACK\r")
                    }
                    6 if snapshot.blocks.iter().any(|b| {
                        b.exit_code == Some(0) && b.output.contains("PHASE2-SCROLLBACK")
                    }) =>
                    {
                        self.advance();
                        self.app(AppMessage::Command(Command::Scrollback))
                    }
                    7 => {
                        if !matches!(self.app.workspace.tab().focused(), Some(workspace::PaneBody::Editor { document, .. }) if document.text().contains("PHASE2-SCROLLBACK"))
                        {
                            return self.fail("scrollback editor does not contain live PTY output");
                        }
                        self.capture()
                    }
                    8 => {
                        self.advance();
                        let tab = self.app(AppMessage::SelectTab(self.tab));
                        let restart = self.app(AppMessage::Command(Command::RestartShell));
                        Task::batch([tab, restart])
                    }
                    9 if snapshot.blocks.is_empty()
                        && snapshot.prompt_ready
                        && !snapshot.exited =>
                    {
                        self.advance();
                        self.input(b"vim -Nu NONE -n\r")
                    }
                    10 if snapshot.screen.mode == RenderMode::Fullscreen
                        && self.entered.elapsed() > Duration::from_millis(500) =>
                    {
                        self.advance();
                        self.input(b"iPHASE2-TUI-RAW-INPUT\x1b")
                    }
                    11 if snapshot.screen.mode == RenderMode::Fullscreen
                        && snapshot
                            .screen
                            .cells
                            .iter()
                            .map(|cell| cell.text.as_str())
                            .collect::<String>()
                            .contains("PHASE2-TUI-RAW-INPUT") =>
                    {
                        self.capture()
                    }
                    12 => {
                        self.advance();
                        self.input(b":q!\r")
                    }
                    13 if snapshot.screen.mode == RenderMode::Blocks
                        && snapshot
                            .blocks
                            .iter()
                            .any(|b| b.command.contains("vim") && b.exit_code == Some(0)) =>
                    {
                        self.capture()
                    }
                    14 => {
                        println!(
                            "TERMINAL_CHECK_OK: real PTY → Blocks → search → clear → scrollback editor → restart → vim fullscreen/raw input → Blocks; GPU frames: {}",
                            self.output.display()
                        );
                        iced::exit()
                    }
                    _ => Task::none(),
                }
            }
        }
    }
}

fn save_png(path: &std::path::Path, screenshot: &iced::window::Screenshot) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("no screenshot directory")?)
        .map_err(|e| e.to_string())?;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        screenshot.size.width,
        screenshot.size.height,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&screenshot.rgba)
        .map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}
