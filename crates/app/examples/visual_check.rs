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

use iced::{Element, Subscription, Task, widget::pane_grid::Axis};
use mud_ui::settings::{Category, Edit};
use shell::{Command, Message as AppMessage, Sidebar};
use std::{fs, io::BufWriter, path::PathBuf, time::Duration};

fn main() -> iced::Result {
    iced::application(Check::boot, Check::update, Check::view)
        .title("mud — Visual check")
        .theme(mud_ui::theme::theme())
        .subscription(Check::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1440.0, 920.0),
            min_size: Some(iced::Size::new(900.0, 640.0)),
            decorations: false,
            transparent: true,
            ..Default::default()
        })
        .default_font(iced::Font::with_name(if cfg!(target_os = "macos") {
            "Helvetica Neue"
        } else {
            "Segoe UI"
        }))
        .run()
}

struct Check {
    app: shell::App,
    stage: usize,
    busy: bool,
    output: PathBuf,
}
#[derive(Debug, Clone)]
enum Message {
    App(AppMessage),
    Tick,
    Captured(iced::window::Screenshot),
    Saved(Result<(), String>),
}
const NAMES: &[&str] = &[
    "01-shell",
    "02-splits",
    "03-context",
    "04-general",
    "05-models",
    "06-autopilot",
    "07-hosts",
    "08-git",
    "09-small-settings",
    "10-palette",
];
impl Check {
    fn boot() -> (Self, Task<Message>) {
        let (app, task) = shell::App::boot();
        let output = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/visual-check"));
        (
            Self {
                app,
                stage: 0,
                busy: false,
                output,
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
            iced::time::every(Duration::from_millis(900)).map(|_| Message::Tick),
        ])
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::App(message) => self.app.update(message).map(Message::App),
            Message::Tick if !self.busy => {
                self.busy = true;
                iced::window::latest()
                    .and_then(iced::window::screenshot)
                    .map(Message::Captured)
            }
            Message::Captured(screenshot) => {
                let path = self.output.join(format!("{}.png", NAMES[self.stage]));
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || save_png(path, screenshot))
                            .await
                            .map_err(|e| e.to_string())?
                    },
                    Message::Saved,
                )
            }
            Message::Saved(result) => {
                if let Err(error) = result {
                    eprintln!("UI_SMOKE_FAILED: {error}");
                    return iced::exit();
                }
                println!("Captured {}", NAMES[self.stage]);
                self.stage += 1;
                self.busy = false;
                let mut commands = Vec::new();
                match self.stage {
                    1 => commands.extend([
                        AppMessage::Split(Axis::Vertical),
                        AppMessage::Split(Axis::Horizontal),
                    ]),
                    2 => commands.extend([
                        AppMessage::Cursor(iced::Point::new(640.0, 210.0)),
                        AppMessage::OpenContext(self.app.workspace.tab().focus),
                    ]),
                    3 => commands
                        .extend([AppMessage::Dismiss, AppMessage::Command(Command::Settings)]),
                    4 => commands.push(AppMessage::Settings(Edit::Category(Category::Models))),
                    5 => commands.push(AppMessage::Settings(Edit::Category(Category::Autopilot))),
                    6 => {
                        commands.extend([AppMessage::Dismiss, AppMessage::Sidebar(Sidebar::Hosts)])
                    }
                    7 => commands.push(AppMessage::Sidebar(Sidebar::Git)),
                    8 => {
                        let task = self.app.update(AppMessage::Command(Command::Settings));
                        return Task::batch([
                            task.map(Message::App),
                            iced::window::latest().and_then(|id| {
                                iced::window::resize(id, iced::Size::new(900.0, 640.0))
                            }),
                        ]);
                    }
                    9 => commands.extend([AppMessage::Dismiss, AppMessage::TogglePalette]),
                    _ => {
                        println!(
                            "UI_SMOKE_OK: {} GPU frames saved to {}",
                            NAMES.len(),
                            self.output.display()
                        );
                        return iced::exit();
                    }
                }
                Task::batch(
                    commands
                        .into_iter()
                        .map(|command| self.app.update(command).map(Message::App)),
                )
            }
            Message::Tick => Task::none(),
        }
    }
}
fn save_png(path: PathBuf, screenshot: iced::window::Screenshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(file),
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
