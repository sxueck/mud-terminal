//! Phase 1 editor domain model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    title: String,
    text: String,
}

impl Document {
    /// Build a document from terminal scrollback content. No disk I/O.
    pub fn from_scrollback(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            text: text.into(),
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_from_scrollback_and_exposes_getters() {
        let d = Document::from_scrollback("session.log", "hello\nworld");
        assert_eq!(d.title(), "session.log");
        assert_eq!(d.text(), "hello\nworld");
    }

    #[test]
    fn empty_document() {
        let d = Document::from_scrollback("", "");
        assert_eq!(d.title(), "");
        assert_eq!(d.text(), "");
    }
}
