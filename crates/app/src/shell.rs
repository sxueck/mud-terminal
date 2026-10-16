use crate::{
    files::{self, Listing},
    settings::{self, Dialog, Edit, Preferences},
    workspace::{PaneBody, Workspace},
};
use iced::{
    Element, Event, Fill, Font, Point, Subscription, Task,
    widget::{
        self, Space, button, column, container, mouse_area,
        pane_grid::{self, Axis, Pane},
        row, scrollable, stack, text, text_input,
    },
};
use mud_agent::LaunchKind;
use mud_editor::Document;
use mud_ssh::Host;
use mud_terminal::SessionTarget;
use mud_ui::{self as ui, Icon, theme};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sidebar {
    Files,
    Hosts,
    Git,
}
#[derive(Debug, Clone, Copy)]
pub enum Command {
    NewTab,
    NewAgent,
    SplitRight,
    SplitDown,
    MoveToTab,
    Scrollback,
    ClosePane,
    Settings,
    ToggleSidebar,
}
#[derive(Debug, Clone, Copy)]
pub enum HostField {
    Name,
    Address,
    User,
    Port,
}
#[derive(Debug, Clone)]
pub enum Message {
    PreferencesLoaded(Result<Preferences, String>),
    DirectoryLoaded(PathBuf, Result<Listing, String>),
    PreviewLoaded(PathBuf, SessionTarget, Result<String, String>),
    SelectTab(u64),
    CloseTab(u64),
    Focus(Pane),
    Split(Axis),
    Resize(pane_grid::ResizeEvent),
    Drag(pane_grid::DragEvent),
    MaximizePane(Pane),
    ClosePane(Pane),
    Input(Pane, String),
    Paste,
    Pasted(u64, Pane, Option<String>),
    Command(Command),
    OpenContext(Pane),
    Cursor(Point),
    Dismiss,
    ToggleSidebar,
    Sidebar(Sidebar),
    ToggleDirectory(PathBuf),
    OpenFile(PathBuf),
    Filter(String),
    ToggleSearch,
    TogglePalette,
    ToggleProfile,
    SetRoot(PathBuf),
    Settings(Edit),
    Persisted(Result<(), String>, Preferences),
    HostField(HostField, String),
    SaveHost,
    RemoveHost(usize),
    Notify,
    ClearNotice,
    DragWindow,
    Minimize,
    Maximize,
    Quit,
}

#[derive(Debug)]
pub struct App {
    pub workspace: Workspace,
    project_root: PathBuf,
    preferences: Preferences,
    preferences_loaded: bool,
    settings: Option<Dialog>,
    sidebar: Sidebar,
    sidebar_visible: bool,
    listings: BTreeMap<PathBuf, Result<Listing, String>>,
    expanded: BTreeSet<PathBuf>,
    loading: BTreeSet<PathBuf>,
    file_query: String,
    search_visible: bool,
    palette: bool,
    profile: bool,
    context: Option<Point>,
    cursor: Point,
    notice: Option<String>,
    host_name: String,
    host_address: String,
    host_user: String,
    host_port: String,
    saving: bool,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("Background task failed: {e}"))?
}
impl App {
    pub fn boot() -> (Self, Task<Message>) {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut app = Self::new(root.clone());
        app.loading.insert(root.clone());
        let preferences = Task::perform(
            blocking(|| {
                settings::config_path()
                    .ok_or_else(|| "Cannot locate your settings directory.".into())
                    .and_then(|path| settings::load(&path))
            }),
            Message::PreferencesLoaded,
        );
        (app, Task::batch([preferences, load_directory(root)]))
    }
    fn new(root: PathBuf) -> Self {
        Self {
            project_root: root.clone(),
            workspace: Workspace::new(root),
            preferences: Preferences::default(),
            preferences_loaded: false,
            settings: None,
            sidebar: Sidebar::Files,
            sidebar_visible: true,
            listings: BTreeMap::new(),
            expanded: BTreeSet::new(),
            loading: BTreeSet::new(),
            file_query: String::new(),
            search_visible: false,
            palette: false,
            profile: false,
            context: None,
            cursor: Point::ORIGIN,
            notice: None,
            host_name: String::new(),
            host_address: String::new(),
            host_user: String::new(),
            host_port: "22".into(),
            saving: false,
        }
    }
    pub fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(|event, status, _| match event {
            Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                Some(Message::Cursor(position))
            }
            Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                use iced::keyboard::{Key, key::Named};
                if key == Key::Named(Named::Escape) {
                    return Some(Message::Dismiss);
                }
                if !modifiers.command()
                    || modifiers.alt()
                    || (cfg!(target_os = "macos") && modifiers.control())
                {
                    return None;
                }
                match key.as_ref() {
                    Key::Character("d") if modifiers.shift() => {
                        Some(Message::Split(Axis::Horizontal))
                    }
                    Key::Character("d") => Some(Message::Split(Axis::Vertical)),
                    Key::Character("t") => Some(Message::Command(Command::NewTab)),
                    Key::Character("a") if modifiers.shift() => {
                        Some(Message::Command(Command::NewAgent))
                    }
                    Key::Character("w") => Some(Message::Command(Command::ClosePane)),
                    Key::Character(",") => Some(Message::Command(Command::Settings)),
                    Key::Character("p") if modifiers.shift() => Some(Message::TogglePalette),
                    Key::Character("b") => Some(Message::ToggleSidebar),
                    Key::Character("q") => Some(Message::Quit),
                    Key::Character("v") if status == iced::event::Status::Ignored => {
                        Some(Message::Paste)
                    }
                    _ => None,
                }
            }
            _ => None,
        })
    }
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PreferencesLoaded(Ok(preferences)) => {
                self.preferences = preferences;
                self.preferences_loaded = true;
            }
            Message::PreferencesLoaded(Err(error)) => {
                self.notice = Some(format!("{error}. Existing settings are preserved; repair the file and restart to enable saving."));
            }
            Message::DirectoryLoaded(path, result) => {
                self.loading.remove(&path);
                self.listings.insert(path, result);
            }
            Message::PreviewLoaded(path, target, Ok(contents)) => {
                let title = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "File".into());
                let document = Document::from_scrollback(title, contents);
                self.workspace.add(PaneBody::Editor { document, target });
            }
            Message::PreviewLoaded(path, _, Err(error)) => {
                self.notice = Some(format!("{}: {error}", path.display()));
            }
            Message::Cursor(point) => self.cursor = point,
            Message::Dismiss => {
                if self
                    .settings
                    .as_ref()
                    .is_some_and(|dialog| dialog.saving)
                {
                    return Task::none();
                }
                self.settings = None;
                self.palette = false;
                self.profile = false;
                self.context = None;
            }
            Message::Settings(edit) => return self.edit_settings(edit),
            Message::Persisted(result, prefs) => {
                self.saving = false;
                if let Some(dialog) = self.settings.as_mut() {
                    dialog.saving = false;
                }
                match result {
                    Ok(()) => {
                        self.preferences = prefs;
                        self.settings = None;
                        self.notice = Some("Preferences saved".into());
                    }
                    Err(error) => {
                        if let Some(dialog) = self.settings.as_mut() {
                            dialog.error = Some(error.clone());
                        }
                        self.notice = Some(error);
                    }
                }
            }
            Message::Quit => return iced::exit(),
            Message::Minimize => return iced::window::latest().and_then(|id| iced::window::minimize(id, true)),
            Message::Maximize => return iced::window::latest().and_then(iced::window::toggle_maximize),
            Message::DragWindow => return iced::window::latest().and_then(iced::window::drag),
            // Modal keyboard shortcuts must never mutate the obscured workspace.
            _ if self.settings.is_some() => {}
            Message::Command(command) => return self.command(command),
            Message::SelectTab(id) => {
                self.workspace.activate(id);
                self.context = None;
            }
            Message::CloseTab(id) => {
                self.workspace.close_tab(id);
                self.context = None;
            }
            Message::Focus(pane) => {
                if self.workspace.tab().panes.get(pane).is_some() {
                    self.workspace.tab_mut().focus = pane;
                }
            }
            Message::Split(axis) => {
                self.context = None;
                self.workspace.split(axis, self.preferences.launch_kind);
            }
            Message::Resize(event) => self.workspace.tab_mut().panes.resize(event.split, event.ratio.clamp(0.1, 0.9)),
            Message::Drag(pane_grid::DragEvent::Dropped { pane, target }) => self.workspace.tab_mut().panes.drop(pane, target),
            Message::Drag(_) => {}
            Message::MaximizePane(pane) => {
                let tab = self.workspace.tab_mut();
                if tab.panes.maximized().is_some() {
                    tab.panes.restore();
                } else {
                    tab.panes.maximize(pane);
                }
            }
            Message::ClosePane(pane) => {
                self.workspace.tab_mut().focus = pane;
                self.workspace.close_pane();
                self.context = None;
            }
            Message::Input(pane, input) => {
                if let Some(PaneBody::Terminal { session, .. }) = self.workspace.tab_mut().panes.get_mut(pane) {
                    session.clear_preview();
                    session.append_paste(&input);
                }
            }
            Message::Paste => {
                self.context = None;
                // The tab id travels with the reply: focus may move before the paste arrives.
                let tab = self.workspace.tab();
                let (id, pane) = (tab.id, tab.focus);
                return iced::clipboard::read().map(move |clipboard| Message::Pasted(id, pane, clipboard));
            }
            Message::Pasted(id, pane, Some(contents)) => {
                if let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == id)
                    && let Some(PaneBody::Terminal { session, .. }) = tab.panes.get_mut(pane)
                {
                    session.append_paste(&contents);
                }
            }
            Message::Pasted(_, _, None) => {}
            Message::OpenContext(pane) => {
                self.workspace.tab_mut().focus = pane;
                self.context = Some(self.cursor);
                self.palette = false;
                self.profile = false;
            }
            Message::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
            Message::Sidebar(mode) => {
                if mode != Sidebar::Git || !self.workspace.is_remote() {
                    self.sidebar = mode;
                }
            }
            Message::ToggleDirectory(path) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path.clone());
                    let cached = matches!(self.listings.get(&path), Some(Ok(_)));
                    if !cached && self.loading.insert(path.clone()) {
                        return load_directory(path);
                    }
                }
            }
            Message::OpenFile(path) => {
                let target = self
                    .workspace
                    .tab()
                    .focused()
                    .map(|pane| pane.target().clone())
                    .unwrap_or_else(|| SessionTarget::Local {
                        directory: self.workspace.root.clone(),
                    });
                if target.is_remote() {
                    self.notice = Some("Remote file access arrives in Phase 4.".into());
                } else {
                    let source = path.clone();
                    return Task::perform(blocking(move || files::preview(&source)), move |result| {
                        Message::PreviewLoaded(path.clone(), target.clone(), result)
                    });
                }
            }
            Message::Filter(query) => self.file_query = query,
            Message::ToggleSearch => {
                self.search_visible = !self.search_visible;
                self.sidebar_visible = true;
                self.sidebar = Sidebar::Files;
            }
            Message::TogglePalette => {
                self.palette = !self.palette;
                self.profile = false;
                self.context = None;
            }
            Message::ToggleProfile => {
                self.profile = !self.profile;
                self.palette = false;
                self.context = None;
            }
            Message::SetRoot(root) => {
                self.profile = false;
                self.workspace.root = root.clone();
                self.workspace.add(PaneBody::terminal(
                    SessionTarget::Local { directory: root.clone() },
                    self.preferences.launch_kind,
                ));
                self.listings.clear();
                self.expanded.clear();
                self.loading.insert(root.clone());
                return load_directory(root);
            }
            Message::HostField(field, value) => match field {
                HostField::Name => self.host_name = value,
                HostField::Address => self.host_address = value,
                HostField::User => self.host_user = value,
                HostField::Port => self.host_port = value,
            },
            Message::SaveHost => {
                let host = Host {
                    name: self.host_name.trim().into(),
                    address: self.host_address.trim().into(),
                    user: self.host_user.trim().into(),
                    // An unparsable port becomes 0 so host validation reports it.
                    port: self.host_port.parse().unwrap_or(0),
                };
                match host.validate() {
                    Err(error) => self.notice = Some(error),
                    Ok(()) => {
                        let mut prefs = self.preferences.clone();
                        prefs.hosts.push(host);
                        return self.persist(prefs);
                    }
                }
            }
            Message::RemoveHost(index) => {
                let mut prefs = self.preferences.clone();
                if index < prefs.hosts.len() {
                    prefs.hosts.remove(index);
                    return self.persist(prefs);
                }
            }
            Message::Notify => self.notice = Some("Phase 1 · Native UI shell. PTY: Phase 2 · Editor/Git: Phase 3 · SSH/SFTP: Phase 4 · AI/JEV: Phase 5.".into()),
            Message::ClearNotice => self.notice = None,
        }
        self.sync_scope()
    }
    fn sync_scope(&mut self) -> Task<Message> {
        if self.workspace.is_remote() && self.sidebar == Sidebar::Git {
            self.sidebar = Sidebar::Files;
        }
        if let Some(SessionTarget::Local { directory }) =
            self.workspace.tab().focused().map(PaneBody::target)
        {
            self.workspace.root = directory.clone();
            let root = self.workspace.root.clone();
            if !self.listings.contains_key(&root) && self.loading.insert(root.clone()) {
                return load_directory(root);
            }
        }
        Task::none()
    }
    fn edit_settings(&mut self, edit: Edit) -> Task<Message> {
        match edit {
            Edit::Cancel => {
                if !self.saving {
                    self.settings = None;
                }
            }
            Edit::Apply => {
                if let Some(dialog) = &self.settings {
                    return self.persist(dialog.draft.clone());
                }
            }
            other => {
                if let Some(dialog) = self.settings.as_mut() {
                    dialog.edit(other);
                }
            }
        }
        Task::none()
    }
    fn persist(&mut self, prefs: Preferences) -> Task<Message> {
        if self.saving || !self.preferences_loaded {
            return Task::none();
        }
        if let Err(error) = prefs.validate() {
            if let Some(dialog) = self.settings.as_mut() {
                dialog.error = Some(error.clone());
            }
            self.notice = Some(error);
            return Task::none();
        }
        let Some(path) = settings::config_path() else {
            self.notice = Some("Cannot locate settings directory.".into());
            return Task::none();
        };
        self.saving = true;
        if let Some(dialog) = self.settings.as_mut() {
            dialog.saving = true;
        }
        let saved = prefs.clone();
        Task::perform(
            blocking(move || settings::save(&path, &saved)),
            move |result| Message::Persisted(result, prefs.clone()),
        )
    }
    fn command(&mut self, command: Command) -> Task<Message> {
        self.context = None;
        self.palette = false;
        self.profile = false;
        match command {
            Command::NewTab => {
                let home = self
                    .preferences
                    .home_directory
                    .then(home_directory)
                    .flatten();
                self.workspace
                    .new_terminal(self.preferences.launch_kind, home);
            }
            Command::NewAgent => self.workspace.new_terminal(LaunchKind::Agent, None),
            Command::SplitRight => self
                .workspace
                .split(Axis::Vertical, self.preferences.launch_kind),
            Command::SplitDown => self
                .workspace
                .split(Axis::Horizontal, self.preferences.launch_kind),
            Command::MoveToTab => self.workspace.move_to_tab(),
            Command::ClosePane => self.workspace.close_pane(),
            Command::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
            Command::Settings => {
                if self.preferences_loaded {
                    self.settings = Some(Dialog::new(&self.preferences));
                }
            }
            Command::Scrollback => {
                if let Some(PaneBody::Terminal { session, .. }) = self.workspace.tab().focused() {
                    let target = session.target().clone();
                    let document = Document::from_scrollback("Scrollback", session.transcript());
                    self.workspace.add(PaneBody::Editor { document, target });
                }
            }
        }
        self.sync_scope()
    }
    pub fn view(&self) -> Element<'_, Message> {
        let workspace =
            pane_grid::PaneGrid::new(&self.workspace.tab().panes, |pane, body, maximized| {
                self.pane(pane, body, maximized)
            })
            .spacing(7)
            .min_size(120)
            .on_click(Message::Focus)
            .on_resize(8, Message::Resize)
            .on_drag(Message::Drag);
        let mut body = row![].spacing(7).height(Fill);
        if self.sidebar_visible {
            body = body.push(self.sidebar());
        }
        body = body.push(workspace);
        let mut layout = column![
            self.top_bar(),
            container(body)
                .padding(iced::padding::horizontal(8.0).bottom(8.0))
                .height(Fill)
        ];
        if let Some(notice) = &self.notice {
            layout = layout.push(
                container(
                    row![
                        text(notice).size(12).width(Fill),
                        ui::action(Icon::Close, "Dismiss", Message::ClearNotice)
                    ]
                    .align_y(iced::Center),
                )
                .padding([0, 12]),
            );
        }
        let base: Element<'_, Message> = container(layout)
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(ui::WINDOW.into()),
                text_color: Some(ui::INK),
                ..Default::default()
            })
            .into();
        if let Some(dialog) = &self.settings {
            stack![
                base,
                widget::opaque(
                    mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
                        container::Style {
                            background: Some(iced::Color::from_rgba8(62, 71, 88, 0.12).into()),
                            ..Default::default()
                        }
                    }))
                    .on_press(Message::Dismiss)
                ),
                container(widget::opaque(dialog.view().map(Message::Settings)))
                    .padding(28)
                    .center(Fill)
            ]
            .into()
        } else if let Some(point) = self.context {
            let menu = self.context_menu();
            // Clamp the menu origin inside the viewport: at least 8px in, leaving 12px at the far edge.
            let clamp = |origin: f32, size: f32, extent: f32| {
                origin.min((extent - size - 12.0).max(8.0)).max(8.0)
            };
            let floating = widget::float(menu).translate(move |bounds, viewport| {
                iced::Vector::new(
                    clamp(point.x, bounds.width, viewport.width) - bounds.x,
                    clamp(point.y, bounds.height, viewport.height) - bounds.y,
                )
            });
            stack![
                base,
                widget::opaque(
                    mouse_area(container(Space::new()).width(Fill).height(Fill))
                        .on_press(Message::Dismiss)
                        .on_right_press(Message::Dismiss)
                ),
                floating
            ]
            .into()
        } else if self.palette || self.profile {
            // The profile switcher sits under the bar's left button; the palette centers.
            let (popover, left, align_x) = if self.palette {
                (
                    self.command_palette(),
                    0.0,
                    iced::alignment::Horizontal::Center,
                )
            } else {
                (self.profiles(), 110.0, iced::Left)
            };
            stack![
                base,
                widget::opaque(
                    mouse_area(container(Space::new()).width(Fill).height(Fill))
                        .on_press(Message::Dismiss)
                ),
                container(widget::opaque(popover))
                    .padding(iced::Padding {
                        top: 52.0,
                        left,
                        ..Default::default()
                    })
                    .align_x(align_x)
                    .width(Fill)
            ]
            .into()
        } else {
            base
        }
    }
    fn top_bar(&self) -> Element<'_, Message> {
        let tabs = iced::widget::Row::with_children(self.workspace.tabs.iter().map(|tab| {
            ui::chrome::tab(
                tab.focused().map(PaneBody::title).unwrap_or("~"),
                tab.focused().map(pane_icon).unwrap_or(Icon::Terminal),
                tab.id == self.workspace.tab().id,
                Message::SelectTab(tab.id),
                Message::CloseTab(tab.id),
            )
        }))
        .spacing(4)
        .align_y(iced::Center);
        let actions = row![
            ui::action(Icon::Panel, "Toggle sidebar · ⌘B", Message::ToggleSidebar),
            ui::action(
                Icon::SplitRight,
                "Split right · ⌘D",
                Message::Split(Axis::Vertical)
            ),
            ui::action(
                Icon::SplitDown,
                "Split down · ⇧⌘D",
                Message::Split(Axis::Horizontal)
            ),
            ui::action(Icon::Search, "Filter files", Message::ToggleSearch),
            ui::action(
                Icon::Command,
                "Command palette · ⇧⌘P",
                Message::TogglePalette
            ),
            ui::action(Icon::Bell, "Notifications", Message::Notify),
            ui::action(
                Icon::Settings,
                "Settings · ⌘,",
                Message::Command(Command::Settings)
            )
        ]
        .spacing(3)
        .align_y(iced::Center);
        ui::chrome::unified_bar(
            self.window_controls(),
            Message::ToggleProfile,
            tabs.into(),
            Message::Command(Command::NewTab),
            mouse_area(Space::new().width(iced::Length::FillPortion(1)).height(40))
                .on_press(Message::DragWindow)
                .on_double_click(Message::Maximize)
                .into(),
            actions.into(),
        )
    }
    fn window_controls(&self) -> Element<'_, Message> {
        if cfg!(target_os = "macos") {
            let dot = |color: iced::Color, message: Message| {
                button(Space::new().width(11).height(11))
                    .padding(0)
                    .on_press(message)
                    .style(move |_, _| widget::button::Style {
                        background: Some(color.into()),
                        border: iced::Border {
                            radius: 6.0.into(),
                            width: 0.5,
                            color: iced::Color::from_rgba8(0, 0, 0, 0.12),
                        },
                        ..Default::default()
                    })
            };
            container(
                row![
                    dot(iced::Color::from_rgb8(255, 96, 88), Message::Quit),
                    dot(iced::Color::from_rgb8(255, 189, 46), Message::Minimize),
                    dot(iced::Color::from_rgb8(40, 200, 64), Message::Maximize)
                ]
                .spacing(9),
            )
            .padding([0, 8])
            .into()
        } else {
            row![
                ui::action(Icon::Minimize, "Minimize", Message::Minimize),
                ui::action(Icon::Maximize, "Maximize", Message::Maximize),
                ui::action(Icon::Close, "Close", Message::Quit)
            ]
            .into()
        }
    }
    fn sidebar(&self) -> Element<'_, Message> {
        let remote = self.workspace.is_remote();
        let root_name = self
            .workspace
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.workspace.root.display().to_string());
        let (header_icon, title) = match self.sidebar {
            Sidebar::Files => (Icon::Folder, if remote { "SFTP" } else { &root_name }),
            Sidebar::Hosts => (Icon::Server, "SSH Hosts"),
            Sidebar::Git => (Icon::Git, "Local Git"),
        };
        let mut modes = row![].spacing(4);
        for (label, mode, kind) in [
            ("Files", Sidebar::Files, Icon::Folder),
            ("SSH", Sidebar::Hosts, Icon::Server),
            ("Git", Sidebar::Git, Icon::Git),
        ] {
            modes = modes.push(
                button(
                    row![ui::icon(kind, 13.0), text(label).size(11)]
                        .spacing(5)
                        .align_y(iced::Center),
                )
                .padding([7, 9])
                .style(if self.sidebar == mode {
                    theme::selected
                } else {
                    theme::quiet
                })
                .on_press_maybe(
                    (mode != Sidebar::Git || !remote).then_some(Message::Sidebar(mode)),
                ),
            );
        }
        let filter = (self.search_visible && self.sidebar == Sidebar::Files).then(|| {
            text_input("Filter this tree…", &self.file_query)
                .on_input(Message::Filter)
                .padding(9)
                .size(12)
                .style(theme::input)
                .into()
        });
        let content: Element<'_, Message> = match self.sidebar {
            Sidebar::Files if remote => container(
                column![
                    ui::icon(Icon::Server, 24.0),
                    text("Remote files").size(15),
                    ui::muted("SFTP browsing is connected in Phase 4.")
                ]
                .spacing(14),
            )
            .padding(18)
            .into(),
            Sidebar::Files => scrollable(self.tree(&self.workspace.root, 0))
                .height(Fill)
                .into(),
            Sidebar::Hosts => self.hosts(),
            Sidebar::Git => container(
                column![
                    ui::icon(Icon::Git, 24.0),
                    text("Local repository").size(15),
                    text(self.workspace.root.display().to_string()).size(12),
                    ui::muted("Git status is connected in Phase 3. This panel never accesses remote sessions.")
                ]
                .spacing(14),
            )
            .padding(18)
            .into(),
        };
        let footer = row![
            ui::icons::tinted(Icon::Terminal, 12.0, ui::ACCENT),
            ui::muted(if remote {
                "Remote session"
            } else {
                "Local workspace"
            }),
            Space::new().width(Fill),
            ui::action(
                Icon::Settings,
                "Settings",
                Message::Command(Command::Settings)
            )
        ]
        .spacing(7)
        .align_y(iced::Center);
        ui::chrome::floating_sidebar(
            title.to_owned(),
            header_icon,
            Message::ToggleSidebar,
            modes.into(),
            filter,
            content,
            footer.into(),
        )
    }
    fn tree<'a>(&'a self, path: &Path, depth: u16) -> Element<'a, Message> {
        if depth > 24 {
            return container(ui::muted("Tree depth limit reached"))
                .padding(12)
                .into();
        }
        let Some(result) = self.listings.get(path) else {
            return container(ui::muted("Loading…")).padding(12).into();
        };
        let listing = match result {
            Ok(list) => list,
            Err(error) => {
                return container(text(error).size(12).color(theme::DANGER))
                    .padding(12)
                    .into();
            }
        };
        let query = self.file_query.to_lowercase();
        let mut nodes = column![].spacing(1).padding([4, 6]);
        for entry in &listing.entries {
            // Directories always survive the filter so matching children keep their context.
            if !query.is_empty() && !entry.directory && !entry.name.to_lowercase().contains(&query)
            {
                continue;
            }
            let expanded = self.expanded.contains(&entry.path);
            let caret: Element<'_, Message> = if entry.directory {
                ui::icons::tinted(
                    if expanded {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    12.0,
                    ui::MUTED,
                )
                .into()
            } else {
                Space::new().width(12).into()
            };
            let label = row![
                caret,
                ui::icon(
                    if entry.directory {
                        Icon::Folder
                    } else {
                        Icon::File
                    },
                    14.0
                ),
                text(&entry.name)
                    .size(12)
                    .wrapping(widget::text::Wrapping::None)
            ]
            .spacing(8)
            .align_y(iced::Center);
            nodes = nodes.push(
                button(label)
                    .width(Fill)
                    .padding(iced::Padding {
                        top: 7.0,
                        bottom: 7.0,
                        left: 8.0 + f32::from(depth) * 12.0,
                        right: 8.0,
                    })
                    .style(theme::quiet)
                    .on_press(if entry.directory {
                        Message::ToggleDirectory(entry.path.clone())
                    } else {
                        Message::OpenFile(entry.path.clone())
                    }),
            );
            if entry.directory && expanded {
                nodes = nodes.push(self.tree(&entry.path, depth + 1));
            }
        }
        if listing.entries.is_empty() {
            nodes = nodes.push(container(ui::muted("Empty directory")).padding(12));
        }
        if listing.truncated {
            nodes = nodes.push(ui::muted("Showing up to 500 entries."));
        }
        nodes.into()
    }
    fn hosts(&self) -> Element<'_, Message> {
        let mut hosts = column![
            text("Saved hosts").size(13),
            ui::muted("Host profiles only · Connections arrive in Phase 4.")
        ]
        .spacing(12);
        for (index, host) in self.preferences.hosts.iter().enumerate() {
            hosts = hosts.push(
                container(
                    column![
                        row![
                            ui::icon(Icon::Server, 15.0),
                            text(&host.name).size(13).width(Fill),
                            button(ui::icon(Icon::Close, 12.0))
                                .padding(4)
                                .style(theme::quiet)
                                .on_press_maybe(
                                    (!self.saving).then_some(Message::RemoveHost(index))
                                )
                        ]
                        .spacing(7)
                        .align_y(iced::Center),
                        text(format!("{}@{}:{}", host.user, host.address, host.port)).size(11)
                    ]
                    .spacing(7),
                )
                .padding(10)
                .style(theme::canvas),
            );
        }
        if self.preferences.hosts.is_empty() {
            hosts = hosts.push(ui::muted("No saved hosts"));
        }
        hosts = hosts
            .push(widget::rule::horizontal(1))
            .push(text("Add a host").size(13));
        for (placeholder, value, field) in [
            ("Name", &self.host_name, HostField::Name),
            ("Hostname or IP", &self.host_address, HostField::Address),
            ("Username", &self.host_user, HostField::User),
            ("Port", &self.host_port, HostField::Port),
        ] {
            hosts = hosts.push(
                text_input(placeholder, value)
                    .on_input(move |value| Message::HostField(field, value))
                    .padding(10)
                    .size(12)
                    .style(theme::input),
            );
        }
        hosts = hosts
            .push(
                button(
                    text(if self.saving {
                        "Saving…"
                    } else {
                        "Save host"
                    })
                    .size(12),
                )
                .padding([9, 12])
                .style(theme::selected)
                .on_press_maybe(
                    (!self.saving && self.preferences_loaded).then_some(Message::SaveHost),
                ),
            )
            .push(ui::muted("Passwords and private keys are not stored here."));
        scrollable(container(hosts).padding(14)).height(Fill).into()
    }
    fn pane<'a>(
        &'a self,
        pane: Pane,
        body: &'a PaneBody,
        maximized: bool,
    ) -> pane_grid::Content<'a, Message> {
        let focused = pane == self.workspace.tab().focus;
        let body_view: Element<'_, Message> = match body {
            PaneBody::Terminal { session, kind } => {
                let agent = *kind == LaunchKind::Agent;
                let prompt = row![
                    text("~").size(14).font(Font::MONOSPACE).color(ui::ACCENT),
                    text(">").size(14).font(Font::MONOSPACE),
                    text_input(if agent { "Agent draft" } else { "" }, session.input())
                        .id(format!("draft-{}-{pane:?}", self.workspace.tab().id))
                        .on_input(move |input| Message::Input(pane, input))
                        .padding(0)
                        .font(Font::MONOSPACE)
                        .size(14)
                        .style(|t, s| widget::text_input::Style {
                            background: iced::Color::TRANSPARENT.into(),
                            border: iced::Border::default(),
                            ..theme::input(t, s)
                        })
                ]
                .spacing(10)
                .align_y(iced::Center);
                column![
                    prompt,
                    Space::new().height(Fill),
                    ui::muted(if agent {
                        "Agent draft only · Model execution arrives in Phase 5"
                    } else {
                        "Input draft only · Local PTY connects in Phase 2"
                    })
                ]
                .spacing(16)
                .into()
            }
            PaneBody::Editor { document, .. } => {
                let lines = document.text().lines().count().max(1);
                scrollable(
                    row![
                        text(
                            (1..=lines)
                                .map(|n| n.to_string())
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                        .font(Font::MONOSPACE)
                        .size(13)
                        .color(ui::MUTED),
                        text(document.text())
                            .size(13)
                            .font(Font::MONOSPACE)
                            .width(Fill)
                    ]
                    .spacing(20),
                )
                .height(Fill)
                .width(Fill)
                .into()
            }
        };
        let content = mouse_area(container(body_view).padding(18).width(Fill).height(Fill))
            .on_right_press(Message::OpenContext(pane));
        let mut pane_content =
            pane_grid::Content::new(content).style(move |_| iced::widget::container::Style {
                border: iced::Border {
                    color: if focused && self.workspace.tab().panes.len() > 1 {
                        iced::Color::from_rgb8(162, 191, 192)
                    } else {
                        ui::BORDER
                    },
                    width: 1.0,
                    radius: 10.0.into(),
                },
                background: Some(ui::CANVAS.into()),
                text_color: Some(ui::INK),
                ..Default::default()
            });
        if self.workspace.tab().panes.len() > 1 || matches!(body, PaneBody::Editor { .. }) {
            let title = row![
                ui::icon(pane_icon(body), 13.0),
                text(body.title()).size(12),
                text(body.target().to_string()).size(11).color(ui::MUTED)
            ]
            .spacing(8)
            .align_y(iced::Center);
            let controls = row![
                ui::action(
                    Icon::Maximize,
                    if maximized {
                        "Restore pane"
                    } else {
                        "Maximize pane"
                    },
                    Message::MaximizePane(pane)
                ),
                ui::action(Icon::Close, "Close pane", Message::ClosePane(pane))
            ];
            pane_content = pane_content.title_bar(
                pane_grid::TitleBar::new(title)
                    .controls(Element::from(controls))
                    .padding([3, 10])
                    .style(|_| container::Style {
                        background: Some(ui::PANEL.into()),
                        text_color: Some(ui::INK),
                        ..Default::default()
                    }),
            );
        }
        pane_content
    }
    fn context_menu(&self) -> Element<'_, Message> {
        let terminal = matches!(
            self.workspace.tab().focused(),
            Some(PaneBody::Terminal { .. })
        );
        ui::chrome::context_menu(ui::chrome::ContextActions {
            paste: terminal.then_some(Message::Paste),
            split_right: Message::Command(Command::SplitRight),
            split_down: Message::Command(Command::SplitDown),
            move_to_tab: (self.workspace.tab().panes.len() > 1)
                .then_some(Message::Command(Command::MoveToTab)),
            scrollback: terminal.then_some(Message::Command(Command::Scrollback)),
            close: Message::Command(Command::ClosePane),
        })
    }
    fn command_palette(&self) -> Element<'_, Message> {
        container(column![
            container(text("Workspace commands").size(13).color(ui::MUTED)).padding([10, 12]),
            ui::menu_item(
                Icon::Plus,
                "New tab",
                "⌘T",
                Some(Message::Command(Command::NewTab))
            ),
            ui::menu_item(
                Icon::Model,
                "New agent pane",
                "⇧⌘A",
                Some(Message::Command(Command::NewAgent))
            ),
            ui::menu_item(
                Icon::SplitRight,
                "Split right",
                "⌘D",
                Some(Message::Command(Command::SplitRight))
            ),
            ui::menu_item(
                Icon::SplitDown,
                "Split down",
                "⇧⌘D",
                Some(Message::Command(Command::SplitDown))
            ),
            ui::menu_item(
                Icon::Panel,
                "Toggle sidebar",
                "⌘B",
                Some(Message::Command(Command::ToggleSidebar))
            ),
            ui::menu_item(
                Icon::Settings,
                "Settings",
                "⌘,",
                Some(Message::Command(Command::Settings))
            )
        ])
        .padding(8)
        .width(420)
        .style(theme::popover)
        .into()
    }
    fn profiles(&self) -> Element<'_, Message> {
        let mut items = column![
            container(text("Default workspace").size(13).color(ui::MUTED)).padding([10, 12]),
            ui::menu_item(
                Icon::Folder,
                "Project directory",
                "",
                Some(Message::SetRoot(self.project_root.clone()))
            )
        ];
        if let Some(home) = home_directory() {
            items = items.push(ui::menu_item(
                Icon::Home,
                "Home directory",
                "",
                Some(Message::SetRoot(home)),
            ));
        }
        container(items)
            .padding(8)
            .width(270)
            .style(theme::popover)
            .into()
    }
}
fn load_directory(path: PathBuf) -> Task<Message> {
    let source = path.clone();
    Task::perform(blocking(move || files::list(&source)), move |result| {
        Message::DirectoryLoaded(path.clone(), result)
    })
}
fn home_directory() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Header icon shared by the tab strip and the pane title bar.
fn pane_icon(body: &PaneBody) -> Icon {
    if matches!(body, PaneBody::Editor { .. }) {
        Icon::File
    } else {
        Icon::Terminal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modal_blocks_workspace_shortcuts_and_cancel_discards_draft() {
        let mut app = App::new(PathBuf::from("/tmp"));
        app.preferences_loaded = true;
        let _ = app.update(Message::Command(Command::Settings));
        let _ = app.update(Message::Settings(Edit::Launch(LaunchKind::Agent)));
        let _ = app.update(Message::Split(Axis::Vertical));
        let _ = app.update(Message::Command(Command::NewTab));
        assert_eq!(app.workspace.tabs.len(), 1);
        assert_eq!(app.workspace.tab().panes.len(), 1);
        let _ = app.update(Message::Dismiss);
        assert_eq!(app.preferences.launch_kind, LaunchKind::Shell);
    }
    #[test]
    fn git_cannot_be_selected_for_remote_sessions_or_remote_previews() {
        let mut app = App::new(PathBuf::from("/tmp"));
        app.sidebar = Sidebar::Git;
        app.workspace.add(PaneBody::terminal(
            SessionTarget::Remote {
                host: "prod".into(),
                directory: "/srv".into(),
            },
            LaunchKind::Shell,
        ));
        let _ = app.update(Message::Focus(app.workspace.tab().focus));
        assert_eq!(app.sidebar, Sidebar::Files);
        let _ = app.update(Message::Sidebar(Sidebar::Git));
        assert_eq!(app.sidebar, Sidebar::Files);
        let _ = app.update(Message::Command(Command::Scrollback));
        assert!(app.workspace.is_remote());
    }
    #[test]
    fn asynchronous_paste_targets_original_tab_not_current_focus() {
        let mut app = App::new(PathBuf::from("/tmp"));
        let pane = app.workspace.tab().focus;
        app.workspace.new_terminal(LaunchKind::Agent, None);
        let _ = app.update(Message::Pasted(0, pane, Some("draft".into())));
        if let Some(PaneBody::Terminal { session, .. }) = app.workspace.tabs[0].panes.get(pane) {
            assert_eq!(session.input(), "draft");
        } else {
            panic!("expected terminal");
        }
        if let Some(PaneBody::Terminal { session, .. }) = app.workspace.tab().focused() {
            assert_eq!(session.input(), "");
        }
    }
}
