use crate::{self as ui, Icon, theme};
use iced::{
    Element, Fill, Font,
    widget::{Space, button, column, container, row, scrollable, text},
};

/// Plain-text tab: `› title`, close affordance only on the active tab.
pub fn tab<'a, M: Clone + 'a>(title: &'a str, active: bool, select: M, close: M) -> Element<'a, M> {
    let label = |color: iced::Color| {
        row![
            text("›")
                .font(Font::MONOSPACE)
                .size(12)
                .color(if active { ui::ACCENT } else { color }),
            text(title).size(12).color(color),
        ]
        .spacing(6)
        .align_y(iced::Center)
    };
    let mut content = row![
        button(label(if active { ui::INK } else { ui::MUTED }))
            .padding([5, 8])
            .style(theme::quiet)
            .on_press(select),
    ]
    .align_y(iced::Center);
    if active {
        content = content.push(
            button(ui::icon(Icon::Close, 10.0))
                .padding(5)
                .style(theme::quiet)
                .on_press(close),
        );
    }
    content.into()
}

pub fn unified_bar<'a, M: Clone + 'a>(
    controls: Element<'a, M>,
    profile: M,
    tabs: Element<'a, M>,
    new_tab: M,
    drag_area: Element<'a, M>,
    actions: Element<'a, M>,
) -> Element<'a, M> {
    container(
        row![
            controls,
            button(
                row![
                    ui::icon(Icon::Folder, 15.0),
                    text("Default").size(13),
                    ui::icon(Icon::ChevronDown, 12.0)
                ]
                .spacing(8)
                .align_y(iced::Center)
            )
            .padding([7, 10])
            .style(theme::quiet)
            .on_press(profile),
            container(Space::new().width(1).height(20)).style(|_| container::Style {
                background: Some(iced::Color::from_rgb8(197, 202, 210).into()),
                ..Default::default()
            }),
            scrollable(tabs)
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::new().width(0).scroller_width(0)
                ))
                .width(iced::Length::Shrink),
            ui::action(Icon::Plus, "New tab · ⌘T", new_tab),
            drag_area,
            actions
        ]
        .spacing(3)
        .align_y(iced::Center),
    )
    .padding([3, 14])
    .height(48)
    .into()
}

pub fn floating_sidebar<'a, M: Clone + 'a>(
    title: String,
    kind: Icon,
    hide: M,
    modes: Element<'a, M>,
    filter: Option<Element<'a, M>>,
    body: Element<'a, M>,
    footer: Element<'a, M>,
) -> Element<'a, M> {
    let header = row![
        ui::icon(kind, 17.0),
        text(title).size(14).width(Fill),
        ui::action(Icon::Panel, "Hide sidebar", hide)
    ]
    .align_y(iced::Center)
    .spacing(9);
    let mut contents = column![
        container(header).padding([8, 12]),
        ui::widgets::divider(),
        container(modes).padding([8, 10])
    ];
    if let Some(filter) = filter {
        contents = contents.push(container(filter).padding([0, 10]));
    }
    container(
        contents
            .push(container(body).height(Fill))
            .push(container(footer).padding([6, 12])),
    )
    .style(theme::flat)
    .width(248)
    .height(Fill)
    .into()
}

pub struct ContextActions<M> {
    pub paste: Option<M>,
    pub split_right: M,
    pub split_down: M,
    pub move_to_tab: Option<M>,
    pub scrollback: Option<M>,
    pub find: Option<M>,
    pub clear: Option<M>,
    pub restart: Option<M>,
    pub close: M,
}

pub fn context_menu<'a, M: Clone + 'a>(actions: ContextActions<M>) -> Element<'a, M> {
    container(
        column![
            ui::menu_item(Icon::Paste, "Paste", "⌘V", actions.paste),
            ui::widgets::divider(),
            ui::menu_item(
                Icon::SplitRight,
                "Split right",
                "⌘D",
                Some(actions.split_right)
            ),
            ui::menu_item(
                Icon::SplitDown,
                "Split down",
                "⇧⌘D",
                Some(actions.split_down)
            ),
            ui::menu_item(Icon::Move, "Move pane to tab…", "", actions.move_to_tab),
            ui::widgets::divider(),
            ui::menu_item(Icon::Search, "Find in scrollback", "⌘F", actions.find),
            ui::menu_item(
                Icon::File,
                "Open scrollback in editor",
                "",
                actions.scrollback
            ),
            ui::menu_item(
                Icon::Trash,
                "Clear screen and scrollback",
                "⇧⌘K",
                actions.clear
            ),
            ui::menu_item(Icon::Restart, "Restart shell", "", actions.restart),
            ui::widgets::divider(),
            ui::menu_item(Icon::Close, "Close block", "⌘W", Some(actions.close))
        ]
        .spacing(3),
    )
    .padding(7)
    .width(355)
    .style(theme::popover)
    .into()
}
