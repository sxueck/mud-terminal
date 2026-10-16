//! Builds the UI [`Screen`] snapshot from the alacritty terminal state.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

use crate::model::{Cell, RenderMode, Screen};

const DEFAULT_FOREGROUND: [u8; 3] = [0x1f, 0x23, 0x28];
/// Default background (white canvas).
const DEFAULT_BACKGROUND: [u8; 3] = [0xff, 0xff, 0xff];

#[derive(Clone, Copy, Debug)]
pub(crate) struct TermSize {
    pub columns: usize,
    pub rows: usize,
}

impl alacritty_terminal::grid::Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

const NAMED: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00], // black
    [0xaa, 0x00, 0x00], // red
    [0x00, 0xaa, 0x00], // green
    [0xaa, 0x55, 0x00], // yellow
    [0x00, 0x00, 0xaa], // blue
    [0xaa, 0x00, 0xaa], // magenta
    [0x00, 0xaa, 0xaa], // cyan
    [0xaa, 0xaa, 0xaa], // white
    [0x55, 0x55, 0x55], // bright black
    [0xff, 0x55, 0x55], // bright red
    [0x55, 0xff, 0x55], // bright green
    [0xff, 0xff, 0x55], // bright yellow
    [0x55, 0x55, 0xff], // bright blue
    [0xff, 0x55, 0xff], // bright magenta
    [0x55, 0xff, 0xff], // bright cyan
    [0xff, 0xff, 0xff], // bright white
];

fn named_rgb(color: NamedColor) -> [u8; 3] {
    match color {
        NamedColor::Foreground | NamedColor::BrightForeground => DEFAULT_FOREGROUND,
        NamedColor::DimForeground => DEFAULT_FOREGROUND,
        NamedColor::Background => DEFAULT_BACKGROUND,
        NamedColor::Cursor => DEFAULT_FOREGROUND,
        NamedColor::DimBlack => NAMED[0],
        NamedColor::DimRed => NAMED[1],
        NamedColor::DimGreen => NAMED[2],
        NamedColor::DimYellow => NAMED[3],
        NamedColor::DimBlue => NAMED[4],
        NamedColor::DimMagenta => NAMED[5],
        NamedColor::DimCyan => NAMED[6],
        NamedColor::DimWhite => NAMED[7],
        NamedColor::Black => NAMED[0],
        NamedColor::Red => NAMED[1],
        NamedColor::Green => NAMED[2],
        NamedColor::Yellow => NAMED[3],
        NamedColor::Blue => NAMED[4],
        NamedColor::Magenta => NAMED[5],
        NamedColor::Cyan => NAMED[6],
        NamedColor::White => NAMED[7],
        NamedColor::BrightBlack => NAMED[8],
        NamedColor::BrightRed => NAMED[9],
        NamedColor::BrightGreen => NAMED[10],
        NamedColor::BrightYellow => NAMED[11],
        NamedColor::BrightBlue => NAMED[12],
        NamedColor::BrightMagenta => NAMED[13],
        NamedColor::BrightCyan => NAMED[14],
        NamedColor::BrightWhite => NAMED[15],
    }
}

fn indexed_rgb(index: u8) -> [u8; 3] {
    match index {
        0..=15 => NAMED[index as usize],
        16..=231 => {
            // 6x6x6 color cube.
            let index = index as usize - 16;
            let step = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
            [
                step[(index / 36) % 6],
                step[(index / 6) % 6],
                step[index % 6],
            ]
        }
        232..=255 => {
            // Grayscale ramp.
            let gray = 8 + (index as usize - 232) * 10;
            [gray as u8, gray as u8, gray as u8]
        }
    }
}

fn color_rgb(color: &Color) -> [u8; 3] {
    match color {
        Color::Spec(Rgb { r, g, b }) => [*r, *g, *b],
        Color::Named(named) => named_rgb(*named),
        Color::Indexed(index) => indexed_rgb(*index),
    }
}

pub fn build_screen<T>(term: &Term<T>, prompt_line: Option<Line>) -> Screen
where
    T: alacritty_terminal::event::EventListener,
{
    let mode = *term.mode();
    let columns = term.columns();
    let rows = term.screen_lines();
    let grid = term.grid();

    let mut cells = Vec::with_capacity(rows * columns);
    let mut last_text_row = 0usize;
    for row in 0..rows {
        let line = &grid[Line(row as i32)];
        for column in 0..columns {
            let cell = &line[Column(column)];
            let mut text = String::with_capacity(1);
            text.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                for combining in zerowidth {
                    text.push(*combining);
                }
            }
            if !text.trim().is_empty() {
                last_text_row = row;
            }
            cells.push(Cell {
                row,
                column,
                text,
                foreground: color_rgb(if cell.flags.contains(Flags::INVERSE) {
                    &cell.bg
                } else {
                    &cell.fg
                }),
                background: color_rgb(if cell.flags.contains(Flags::INVERSE) {
                    &cell.fg
                } else {
                    &cell.bg
                }),
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.contains(Flags::UNDERLINE),
                wide_spacer: cell.flags.contains(Flags::WIDE_CHAR_SPACER),
            });
        }
    }

    let cursor = if mode.contains(TermMode::SHOW_CURSOR) {
        let point = grid.cursor.point;
        let column = point.column.0.min(columns.saturating_sub(1));
        let row = point.line.0.max(0).min(rows as i32 - 1) as usize;
        Some((column, row))
    } else {
        None
    };

    let alt_screen = mode.contains(TermMode::ALT_SCREEN);
    // A TUI owns the whole viewport, so only the Blocks view crops. The prompt
    // line is absolute; the viewport top is `-display_offset`.
    let hidden_rows = if alt_screen {
        0
    } else {
        prompt_line
            .map(|line| {
                (line.0 + term.grid().display_offset() as i32).clamp(0, rows as i32) as usize
            })
            .unwrap_or(0)
    };
    // A TUI owns every row; the Blocks tail stops at the last row with content.
    let live_rows = if alt_screen {
        rows
    } else {
        cursor
            .map(|(_, row)| row)
            .unwrap_or(0)
            .max(last_text_row)
            .saturating_add(1)
            .saturating_sub(hidden_rows)
            .max(1)
    };

    Screen {
        columns,
        rows,
        cells,
        cursor,
        hidden_rows,
        live_rows,
        mode: if alt_screen {
            RenderMode::Fullscreen
        } else {
            RenderMode::Blocks
        },
        application_cursor: mode.contains(TermMode::APP_CURSOR),
        bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        mouse_tracking: mode.intersects(
            TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION,
        ),
        mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
        mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
        mouse_sgr: mode.contains(TermMode::SGR_MOUSE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;

    fn term_for() -> Term<VoidListener> {
        Term::new(
            Config::default(),
            &TermSize {
                columns: 80,
                rows: 24,
            },
            VoidListener,
        )
    }

    fn screen_for(bytes: &[u8]) -> Screen {
        let mut term = term_for();
        let mut processor: Processor = Processor::new();
        processor.advance(&mut term, bytes);
        build_screen(&term, None)
    }

    fn screen_for_chunked(bytes: &[u8], chunk: usize) -> Screen {
        let mut term = term_for();
        let mut processor: Processor = Processor::new();
        for piece in bytes.chunks(chunk.max(1)) {
            processor.advance(&mut term, piece);
        }
        build_screen(&term, None)
    }

    #[test]
    fn hidden_rows_crop_to_the_prompt_line() {
        let mut term = term_for();
        let mut processor: Processor = Processor::new();
        processor.advance(&mut term, b"cmd output\r\n");
        let prompt = term.grid().cursor.point.line;
        processor.advance(&mut term, b"~/proj $ ls\r\n");
        let screen = build_screen(&term, Some(prompt));
        assert_eq!(screen.hidden_rows, 1);
        // Rows 1..2 hold text; the rest of the 24-row screen is empty and must
        // not stretch the live tail.
        assert_eq!(screen.live_rows, 2);
        assert_eq!(cell_text_at(&screen, 0, screen.hidden_rows - 1), "c");
        assert_eq!(cell_text_at(&screen, 0, screen.hidden_rows), "~");
    }

    #[test]
    fn fullscreen_never_crops() {
        let mut term = term_for();
        let mut processor: Processor = Processor::new();
        processor.advance(&mut term, b"\x1b[?1049hmid\r\n");
        let prompt = term.grid().cursor.point.line;
        let screen = build_screen(&term, Some(prompt));
        assert_eq!(screen.mode, RenderMode::Fullscreen);
        assert_eq!(screen.hidden_rows, 0);
        assert_eq!(screen.live_rows, screen.rows);
    }

    fn cell_text_at(screen: &Screen, column: usize, row: usize) -> String {
        screen
            .cells
            .iter()
            .find(|cell| cell.row == row && cell.column == column)
            .map(|cell| cell.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn bounds_match_dimensions() {
        let screen = screen_for(b"hello");
        assert_eq!(screen.cells.len(), screen.rows * screen.columns);
        assert!(screen.rows > 0 && screen.columns > 0);
        for cell in &screen.cells {
            assert!(cell.row < screen.rows);
            assert!(cell.column < screen.columns);
        }
    }

    #[test]
    fn text_lands_in_grid_with_cursor() {
        let screen = screen_for(b"abc");
        assert_eq!(cell_text_at(&screen, 0, 0), "a");
        assert_eq!(cell_text_at(&screen, 1, 0), "b");
        assert_eq!(cell_text_at(&screen, 2, 0), "c");
        assert_eq!(screen.cursor, Some((3, 0)));
        assert_eq!(screen.mode, RenderMode::Blocks);
    }

    #[test]
    fn truecolor_foreground_and_background() {
        let screen = screen_for(b"\x1b[38;2;10;20;30;48;2;200;210;220mX");
        let cell = screen
            .cells
            .iter()
            .find(|c| c.text == "X")
            .expect("X rendered");
        assert_eq!(cell.foreground, [10, 20, 30]);
        assert_eq!(cell.background, [200, 210, 220]);
    }

    #[test]
    fn indexed_and_named_colors_map_to_palette() {
        let screen = screen_for(b"\x1b[31;44mR");
        let cell = screen
            .cells
            .iter()
            .find(|c| c.text == "R")
            .expect("R rendered");
        assert_eq!(cell.foreground, NAMED[1]);
        assert_eq!(cell.background, NAMED[4]);
    }

    #[test]
    fn bold_italic_underline_flags() {
        let screen = screen_for(b"\x1b[1;3;4mS");
        let cell = screen
            .cells
            .iter()
            .find(|c| c.text == "S")
            .expect("S rendered");
        assert!(cell.bold && cell.italic && cell.underline);
    }

    #[test]
    fn alt_screen_modes_47_1047_1049() {
        let mut translator = crate::text::AltCompatTranslator::default();
        for sequence in ["\x1b[?47h", "\x1b[?1047h", "\x1b[?1049h"] {
            let screen = screen_for(&translator.feed(sequence.as_bytes()));
            assert_eq!(screen.mode, RenderMode::Fullscreen, "sequence {sequence:?}");
        }
        // Leaving the alternate screen restores block mode.
        let screen = screen_for(&translator.feed(b"\x1b[?1049h\x1b[?1049l"));
        assert_eq!(screen.mode, RenderMode::Blocks);
        // 1049 must also save/restore the cursor and clear the alt screen.
        let screen = screen_for(&translator.feed(b"\x1b[?1049halt\x1b[?1049l"));
        let line: String = screen
            .cells
            .iter()
            .take(4)
            .map(|c| c.text.clone())
            .collect();
        assert_eq!(
            line.trim(),
            "",
            "alt screen content must not leak: {line:?}"
        );
        // 47/1047 chunk-split across feeds still translates.
        let mut split = crate::text::AltCompatTranslator::default();
        assert_eq!(split.feed(b"\x1b[?4"), Vec::<u8>::new());
        let screen = screen_for(&split.feed(b"7h"));
        assert_eq!(screen.mode, RenderMode::Fullscreen);
    }

    #[test]
    fn chunked_sequences_across_feeds() {
        let screen = screen_for_chunked(b"\x1b[?1049hX\x1b[38;2;1;2;3mY", 1);
        assert_eq!(screen.mode, RenderMode::Fullscreen);
        assert_eq!(cell_text_at(&screen, 0, 0), "X");
        let colored = screen
            .cells
            .iter()
            .find(|c| c.text == "Y")
            .expect("Y rendered");
        assert_eq!(colored.foreground, [1, 2, 3]);
    }

    #[test]
    fn mode_flags_reported() {
        let screen = screen_for(b"\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?1003h\x1b[?1006h");
        assert!(screen.application_cursor);
        assert!(screen.bracketed_paste);
        assert!(screen.mouse_tracking);
        assert!(screen.mouse_motion);
        assert!(screen.mouse_sgr);
    }

    #[test]
    fn alternate_screen_compat_handles_every_split_including_lone_escape() {
        for sequence in [b"\x1b[?47h".as_slice(), b"\x1b[?1047h", b"\x1b[?1049h"] {
            for split in 0..=sequence.len() {
                let mut translator = crate::text::AltCompatTranslator::default();
                let mut translated = translator.feed(&sequence[..split]);
                translated.extend(translator.feed(&sequence[split..]));
                assert_eq!(
                    screen_for(&translated).mode,
                    RenderMode::Fullscreen,
                    "split {split}"
                );
            }
        }
    }

    #[test]
    fn inverse_video_and_drag_mode_are_preserved() {
        let screen = screen_for(b"\x1b[31;44;7mX\x1b[?1002h");
        let cell = screen.cells.iter().find(|c| c.text == "X").expect("X");
        assert_eq!(cell.foreground, NAMED[4]);
        assert_eq!(cell.background, NAMED[1]);
        assert!(screen.mouse_drag);
        assert!(!screen.mouse_motion);
    }

    #[test]
    fn hide_cursor_reports_none() {
        let screen = screen_for(b"\x1b[?25lX");
        assert_eq!(screen.cursor, None);
    }

    #[test]
    fn wide_characters_get_spacer_cells() {
        let screen = screen_for("語".as_bytes());
        let spacer = screen
            .cells
            .iter()
            .find(|c| c.wide_spacer)
            .expect("spacer cell");
        assert_eq!(spacer.column, 1);
        assert_eq!(cell_text_at(&screen, 0, 0), "語");
    }
}
