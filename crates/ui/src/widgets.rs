use crate::{Icon, icon, theme};
use iced::{
    Element, Fill, Font,
    widget::{Space, button, column, container, row, text, tooltip},
};

pub fn divider<'a>() -> iced::widget::Rule<'a> {
    iced::widget::rule::horizontal(1).style(|t| iced::widget::rule::Style {
        color: theme::BORDER,
        ..iced::widget::rule::default(t)
    })
}
pub fn heading_font() -> Font {
    Font {
        weight: iced::font::Weight::Semibold,
        ..Font::DEFAULT
    }
}
pub fn muted<'a>(label: impl iced::advanced::text::IntoFragment<'a>) -> iced::widget::Text<'a> {
    text(label).size(12).color(theme::MUTED)
}
pub fn surface<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> iced::widget::Container<'a, M> {
    container(content).style(theme::panel)
}
pub fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> iced::widget::Container<'a, M> {
    container(content)
        .padding(18)
        .width(Fill)
        .style(|_| theme::box_style(theme::CANVAS, 14.0))
}
pub fn action<'a, M: Clone + 'a>(kind: Icon, hint: &'a str, message: M) -> Element<'a, M> {
    tooltip(
        button(icon(kind, 17.0))
            .padding(8)
            .style(theme::quiet)
            .on_press(message),
        container(text(hint).size(12))
            .padding(7)
            .style(theme::popover),
        tooltip::Position::Bottom,
    )
    .into()
}
pub fn chip<'a, M: Clone + 'a>(label: &'a str, active: bool, message: M) -> Element<'a, M> {
    button(text(label).size(13))
        .padding([7, 12])
        .style(if active {
            theme::selected
        } else {
            theme::quiet
        })
        .on_press(message)
        .into()
}
pub fn menu_item<'a, M: Clone + 'a>(
    kind: Icon,
    label: &'a str,
    key: &'a str,
    message: Option<M>,
) -> Element<'a, M> {
    let enabled = message.is_some();
    button(
        row![
            icon(kind, 16.0),
            text(label).size(13).width(Fill),
            text(key).size(11).font(Font::MONOSPACE)
        ]
        .spacing(12)
        .align_y(iced::Center),
    )
    .on_press_maybe(message)
    .padding([10, 12])
    .width(Fill)
    .style(move |t, s| {
        let mut style = theme::quiet(t, s);
        if !enabled {
            style.text_color = theme::MUTED;
        }
        style
    })
    .into()
}
pub fn section<'a, M: 'a>(number: &'a str, title: &'a str, description: &'a str) -> Element<'a, M> {
    column![
        text(number)
            .font(Font::MONOSPACE)
            .size(13)
            .color(theme::ACCENT),
        text(title).size(22).font(heading_font()).color(theme::INK),
        text(description).size(14).color(theme::INK),
        Space::new().height(10)
    ]
    .spacing(12)
    .into()
}
