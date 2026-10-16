use iced::{
    Border, Color, Shadow, Vector,
    widget::{button, container, text_input},
};

pub const WINDOW: Color = Color::from_rgb8(211, 214, 220);
pub const PANEL: Color = Color::from_rgb8(249, 250, 251);
pub const CANVAS: Color = Color::WHITE;
pub const INK: Color = Color::from_rgb8(62, 71, 88);
pub const MUTED: Color = Color::from_rgb8(125, 136, 153);
pub const BORDER: Color = Color::from_rgb8(229, 231, 235);
pub const ACCENT: Color = Color::from_rgb8(100, 150, 153);
pub const SELECTED: Color = Color::from_rgb8(232, 240, 240);
pub const DANGER: Color = Color::from_rgb8(175, 65, 65);

/// A borderless window clears its alpha surface with the theme's extended
/// background colour, which is opaque by default and would square off the
/// corners. Zeroing only that one colour keeps every derived colour from Light
/// while letting [`window_frame`]'s radius define the real silhouette.
pub fn theme() -> iced::Theme {
    let light = iced::Theme::Light.palette();
    let mut extended = iced::theme::palette::Extended::generate(light);
    extended.background.base.color = Color::TRANSPARENT;
    iced::Theme::custom_with_fn("mud", light, move |_| extended)
}

/// Borderless window silhouette. Everything outside this radius is the alpha
/// surface, so the transparent window reads as a rounded frame.
pub const WINDOW_RADIUS: f32 = 12.0;
pub fn window_frame(_: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(WINDOW.into()),
        text_color: Some(INK),
        border: Border {
            color: Color::from_rgba8(62, 71, 88, 0.26),
            width: 1.0,
            radius: WINDOW_RADIUS.into(),
        },
        ..Default::default()
    }
}
pub fn panel(_: &iced::Theme) -> container::Style {
    box_style(PANEL, 10.0)
}
/// Tone-only panel: no border, no radius. Chrome separation comes from the
/// surface colour, not from drawn boxes.
pub fn flat(_: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(PANEL.into()),
        text_color: Some(INK),
        ..Default::default()
    }
}
pub fn canvas(_: &iced::Theme) -> container::Style {
    box_style(CANVAS, 10.0)
}
pub fn box_style(background: Color, radius: f32) -> container::Style {
    container::Style {
        background: Some(background.into()),
        text_color: Some(INK),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: radius.into(),
        },
        ..Default::default()
    }
}
pub fn popover(_: &iced::Theme) -> container::Style {
    container::Style {
        shadow: Shadow {
            color: Color::from_rgba8(28, 35, 48, 0.18),
            offset: Vector::new(0.0, 14.0),
            blur_radius: 36.0,
        },
        ..box_style(CANVAS, 14.0)
    }
}
pub fn quiet(_: &iced::Theme, status: button::Status) -> button::Style {
    button::Style {
        text_color: INK,
        background: match status {
            button::Status::Hovered | button::Status::Pressed => {
                Some(Color::from_rgb8(238, 240, 243).into())
            }
            _ => None,
        },
        border: Border {
            radius: 6.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
pub fn selected(_: &iced::Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(SELECTED.into()),
        text_color: ACCENT,
        ..quiet(&iced::Theme::Light, status)
    }
}
pub fn input(theme: &iced::Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    style.background = CANVAS.into();
    style.border = Border {
        color: if matches!(status, text_input::Status::Focused { .. }) {
            ACCENT
        } else {
            BORDER
        },
        width: 1.0,
        radius: 10.0.into(),
    };
    style.value = INK;
    style.placeholder = MUTED;
    style
}
