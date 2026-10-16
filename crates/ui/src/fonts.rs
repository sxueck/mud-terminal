//! The one monospace face used for every terminal-shaped text.
//!
//! `iced::Font::MONOSPACE` asks cosmic-text for `Family::Monospace`, and its
//! fallback then fails on any glyph outside that family's coverage: a CJK rune
//! is laid out but never drawn, and it forces a line break where it sits —
//! which is why `ls -l` output looked double-spaced on a Chinese locale. A
//! concrete family *name* keeps the ASCII metrics and restores fallback, so
//! wide characters render inline.

use iced::Font;

/// Platform monospace face. Menlo's 14 px advance is 8.4 px, which is the cell
/// width the grid renderer hard-codes.
#[cfg(target_os = "macos")]
const MONO_NAME: &str = "Menlo";
#[cfg(windows)]
const MONO_NAME: &str = "Consolas";
#[cfg(not(any(target_os = "macos", windows)))]
const MONO_NAME: &str = "monospace";

pub const MONO: Font = Font::with_name(MONO_NAME);

#[cfg(test)]
mod tests {
    use super::MONO;
    use iced::font::Family;

    /// `Family::Monospace` makes cosmic-text drop CJK glyphs and break the line
    /// where they sit, so terminal text must always name a concrete face.
    #[test]
    fn mono_names_a_family_instead_of_asking_for_monospace() {
        assert!(matches!(MONO.family, Family::Name(name) if !name.is_empty()));
    }
}
