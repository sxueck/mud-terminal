//! Standalone Iced 0.14 advanced widget rendering a GPU text terminal grid.
//!
//! Draws each [`mud_terminal::Cell`] of a [`mud_terminal::Screen`] with
//! per-cell RGB background, bold/italic/underline styling and a cursor
//! rectangle, using only `renderer.fill_quad` / `renderer.fill_text` (no
//! canvas). Emits [`Action`] values to the parent, which owns the PTY.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text;
use iced::advanced::widget::Tree;
use iced::advanced::widget::Widget;
use iced::advanced::{Clipboard, Shell};
use iced::{
    Background, Color, Element, Event, Length, Rectangle, Size, font::Weight, keyboard, mouse,
};

/// Fixed monospace metrics for a 14 px grid (stable, no measurement needed).
/// Published so the Blocks view can render command output with the same row
/// height and glyph size as the live tail.
pub const FONT_SIZE: f32 = 14.0;
pub const CELL_WIDTH: f32 = 8.4;
pub const CELL_HEIGHT: f32 = 17.0;
/// Blocks mode draws a caret instead of a full cell: the tail is a prompt the
/// user is typing into. Full-screen TUIs keep the cell-sized block, which
/// vim/htop need to show the cursor over a glyph.
const CURSOR_BAR: f32 = 2.0;
/// Default SGR-style black/white palette assumptions for skipping fills.
const DEFAULT_FG: [u8; 3] = [31, 35, 40];
const DEFAULT_BG: [u8; 3] = [255, 255, 255];

const MONO: iced::Font = mud_ui::fonts::MONO;
const MONO_BOLD: iced::Font = iced::Font {
    weight: Weight::Bold,
    ..MONO
};

#[derive(Clone, Debug)]
pub enum Action {
    Input(Vec<u8>),
    /// Grid geometry changed after layout settled.
    Resize {
        columns: u16,
        rows: u16,
    },
    Focus,
}

#[derive(Debug, Default)]
struct State {
    session: u64,
    modifiers: keyboard::Modifiers,
    preedit: Option<iced::advanced::input_method::Preedit>,
    /// Last resize we reported; guards against repeated emission loops.
    reported: Option<(u16, u16)>,
    held_button: Option<u8>,
    last_mouse_cell: Option<(u16, u16)>,
}

struct TerminalView<'a, M> {
    id: u64,
    screen: &'a mud_terminal::Screen,
    focused: bool,
    on_action: Box<dyn Fn(Action) -> M + 'a>,
}

/// Height the live tail needs. In Blocks mode the grid is embedded in the
/// block scrollable, so it is sized by content rather than by the pane.
pub fn content_height(screen: &mud_terminal::Screen) -> f32 {
    screen.live_rows.max(1) as f32 * CELL_HEIGHT
}

pub fn view<'a, M: Clone + 'a>(
    id: u64,
    screen: &'a mud_terminal::Screen,
    focused: bool,
    on_action: impl Fn(Action) -> M + 'a,
) -> Element<'a, M> {
    Element::from(TerminalView {
        id,
        screen,
        focused,
        on_action: Box::new(on_action),
    })
}

impl<'a, M> TerminalView<'a, M> {
    fn grid_of(bounds: Size) -> (u16, u16) {
        let columns = ((bounds.width / CELL_WIDTH).floor() as u16).max(1);
        let rows = ((bounds.height / CELL_HEIGHT).floor() as u16).max(1);
        (columns, rows)
    }

    fn local_cell(bounds: Rectangle, position: iced::Point, hidden: usize) -> Option<(u16, u16)> {
        let x = position.x - bounds.x;
        let y = position.y - bounds.y;
        if x < 0.0 || y < 0.0 || x >= bounds.width || y >= bounds.height {
            return None;
        }
        Some((
            ((x / CELL_WIDTH).floor() as u16).saturating_add(1),
            ((y / CELL_HEIGHT).floor() as usize + hidden) as u16 + 1,
        ))
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer> for TerminalView<'a, Message>
where
    Message: Clone + 'a,
    Renderer: text::Renderer<Font = iced::Font> + 'a,
{
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(Length::Fill).height(Length::Fill);
        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        if state.session != self.id {
            *state = State {
                session: self.id,
                ..State::default()
            };
        }

        // Debounced resize: computed here where real bounds are known; only
        // published when the value actually changed.
        // The Blocks grid is sized by its content inside the scrolling
        // document, so only columns may come from the box; the PTY keeps rows.
        let grid = match self.screen.mode {
            mud_terminal::RenderMode::Blocks => (
                ((bounds.width / CELL_WIDTH).floor() as u16).max(1),
                self.screen.rows.max(1) as u16,
            ),
            mud_terminal::RenderMode::Fullscreen => Self::grid_of(bounds.size()),
        };
        if state.reported != Some(grid) {
            state.reported = Some(grid);
            shell.publish((self.on_action)(Action::Resize {
                columns: grid.0,
                rows: grid.1,
            }));
        }

        let screen = self.screen;
        if self.focused
            && matches!(
                event,
                Event::Window(iced::window::Event::RedrawRequested(_))
            )
        {
            let (column, row) = screen.cursor.unwrap_or((0, 0));
            shell.request_input_method(&iced::advanced::input_method::InputMethod::Enabled {
                cursor: Rectangle::new(
                    iced::Point::new(
                        bounds.x + column as f32 * CELL_WIDTH,
                        bounds.y + row.saturating_sub(screen.hidden_rows) as f32 * CELL_HEIGHT,
                    ),
                    Size::new(CELL_WIDTH, CELL_HEIGHT),
                ),
                purpose: iced::advanced::input_method::Purpose::Terminal,
                preedit: state.preedit.clone(),
            });
        }
        if shell.is_event_captured() {
            return;
        }

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                if let Some(position) = cursor.position()
                    && let Some(cell) = Self::local_cell(bounds, position, screen.hidden_rows)
                {
                    shell.publish((self.on_action)(Action::Focus));
                    if *button != mouse::Button::Right || screen.mouse_tracking {
                        shell.capture_event();
                    }
                    if screen.mouse_tracking {
                        let b =
                            encode_button(*button, false, false) | mouse_modifiers(state.modifiers);
                        if let Some(bytes) = encode_mouse(screen, b, cell, false) {
                            shell.publish((self.on_action)(Action::Input(bytes)));
                        }
                        state.held_button = Some(b);
                        state.last_mouse_cell = Some(cell);
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                if state.held_button.is_some() {
                    if let Some(cell) = cursor
                        .position()
                        .and_then(|position| Self::local_cell(bounds, position, screen.hidden_rows))
                        .or(state.last_mouse_cell)
                        && screen.mouse_tracking
                    {
                        let b =
                            encode_button(*button, false, false) | mouse_modifiers(state.modifiers);
                        if let Some(bytes) = encode_mouse_report(screen, b, cell, false, true) {
                            shell.publish((self.on_action)(Action::Input(bytes)));
                        }
                    }
                    state.held_button = None;
                    state.last_mouse_cell = None;
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if !screen.mouse_tracking
                    || (!screen.mouse_motion && !(screen.mouse_drag && state.held_button.is_some()))
                {
                    return;
                }
                if let Some(cell) = Self::local_cell(bounds, *position, screen.hidden_rows) {
                    if state.last_mouse_cell == Some(cell) {
                        return; // debounce duplicate motion reports
                    }
                    let held =
                        (state.held_button.unwrap_or(3) & !28) | mouse_modifiers(state.modifiers);
                    if let Some(bytes) = encode_mouse(screen, held, cell, true) {
                        shell.publish((self.on_action)(Action::Input(bytes)));
                    }
                    state.last_mouse_cell = Some(cell);
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if !screen.mouse_tracking {
                    return;
                }
                if let Some(position) = cursor.position()
                    && let Some(cell) = Self::local_cell(bounds, position, screen.hidden_rows)
                {
                    let lines = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => *y,
                        mouse::ScrollDelta::Pixels { y, .. } => y / CELL_HEIGHT,
                    };
                    let step = lines.signum();
                    let mut captured = false;
                    for _ in 0..lines.abs().max(1.0) as usize {
                        let b =
                            (if step > 0.0 { 64 } else { 65 }) | mouse_modifiers(state.modifiers);
                        if let Some(bytes) = encode_mouse(screen, b, cell, false) {
                            shell.publish((self.on_action)(Action::Input(bytes)));
                            captured = true;
                        }
                    }
                    if captured {
                        shell.capture_event();
                    }
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                ..
            }) => {
                if !self.focused {
                    return;
                }
                // Never swallow application shortcuts (Command/Super).
                if modifiers.logo() || workspace_shortcut(*modifiers) {
                    return;
                }
                if let Some(bytes) = encode_key(key, *modifiers, screen.application_cursor) {
                    shell.publish((self.on_action)(Action::Input(bytes)));
                    shell.capture_event();
                } else if let Some(text) = text {
                    let mut bytes: Vec<u8> = Vec::new();
                    if modifiers.alt() {
                        bytes.push(0x1b);
                    }
                    bytes.extend_from_slice(text.as_bytes());
                    shell.publish((self.on_action)(Action::Input(bytes)));
                    shell.capture_event();
                }
            }
            Event::InputMethod(iced::advanced::input_method::Event::Preedit(text, selection))
                if self.focused =>
            {
                state.preedit = Some(iced::advanced::input_method::Preedit {
                    content: text.clone(),
                    selection: selection.clone(),
                    text_size: Some(iced::Pixels(FONT_SIZE)),
                });
                shell.request_redraw();
            }
            Event::InputMethod(iced::advanced::input_method::Event::Commit(text)) => {
                state.preedit = None;
                if self.focused && !text.is_empty() {
                    shell.publish((self.on_action)(Action::Input(text.as_bytes().to_vec())));
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(clip) = bounds.intersection(viewport) else {
            return;
        };

        renderer.fill_quad(
            Quad {
                bounds,
                border: Default::default(),
                shadow: Default::default(),
                ..Quad::default()
            },
            Color::from_rgb8(DEFAULT_BG[0], DEFAULT_BG[1], DEFAULT_BG[2]),
        );

        let baseline = CELL_HEIGHT * 0.82; // approximated ascent for 14 px

        let hidden = self.screen.hidden_rows;
        for cell in &self.screen.cells {
            if cell.row < hidden {
                continue;
            }
            let x = bounds.x + cell.column as f32 * CELL_WIDTH;
            let y = bounds.y + (cell.row - hidden) as f32 * CELL_HEIGHT;
            if x > clip.x + clip.width
                || x + CELL_WIDTH < clip.x
                || y > clip.y + clip.height
                || y + CELL_HEIGHT < clip.y
            {
                continue; // clipped viewport culling
            }
            let cell_bounds =
                Rectangle::new(iced::Point::new(x, y), Size::new(CELL_WIDTH, CELL_HEIGHT));
            if cell.background != DEFAULT_BG {
                renderer.fill_quad(
                    Quad {
                        bounds: cell_bounds,
                        ..Quad::default()
                    },
                    Background::Color(Color::from_rgb8(
                        cell.background[0],
                        cell.background[1],
                        cell.background[2],
                    )),
                );
            }
            if cell.text.is_empty() || cell.wide_spacer || cell.text == " " {
                continue;
            }
            let mut font = if cell.bold { MONO_BOLD } else { MONO };
            if cell.italic {
                font.style = iced::font::Style::Italic;
            }
            renderer.fill_text(
                text::Text {
                    content: cell.text.clone(),
                    bounds: cell_bounds.size(),
                    size: iced::Pixels(FONT_SIZE),
                    line_height: iced::widget::text::LineHeight::Absolute(iced::Pixels(
                        CELL_HEIGHT,
                    )),
                    font,
                    align_x: iced::alignment::Horizontal::Left.into(),
                    align_y: iced::alignment::Vertical::Top,
                    shaping: text::Shaping::Basic,
                    wrapping: text::Wrapping::None,
                },
                iced::Point::new(x, y + baseline - FONT_SIZE),
                Color::from_rgb8(cell.foreground[0], cell.foreground[1], cell.foreground[2]),
                clip,
            );
            // Underline drawn as a thin quad; italics via font style.
            if cell.underline {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle::new(
                            iced::Point::new(x, y + CELL_HEIGHT - 2.0),
                            Size::new(CELL_WIDTH, 1.0),
                        ),
                        ..Quad::default()
                    },
                    Color::from_rgb8(cell.foreground[0], cell.foreground[1], cell.foreground[2]),
                );
            }
        }

        if let Some((col, row)) = self.screen.cursor.filter(|_| self.focused) {
            let x = bounds.x + col as f32 * CELL_WIDTH;
            let y = bounds.y + row.saturating_sub(hidden) as f32 * CELL_HEIGHT;
            let width = match self.screen.mode {
                mud_terminal::RenderMode::Blocks => CURSOR_BAR,
                mud_terminal::RenderMode::Fullscreen => CELL_WIDTH,
            };
            let rect = Rectangle::new(iced::Point::new(x, y), Size::new(width, CELL_HEIGHT));
            if rect.intersects(&clip) {
                renderer.fill_quad(
                    Quad {
                        bounds: rect,
                        ..Quad::default()
                    },
                    Color::from_rgba8(DEFAULT_FG[0], DEFAULT_FG[1], DEFAULT_FG[2], 0.6),
                );
            }
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            if self.screen.mouse_tracking {
                mouse::Interaction::Crosshair
            } else {
                mouse::Interaction::Text
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a, Message> From<TerminalView<'a, Message>> for Element<'a, Message>
where
    Message: Clone + 'a,
{
    fn from(view: TerminalView<'a, Message>) -> Self {
        Element::new(view)
    }
}

/// Maps an Iced mouse button to the X10 button code (0 left, 1 middle,
/// 2 right; release is handled by callers; 64/65 wheel).
fn encode_button(button: mouse::Button, shift: bool, release: bool) -> u8 {
    let base = match button {
        mouse::Button::Left => 0,
        mouse::Button::Middle => 1,
        mouse::Button::Right => 2,
        mouse::Button::Back => 128,
        mouse::Button::Forward => 129,
        mouse::Button::Other(_) => 0,
    };
    let mut code = base;
    if shift {
        code += 4;
    }
    if release {
        code = 3;
    }
    code
}

/// Encodes a mouse report for the given tracking mode.
/// `motion` adds the motion bit (32).
fn encode_mouse(
    screen: &mud_terminal::Screen,
    button: u8,
    cell: (u16, u16),
    motion: bool,
) -> Option<Vec<u8>> {
    encode_mouse_report(screen, button, cell, motion, false)
}

fn encode_mouse_report(
    screen: &mud_terminal::Screen,
    button: u8,
    cell: (u16, u16),
    motion: bool,
    release: bool,
) -> Option<Vec<u8>> {
    if !screen.mouse_tracking {
        return None;
    }
    let b = if motion {
        button | 32
    } else if release && !screen.mouse_sgr {
        3
    } else {
        button
    };
    let (x, y) = cell;
    if screen.mouse_sgr {
        let action = if release { 'm' } else { 'M' };
        Some(format!("\x1b[<{b};{x};{y}{action}").into_bytes())
    } else {
        // Legacy X10: coordinates must fit into a single byte.
        if x > 222 || y > 222 {
            return None;
        }
        Some(vec![0x1b, b'[', b'M', 32 + b, 32 + x as u8, 32 + y as u8])
    }
}

fn mouse_modifiers(modifiers: keyboard::Modifiers) -> u8 {
    (u8::from(modifiers.shift()) << 2)
        | (u8::from(modifiers.alt()) << 3)
        | (u8::from(modifiers.control()) << 4)
}

pub fn workspace_shortcut(modifiers: keyboard::Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.logo() && !modifiers.alt() && !modifiers.control()
    } else {
        // Preserve Ctrl+A/D/K/W and the rest of readline on Windows/Linux.
        modifiers.control() && modifiers.shift() && !modifiers.alt()
    }
}

/// Modifier parameter (1 + bitmask) for CSI/SS3 escape sequences.
fn modifier_code(modifiers: keyboard::Modifiers) -> Option<u8> {
    let mut m = 0u8;
    if modifiers.shift() {
        m += 1;
    }
    if modifiers.alt() {
        m += 2;
    }
    if modifiers.control() {
        m += 4;
    }
    if m == 0 { None } else { Some(1 + m) }
}

/// Encodes a keypress to raw PTY input bytes. Returns `None` when the key
/// should be ignored (e.g. plain modifiers).
fn encode_key(
    key: &keyboard::Key,
    modifiers: keyboard::Modifiers,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    use keyboard::key::Named;
    let mods = modifier_code(modifiers);
    let csi = |suffix: &str, mods: Option<u8>| -> Vec<u8> {
        match mods {
            Some(m) if suffix.ends_with('~') => {
                format!("\x1b[{};{m}~", suffix.trim_end_matches('~')).into_bytes()
            }
            Some(m) => format!("\x1b[1;{m}{suffix}").into_bytes(),
            None => format!("\x1b[{suffix}").into_bytes(),
        }
    };
    match key.as_ref() {
        keyboard::Key::Named(named) => {
            let seq: Vec<u8> = match named {
                Named::Enter => b"\r".to_vec(),
                Named::Tab if modifiers.shift() => b"\x1b[Z".to_vec(),
                Named::Tab => b"\t".to_vec(),
                Named::Backspace => b"\x7f".to_vec(),
                Named::Escape => b"\x1b".to_vec(),
                Named::ArrowUp => {
                    if mods.is_some() {
                        csi("A", mods)
                    } else if app_cursor {
                        b"\x1bOA".to_vec()
                    } else {
                        b"\x1b[A".to_vec()
                    }
                }
                Named::ArrowDown => {
                    if mods.is_some() {
                        csi("B", mods)
                    } else if app_cursor {
                        b"\x1bOB".to_vec()
                    } else {
                        b"\x1b[B".to_vec()
                    }
                }
                Named::ArrowRight => {
                    if mods.is_some() {
                        csi("C", mods)
                    } else if app_cursor {
                        b"\x1bOC".to_vec()
                    } else {
                        b"\x1b[C".to_vec()
                    }
                }
                Named::ArrowLeft => {
                    if mods.is_some() {
                        csi("D", mods)
                    } else if app_cursor {
                        b"\x1bOD".to_vec()
                    } else {
                        b"\x1b[D".to_vec()
                    }
                }
                Named::Home => {
                    if mods.is_some() {
                        csi("H", mods)
                    } else if app_cursor {
                        b"\x1bOH".to_vec()
                    } else {
                        b"\x1b[H".to_vec()
                    }
                }
                Named::End => {
                    if mods.is_some() {
                        csi("F", mods)
                    } else if app_cursor {
                        b"\x1bOF".to_vec()
                    } else {
                        b"\x1b[F".to_vec()
                    }
                }
                Named::PageUp => csi("5~", mods),
                Named::PageDown => csi("6~", mods),
                Named::Insert => csi("2~", mods),
                Named::Delete => csi("3~", mods),
                Named::F1 => {
                    if mods.is_some() {
                        csi("P", mods)
                    } else {
                        b"\x1bOP".to_vec()
                    }
                }
                Named::F2 => {
                    if mods.is_some() {
                        csi("Q", mods)
                    } else {
                        b"\x1bOQ".to_vec()
                    }
                }
                Named::F3 => {
                    if mods.is_some() {
                        csi("R", mods)
                    } else {
                        b"\x1bOR".to_vec()
                    }
                }
                Named::F4 => {
                    if mods.is_some() {
                        csi("S", mods)
                    } else {
                        b"\x1bOS".to_vec()
                    }
                }
                Named::F5 => csi("15~", mods),
                Named::F6 => csi("17~", mods),
                Named::F7 => csi("18~", mods),
                Named::F8 => csi("19~", mods),
                Named::F9 => csi("20~", mods),
                Named::F10 => csi("21~", mods),
                Named::F11 => csi("23~", mods),
                Named::F12 => csi("24~", mods),
                _ => return None,
            };
            Some(seq)
        }
        keyboard::Key::Character(ch) => {
            let c = ch.chars().next()?;
            if modifiers.control() {
                if modifiers.alt() {
                    let mut v = vec![0x1b];
                    v.push(ctrl_byte(c));
                    return Some(v);
                }
                return Some(vec![ctrl_byte(c)]);
            }
            if modifiers.alt() {
                let mut v = vec![0x1b];
                let mut buf = [0u8; 4];
                v.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                return Some(v);
            }
            // Plain characters are delivered via KeyPressed `text`
            // (UTF-8, shift-aware); report None so that path runs.
            None
        }
        keyboard::Key::Unidentified => None,
    }
}

/// Control-code byte for Ctrl + letter/space/@ etc.
fn ctrl_byte(c: char) -> u8 {
    let code = c as u32;
    match code {
        0x40..=0x5f => (code - 0x40) as u8, // @ A-Z [ \ ] ^ _
        0x61..=0x7a => (code - 0x60) as u8, // a-z
        0x20 => 0x00,                       // Ctrl-Space -> NUL
        0x3f => 0x7f,                       // Ctrl-? -> DEL
        _ => (code & 0x1f) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ch: &str) -> keyboard::Key {
        keyboard::Key::Character(ch.into())
    }

    #[test]
    fn ctrl_characters() {
        let none = keyboard::Modifiers::empty();
        let ctrl = keyboard::Modifiers::CTRL;
        assert_eq!(encode_key(&key("c"), ctrl, false), Some(vec![0x03]));
        assert_eq!(encode_key(&key("z"), ctrl, false), Some(vec![0x1a]));
        assert_eq!(encode_key(&key("d"), ctrl, false), Some(vec![0x04]));
        assert_eq!(encode_key(&key(" "), ctrl, false), Some(vec![0x00]));
        assert_eq!(encode_key(&key("["), ctrl, false), Some(vec![0x1b]));
        // Alt acts as an ESC prefix.
        assert_eq!(
            encode_key(&key("a"), keyboard::Modifiers::ALT, false),
            Some(vec![0x1b, b'a'])
        );
        assert_eq!(
            encode_key(&key("c"), ctrl | keyboard::Modifiers::ALT, false),
            Some(vec![0x1b, 0x03])
        );
        // Plain characters fall through to the `text` path.
        assert_eq!(encode_key(&key("a"), none, false), None);
    }

    #[test]
    fn named_keys() {
        use keyboard::key::Named;
        let none = keyboard::Modifiers::empty();
        let named = |n: Named| keyboard::Key::Named(n);
        assert_eq!(
            encode_key(&named(Named::Enter), none, false),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::Backspace), none, false),
            Some(b"\x7f".to_vec())
        );
        // Normal cursor mode vs application cursor mode.
        assert_eq!(
            encode_key(&named(Named::ArrowUp), none, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::ArrowUp), none, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::Home), none, true),
            Some(b"\x1bOH".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::ArrowLeft), keyboard::Modifiers::SHIFT, true),
            Some(b"\x1b[1;2D".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::Delete), none, false),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::PageUp), none, false),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::F5), none, false),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::F12), none, false),
            Some(b"\x1b[24~".to_vec())
        );
        assert_eq!(
            encode_key(&named(Named::F1), none, false),
            Some(b"\x1bOP".to_vec())
        );
    }

    fn screen(sgr: bool, motion: bool) -> mud_terminal::Screen {
        mud_terminal::Screen {
            columns: 80,
            rows: 24,
            cells: Vec::new(),
            cursor: None,
            hidden_rows: 0,
            live_rows: 24,
            mode: mud_terminal::RenderMode::Fullscreen,
            application_cursor: false,
            bracketed_paste: false,
            mouse_tracking: true,
            mouse_motion: motion,
            mouse_drag: false,
            mouse_sgr: sgr,
        }
    }

    #[test]
    fn mouse_sgr_encoding() {
        let s = screen(true, true);
        assert_eq!(
            encode_mouse(&s, 0, (11, 22), false),
            Some(b"\x1b[<0;11;22M".to_vec())
        );
        // SGR release uses lowercase 'm'.
        assert_eq!(
            encode_mouse_report(&s, 0, (5, 6), false, true),
            Some(b"\x1b[<0;5;6m".to_vec())
        );
        // Motion adds the 32 bit.
        assert_eq!(
            encode_mouse(&s, 0, (1, 1), true),
            Some(b"\x1b[<32;1;1M".to_vec())
        );
        // Wheel.
        assert_eq!(
            encode_mouse(&s, 64, (2, 3), false),
            Some(b"\x1b[<64;2;3M".to_vec())
        );
    }

    #[test]
    fn mouse_legacy_encoding() {
        let s = screen(false, true);
        assert_eq!(
            encode_mouse(&s, 0, (1, 1), false),
            Some(b"\x1b[M !!".to_vec())
        );
        assert_eq!(
            encode_mouse(&s, 3, (10, 5), false),
            Some(b"\x1b[M#*%".to_vec())
        );
        // Legacy motion on button 1.
        assert_eq!(
            encode_mouse(&s, 0, (2, 2), true),
            Some(b"\x1b[M@\"\"".to_vec())
        );
        // Out-of-range coordinates are dropped instead of wrapping.
        assert_eq!(encode_mouse(&s, 0, (250, 2), false), None);
        // Tracking disabled reports nothing.
        let off = screen(true, true);
        let off = mud_terminal::Screen {
            mouse_tracking: false,
            ..off
        };
        assert_eq!(encode_mouse(&off, 0, (1, 1), false), None);
    }

    #[test]
    fn button_encoding() {
        assert_eq!(encode_button(mouse::Button::Left, false, false), 0);
        assert_eq!(encode_button(mouse::Button::Middle, false, false), 1);
        assert_eq!(encode_button(mouse::Button::Right, false, false), 2);
        assert_eq!(encode_button(mouse::Button::Right, true, false), 6);
        assert_eq!(encode_button(mouse::Button::Left, false, true), 3);
    }
}
