use crate::{self as ui, Icon, theme};
use iced::{
    Element, Fill,
    widget::{
        Space, button, column, container, row, scrollable, slider, text, text_input, toggler,
    },
};
use mud_agent::{AutopilotMode, LaunchKind, ModelSettings};
use mud_ssh::Host;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub shell: String,
    pub launch_kind: LaunchKind,
    pub home_directory: bool,
    pub copy_on_select: bool,
    pub command_click: bool,
    pub model: ModelSettings,
    pub autopilot: AutopilotMode,
    pub confidence: f32,
    pub hosts: Vec<Host>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            shell: String::new(),
            launch_kind: LaunchKind::Shell,
            home_directory: false,
            copy_on_select: false,
            command_click: false,
            model: ModelSettings::default(),
            autopilot: AutopilotMode::StepApproval,
            confidence: 0.9,
            hosts: Vec::new(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<(), String> {
        if !self.model.model_id.trim().is_empty() {
            self.model.validate()?;
        }
        if !self.confidence.is_finite() || !(0.5..=1.0).contains(&self.confidence) {
            return Err("Confidence must be between 50% and 100%.".into());
        }
        for host in &self.hosts {
            host.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    General,
    Models,
    Autopilot,
}
#[derive(Debug, Clone)]
pub enum Edit {
    Search(String),
    Category(Category),
    Shell(String),
    Launch(LaunchKind),
    Home(bool),
    Copy(bool),
    CommandClick(bool),
    BaseUrl(String),
    Model(String),
    Mode(AutopilotMode),
    Confidence(f32),
    Apply,
    Cancel,
}
/// Search index per section; a full search reads all three as one string.
const GENERAL_TERMS: &str = "general shell tabs splits selection copy links directory home agent";
const MODELS_TERMS: &str = "models API base URL model id OpenAI responses";
const AUTOPILOT_TERMS: &str = "JEV autopilot approval confidence smart full auto safety";

#[derive(Debug)]
pub struct Dialog {
    pub draft: Preferences,
    pub search: String,
    pub category: Category,
    pub saving: bool,
    pub error: Option<String>,
}
impl Dialog {
    pub fn new(prefs: &Preferences) -> Self {
        Self {
            draft: prefs.clone(),
            search: String::new(),
            category: Category::General,
            saving: false,
            error: None,
        }
    }
    pub fn edit(&mut self, edit: Edit) {
        if self.saving {
            return;
        }
        self.error = None;
        match edit {
            Edit::Search(s) => self.search = s,
            Edit::Category(c) => self.category = c,
            Edit::Shell(s) => self.draft.shell = s,
            Edit::Launch(k) => self.draft.launch_kind = k,
            Edit::Home(v) => self.draft.home_directory = v,
            Edit::Copy(v) => self.draft.copy_on_select = v,
            Edit::CommandClick(v) => self.draft.command_click = v,
            Edit::BaseUrl(s) => self.draft.model.base_url = s,
            Edit::Model(s) => self.draft.model.model_id = s,
            Edit::Mode(m) => self.draft.autopilot = m,
            Edit::Confidence(v) => self.draft.confidence = v,
            Edit::Apply | Edit::Cancel => {}
        }
    }
    fn matches(&self, terms: &str) -> bool {
        self.search.trim().is_empty()
            || terms
                .to_lowercase()
                .contains(&self.search.trim().to_lowercase())
    }
    pub fn view(&self) -> Element<'_, Edit> {
        let header = row![
            container(ui::icon(Icon::Settings, 19.0))
                .padding(10)
                .style(|_| theme::box_style(theme::SELECTED, 10.0)),
            text("Settings")
                .size(22)
                .font(ui::widgets::heading_font())
                .width(Fill),
            ui::action(Icon::Close, "Close settings · Esc", Edit::Cancel)
        ]
        .spacing(12)
        .align_y(iced::Center);
        let tabs = iced::widget::Row::with_children(
            [
                ("General", Icon::Settings, Category::General),
                ("Models & API", Icon::Model, Category::Models),
                ("JEV Autopilot", Icon::Shield, Category::Autopilot),
            ]
            .into_iter()
            .map(|(label, kind, category)| {
                let active = self.category == category;
                button(
                    column![
                        row![
                            ui::icons::tinted(
                                kind,
                                18.0,
                                if active { ui::ACCENT } else { ui::INK }
                            ),
                            text(label).size(12)
                        ]
                        .spacing(8)
                        .align_y(iced::Center),
                        container(Space::new().height(2).width(Fill)).style(move |_| {
                            container::Style {
                                background: active.then_some(ui::ACCENT.into()),
                                ..Default::default()
                            }
                        })
                    ]
                    .spacing(10),
                )
                .padding([7, 12])
                .style(theme::quiet)
                .on_press(Edit::Category(category))
                .into()
            }),
        )
        .spacing(12);
        let search = container(
            row![
                ui::icon(Icon::Search, 18.0),
                text_input("Search settings", &self.search)
                    .on_input(Edit::Search)
                    .padding(10)
                    .style(|t, s| iced::widget::text_input::Style {
                        border: iced::Border::default(),
                        ..theme::input(t, s)
                    })
            ]
            .spacing(10)
            .align_y(iced::Center),
        )
        .padding([2, 12])
        .style(|_| theme::box_style(ui::CANVAS, 10.0));
        let mut content = column![].spacing(20).padding(24);
        let searching = !self.search.trim().is_empty();
        if (searching || self.category == Category::General) && self.matches(GENERAL_TERMS) {
            content = content.push(self.general());
        }
        if (searching || self.category == Category::Models) && self.matches(MODELS_TERMS) {
            content = content.push(self.models());
        }
        if (searching || self.category == Category::Autopilot) && self.matches(AUTOPILOT_TERMS) {
            content = content.push(self.autopilot());
        }
        if searching && !self.matches(&[GENERAL_TERMS, MODELS_TERMS, AUTOPILOT_TERMS].join(" ")) {
            content = content.push(ui::muted("No settings match your search."));
        }
        let mut footer = row![
            ui::muted("Preferences are stored locally. No credentials are stored."),
            Space::new().width(Fill)
        ]
        .spacing(10)
        .align_y(iced::Center);
        if let Some(error) = &self.error {
            footer = footer.push(text(error).size(12).color(theme::DANGER));
        }
        footer = footer
            .push(
                button(text("Cancel").size(13))
                    .padding([9, 16])
                    .style(theme::quiet)
                    .on_press_maybe((!self.saving).then_some(Edit::Cancel)),
            )
            .push(
                button(
                    text(if self.saving {
                        "Saving…"
                    } else {
                        "Apply changes"
                    })
                    .size(13),
                )
                .padding([9, 16])
                .style(theme::selected)
                .on_press_maybe((!self.saving).then_some(Edit::Apply)),
            );
        container(column![
            container(column![header, search, tabs].spacing(18)).padding(24),
            ui::widgets::divider(),
            scrollable(content).height(Fill),
            ui::widgets::divider(),
            container(footer).padding([14, 24])
        ])
        .width(980)
        .height(Fill)
        .max_height(820)
        .style(|t| container::Style {
            background: Some(ui::PANEL.into()),
            ..theme::popover(t)
        })
        .into()
    }
    fn general(&self) -> Element<'_, Edit> {
        let shell = ui::card(
            column![
                text("Shell").size(16),
                text("Login shell for new blocks. Empty uses your system default.")
                    .size(13)
                    .color(ui::INK),
                text_input(
                    if cfg!(windows) {
                        "System default"
                    } else {
                        "/bin/zsh"
                    },
                    &self.draft.shell
                )
                .on_input(Edit::Shell)
                .padding(12)
                .style(theme::input)
            ]
            .spacing(10),
        );
        let launch = segmented(&[
            (
                "Shell",
                self.draft.launch_kind == LaunchKind::Shell,
                Edit::Launch(LaunchKind::Shell),
            ),
            (
                "Agent",
                self.draft.launch_kind == LaunchKind::Agent,
                Edit::Launch(LaunchKind::Agent),
            ),
        ]);
        let home = segmented(&[
            (
                "Current directory",
                !self.draft.home_directory,
                Edit::Home(false),
            ),
            ("Home", self.draft.home_directory, Edit::Home(true)),
        ]);
        let links = segmented(&[
            (
                "Click",
                !self.draft.command_click,
                Edit::CommandClick(false),
            ),
            (
                "⌘-click",
                self.draft.command_click,
                Edit::CommandClick(true),
            ),
        ]);
        column![
            ui::section("01 · General", "Shell, tabs and selection", "What new blocks run, where new tabs start, and what a selection does."),
            text("Shell").size(14),
            shell,
            text("Tabs and splits").size(14),
            ui::card(
                column![
                    setting_row("New tabs and splits open", "Choose the type of new tabs and splits.", launch),
                    ui::widgets::divider(),
                    setting_row("New tabs start in", "Splits inherit the directory of the pane they split.", home)
                ]
                .spacing(16)
            ),
            text("Selection").size(14),
            ui::card(setting_row(
                "Copy on select",
                "Finishing a selection puts it on the clipboard.",
                toggler(self.draft.copy_on_select).on_toggle(Edit::Copy).into(),
            )),
            text("Links").size(14),
            ui::card(setting_row("Open links with", "Choose how links in terminal output open.", links)),
            ui::muted("Shell, selection and link preferences take effect when the terminal engine is connected in Phase 2.")
        ]
        .spacing(18)
        .into()
    }
    fn models(&self) -> Element<'_, Edit> {
        column![
            ui::section("02 · Models & API", "A direct line to your models", "OpenAI-compatible Chat Completions and Responses configuration."),
            ui::card(
                column![
                    text("API base URL").size(16),
                    text_input("https://api.openai.com/v1", &self.draft.model.base_url)
                        .on_input(Edit::BaseUrl)
                        .padding(12)
                        .style(theme::input),
                    text("Model ID override").size(16),
                    text_input("Enter a model ID", &self.draft.model.model_id)
                        .on_input(Edit::Model)
                        .padding(12)
                        .style(theme::input)
                ]
                .spacing(12)
            ),
            ui::muted("Configuration only · Model discovery, streaming and secure API credentials arrive in Phase 5.")
        ]
        .spacing(18)
        .into()
    }
    fn autopilot(&self) -> Element<'_, Edit> {
        let mode = segmented(&[
            (
                "Step approval",
                self.draft.autopilot == AutopilotMode::StepApproval,
                Edit::Mode(AutopilotMode::StepApproval),
            ),
            (
                "JEV Smart",
                self.draft.autopilot == AutopilotMode::Smart,
                Edit::Mode(AutopilotMode::Smart),
            ),
            (
                "Full auto",
                self.draft.autopilot == AutopilotMode::FullAuto,
                Edit::Mode(AutopilotMode::FullAuto),
            ),
        ]);
        column![
            ui::section("03 · JEV Autopilot", "You set the boundaries", "Choose how the agent will request approval before running tools."),
            ui::card(
                column![
                    text("Approval mode").size(16),
                    mode,
                    text("Dangerous actions must be blocked in every mode.").size(13)
                ]
                .spacing(14)
            ),
            ui::card(
                column![
                    row![
                        text("Smart autopilot confidence").size(16).width(Fill),
                        text(format!("{:.0}%", self.draft.confidence * 100.0)).color(ui::ACCENT)
                    ],
                    slider(0.5..=1.0, self.draft.confidence, Edit::Confidence).step(0.01_f32),
                    ui::muted("Read-only probability must also be at least 85%.")
                ]
                .spacing(14)
            ),
            ui::muted("Configuration only · No AI tools execute in Phase 1. JEV auditing is implemented in Phase 5.")
        ]
        .spacing(18)
        .into()
    }
}
fn segmented(options: &[(&str, bool, Edit)]) -> Element<'static, Edit> {
    let buttons = options.iter().map(|(label, active, message)| {
        let active = *active;
        button(text((*label).to_owned()).size(13))
            .padding([7, 13])
            .style(move |t, s| {
                if active {
                    iced::widget::button::Style {
                        background: Some(ui::CANVAS.into()),
                        border: iced::Border {
                            color: ui::BORDER,
                            width: 1.0,
                            radius: 10.0.into(),
                        },
                        ..theme::quiet(t, s)
                    }
                } else {
                    theme::quiet(t, s)
                }
            })
            .on_press(message.clone())
            .into()
    });
    container(iced::widget::Row::with_children(buttons).spacing(2))
        .padding(3)
        .style(|_| theme::box_style(iced::Color::from_rgb8(243, 244, 246), 13.0))
        .into()
}
fn setting_row<'a>(title: &'a str, help: &'a str, control: Element<'a, Edit>) -> Element<'a, Edit> {
    row![
        column![text(title).size(16), text(help).size(12).color(ui::INK)]
            .spacing(8)
            .width(Fill),
        control
    ]
    .spacing(20)
    .align_y(iced::Center)
    .into()
}
