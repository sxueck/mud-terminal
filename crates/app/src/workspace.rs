use iced::widget::pane_grid::{self, Axis, Pane};
use mud_agent::LaunchKind;
use mud_editor::Document;
use mud_terminal::{SessionTarget, TerminalSession};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum PaneBody {
    Terminal {
        session: TerminalSession,
        kind: LaunchKind,
    },
    Editor {
        document: Document,
        target: SessionTarget,
    },
}

impl PaneBody {
    pub fn terminal(target: SessionTarget, kind: LaunchKind) -> Self {
        Self::Terminal {
            session: TerminalSession::new(target),
            kind,
        }
    }

    pub fn target(&self) -> &SessionTarget {
        match self {
            Self::Terminal { session, .. } => session.target(),
            Self::Editor { target, .. } => target,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Terminal {
                kind: LaunchKind::Shell,
                ..
            } => "~",
            Self::Terminal {
                kind: LaunchKind::Agent,
                ..
            } => "Agent",
            Self::Editor { document, .. } => document.title(),
        }
    }
}

#[derive(Debug)]
pub struct Tab {
    pub id: u64,
    pub panes: pane_grid::State<PaneBody>,
    pub focus: Pane,
}

impl Tab {
    fn new(id: u64, body: PaneBody) -> Self {
        let (panes, focus) = pane_grid::State::new(body);
        Self { id, panes, focus }
    }

    pub fn focused(&self) -> Option<&PaneBody> {
        self.panes.get(self.focus)
    }
}

#[derive(Debug)]
pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub root: PathBuf,
    next_id: u64,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Self {
        let mut workspace = Self {
            tabs: Vec::new(),
            active: 0,
            root,
            next_id: 0,
        };
        workspace.add_root_tab();
        workspace
    }

    /// The tab a workspace always keeps: a local shell rooted at the workspace directory.
    fn add_root_tab(&mut self) {
        self.add(PaneBody::terminal(
            SessionTarget::Local {
                directory: self.root.clone(),
            },
            LaunchKind::Shell,
        ));
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    pub fn is_remote(&self) -> bool {
        self.tab()
            .focused()
            .is_some_and(|body| body.target().is_remote())
    }
    pub fn activate(&mut self, id: u64) {
        if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
            self.active = index;
        }
    }
    pub fn add(&mut self, body: PaneBody) {
        self.tabs.push(Tab::new(self.next_id, body));
        self.next_id += 1;
        self.active = self.tabs.len() - 1;
    }
    pub fn new_terminal(&mut self, kind: LaunchKind, home: Option<PathBuf>) {
        let current = self.tab().focused().map(|p| p.target().clone());
        // A local Home preference must never switch a remote session to local execution.
        let target = match current {
            Some(remote @ SessionTarget::Remote { .. }) => remote,
            current => home
                .map(|directory| SessionTarget::Local { directory })
                .or(current)
                .unwrap_or_else(|| SessionTarget::Local {
                    directory: self.root.clone(),
                }),
        };
        self.add(PaneBody::terminal(target, kind));
    }
    pub fn close_tab(&mut self, id: u64) {
        if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
            let active_id = self.tab().id;
            self.tabs.remove(index);
            if self.tabs.is_empty() {
                self.add_root_tab();
            } else {
                self.active = self
                    .tabs
                    .iter()
                    .position(|t| t.id == active_id)
                    .unwrap_or(index.min(self.tabs.len() - 1));
            }
        }
    }
    pub fn split(&mut self, axis: Axis, kind: LaunchKind) {
        let Some(target) = self.tab().focused().map(|p| p.target().clone()) else {
            return;
        };
        let tab = self.tab_mut();
        if tab.panes.len() >= 16 {
            return;
        }
        tab.panes.restore();
        if let Some((pane, _)) = tab
            .panes
            .split(axis, tab.focus, PaneBody::terminal(target, kind))
        {
            tab.focus = pane;
        }
    }
    pub fn close_pane(&mut self) {
        let tab = self.tab_mut();
        if let Some((_, sibling)) = tab.panes.close(tab.focus) {
            tab.focus = sibling;
        } else {
            let id = tab.id;
            self.close_tab(id);
        }
    }
    pub fn move_to_tab(&mut self) {
        let tab = self.tab_mut();
        if let Some((body, sibling)) = tab.panes.close(tab.focus) {
            tab.focus = sibling;
            self.add(body);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn workspace() -> Workspace {
        Workspace::new(PathBuf::from("/tmp"))
    }
    #[test]
    fn recursive_splits_resize_close_and_move_preserve_focus() {
        let mut w = workspace();
        w.split(Axis::Vertical, LaunchKind::Shell);
        w.split(Axis::Horizontal, LaunchKind::Agent);
        assert_eq!(w.tab().panes.len(), 3);
        assert_eq!(w.tab().focused().map(PaneBody::title), Some("Agent"));
        w.move_to_tab();
        assert_eq!(w.tabs.len(), 2);
        assert_eq!(w.tabs[0].panes.len(), 2);
        assert_eq!(w.tab().focused().map(PaneBody::title), Some("Agent"));
        w.close_pane();
        assert_eq!(w.tabs.len(), 1);
        assert!(w.tab().focused().is_some());
    }
    #[test]
    fn last_pane_and_last_tab_leave_valid_workspace() {
        let mut w = workspace();
        w.close_pane();
        w.close_tab(w.tab().id);
        assert_eq!(w.tabs.len(), 1);
        assert!(w.tab().focused().is_some());
        w.move_to_tab();
        assert_eq!(w.tabs.len(), 1);
    }
    #[test]
    fn closing_background_tab_keeps_active_tab() {
        let mut w = workspace();
        w.new_terminal(LaunchKind::Agent, None);
        let active = w.tab().id;
        w.close_tab(0);
        assert_eq!(w.tab().id, active);
    }
    #[test]
    fn remote_scope_is_inherited_by_splits_and_editor() {
        let mut w = workspace();
        let target = SessionTarget::Remote {
            host: "prod".into(),
            directory: "/srv".into(),
        };
        w.add(PaneBody::terminal(target.clone(), LaunchKind::Shell));
        w.split(Axis::Vertical, LaunchKind::Agent);
        assert!(w.is_remote());
        assert!(w.tab().panes.iter().all(|(_, p)| p.target() == &target));
        w.add(PaneBody::Editor {
            document: Document::from_scrollback("log", ""),
            target,
        });
        assert!(w.is_remote());
        w.activate(0);
        assert!(!w.is_remote());
    }
    #[test]
    fn home_preference_cannot_change_remote_execution_scope() {
        let mut w = workspace();
        w.add(PaneBody::terminal(
            SessionTarget::Remote {
                host: "prod".into(),
                directory: "/srv".into(),
            },
            LaunchKind::Shell,
        ));
        w.new_terminal(LaunchKind::Shell, Some(PathBuf::from("/Users/local")));
        assert!(w.is_remote());
    }
    #[test]
    fn split_limit_bounds_layout_complexity() {
        let mut w = workspace();
        for _ in 0..30 {
            w.split(Axis::Vertical, LaunchKind::Shell);
        }
        assert_eq!(w.tab().panes.len(), 16);
    }
}
