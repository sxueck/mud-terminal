mod files;
mod settings;
mod shell;
mod workspace;

fn main() -> iced::Result {
    iced::application(shell::App::boot, shell::App::update, shell::App::view)
        .title("mud — Native workspace")
        .theme(iced::Theme::Light)
        .subscription(shell::App::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1440.0, 920.0),
            min_size: Some(iced::Size::new(900.0, 640.0)),
            decorations: false,
            ..Default::default()
        })
        .default_font(iced::Font::with_name(if cfg!(target_os = "macos") {
            "Helvetica Neue"
        } else {
            "Segoe UI"
        }))
        .run()
}
