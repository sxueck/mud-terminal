use crate::theme::INK;
use iced::{
    Color,
    widget::{Svg, svg},
};

#[derive(Debug, Clone, Copy)]
pub enum Icon {
    Terminal,
    Folder,
    File,
    ChevronRight,
    ChevronDown,
    Plus,
    Close,
    SplitRight,
    SplitDown,
    Search,
    Command,
    Bell,
    Settings,
    Server,
    Git,
    Copy,
    Paste,
    Move,
    Restart,
    Trash,
    Check,
    Home,
    Model,
    Shield,
    Maximize,
    Minimize,
    Panel,
}

pub fn icon<'a>(kind: Icon, size: f32) -> Svg<'a> {
    tinted(kind, size, INK)
}
pub fn tinted<'a>(kind: Icon, size: f32, color: Color) -> Svg<'a> {
    let bytes: &'static [u8] = match kind {
        Icon::Terminal => include_bytes!("../assets/icons/terminal.svg"),
        Icon::Folder => include_bytes!("../assets/icons/folder.svg"),
        Icon::File => include_bytes!("../assets/icons/file-text.svg"),
        Icon::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
        Icon::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
        Icon::Plus => include_bytes!("../assets/icons/plus.svg"),
        Icon::Close => include_bytes!("../assets/icons/x.svg"),
        Icon::SplitRight => include_bytes!("../assets/icons/columns-2.svg"),
        Icon::SplitDown => include_bytes!("../assets/icons/rows-2.svg"),
        Icon::Search => include_bytes!("../assets/icons/search.svg"),
        Icon::Command => include_bytes!("../assets/icons/slash.svg"),
        Icon::Bell => include_bytes!("../assets/icons/bell.svg"),
        Icon::Settings => include_bytes!("../assets/icons/settings-2.svg"),
        Icon::Server => include_bytes!("../assets/icons/server.svg"),
        Icon::Git => include_bytes!("../assets/icons/git-branch.svg"),
        Icon::Copy => include_bytes!("../assets/icons/copy.svg"),
        Icon::Paste => include_bytes!("../assets/icons/clipboard.svg"),
        Icon::Move => include_bytes!("../assets/icons/arrow-left-to-line.svg"),
        Icon::Restart => include_bytes!("../assets/icons/rotate-ccw.svg"),
        Icon::Trash => include_bytes!("../assets/icons/trash-2.svg"),
        Icon::Check => include_bytes!("../assets/icons/check.svg"),
        Icon::Home => include_bytes!("../assets/icons/house.svg"),
        Icon::Model => include_bytes!("../assets/icons/cpu.svg"),
        Icon::Shield => include_bytes!("../assets/icons/shield.svg"),
        Icon::Maximize => include_bytes!("../assets/icons/maximize-2.svg"),
        Icon::Minimize => include_bytes!("../assets/icons/minus.svg"),
        Icon::Panel => include_bytes!("../assets/icons/panel-left.svg"),
    };
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
}
