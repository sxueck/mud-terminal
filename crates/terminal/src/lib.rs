//! Phase 1 terminal domain model.
use std::fmt;
use std::path::PathBuf;

/// Where a terminal session runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionTarget {
    /// A local session rooted at a directory.
    Local { directory: PathBuf },
    /// A remote session on a host, rooted at a directory.
    Remote { host: String, directory: String },
}

impl SessionTarget {
    pub fn is_remote(&self) -> bool {
        matches!(self, SessionTarget::Remote { .. })
    }
}

impl fmt::Display for SessionTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionTarget::Local { directory } => write!(f, "{}", directory.display()),
            SessionTarget::Remote { host, directory } => write!(f, "{}:{}", host, directory),
        }
    }
}

/// A terminal session shell around input and a paste draft.
/// Produces no fake terminal output; rendering is a later phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSession {
    target: SessionTarget,
    input: String,
    transcript: String,
}

impl TerminalSession {
    pub fn new(target: SessionTarget) -> Self {
        Self {
            target,
            input: String::new(),
            transcript: String::new(),
        }
    }

    pub fn target(&self) -> &SessionTarget {
        &self.target
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn transcript(&self) -> &str {
        &self.transcript
    }

    /// Drafts a paste into the input buffer. Does not execute anything.
    pub fn append_paste(&mut self, paste: &str) {
        self.input.push_str(paste);
    }

    /// Clears the draft (paste/input preview). Transcript is untouched.
    pub fn clear_preview(&mut self) {
        self.input.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_target_is_not_remote_and_displays_directory() {
        let t = SessionTarget::Local {
            directory: PathBuf::from("/tmp"),
        };
        assert!(!t.is_remote());
        assert_eq!(t.to_string(), "/tmp");
    }

    #[test]
    fn remote_target_is_remote_and_displays_host_and_directory() {
        let t = SessionTarget::Remote {
            host: "box".into(),
            directory: "/home/u".into(),
        };
        assert!(t.is_remote());
        assert_eq!(t.to_string(), "box:/home/u");
    }

    #[test]
    fn paste_drafts_into_input_without_executing() {
        let mut s = TerminalSession::new(SessionTarget::Local {
            directory: PathBuf::from("/tmp"),
        });
        s.append_paste("echo hi");
        assert_eq!(s.input(), "echo hi");
        assert_eq!(s.transcript(), "");
    }

    #[test]
    fn clear_preview_clears_input_only() {
        let mut s = TerminalSession::new(SessionTarget::Local {
            directory: PathBuf::from("/tmp"),
        });
        s.append_paste("ls");
        s.clear_preview();
        assert_eq!(s.input(), "");
        assert_eq!(s.transcript(), "");
    }

    #[test]
    fn target_is_accessible() {
        let s = TerminalSession::new(SessionTarget::Remote {
            host: "h".into(),
            directory: "/".into(),
        });
        assert!(s.target().is_remote());
    }
}
