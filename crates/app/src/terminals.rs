use crate::{
    terminal_view,
    workspace::{PaneBody, Workspace},
};
use iced::{
    Border, Color, Element, Fill, Pixels, Subscription, Task,
    futures::{SinkExt, Stream},
    widget::{
        Space, button, column, container, mouse_area, row, scrollable, stack, text, text_input,
    },
};
use mud_agent::LaunchKind;
use mud_terminal::{
    Block, PtyEvent, PtyHandle, RenderMode, Request, SessionTarget, Snapshot, TerminalSession,
};
use mud_ui::{self as ui, Icon, theme};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone, Debug)]
pub enum Event {
    Ready(Result<PtyHandle, String>),
    Backend(PtyEvent),
    Grid(terminal_view::Action),
    Find,
    Query(String),
    Match(i32),
    Select(u64),
    All,
    Step(i32),
    Copy(u64),
    Clear,
    Restart,
}

#[derive(Debug, Default)]
pub struct Live {
    pub handle: Option<PtyHandle>,
    pub snapshot: Option<Arc<Snapshot>>,
    pub error: Option<String>,
    pub finding: bool,
    query: String,
    match_index: usize,
    selected: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Terminals {
    pub live: BTreeMap<u64, Live>,
}

pub fn shell_ids(workspace: &Workspace) -> impl Iterator<Item = (u64, PathBuf)> + '_ {
    workspace
        .tabs
        .iter()
        .flat_map(|tab| tab.panes.iter())
        .filter_map(|(_, body)| {
            if let PaneBody::Terminal {
                session,
                kind: LaunchKind::Shell,
            } = body
                && let SessionTarget::Local { directory } = session.target()
            {
                Some((session.id(), directory.clone()))
            } else {
                None
            }
        })
}

impl Terminals {
    pub fn retain(&mut self, workspace: &Workspace) {
        let ids: BTreeSet<_> = shell_ids(workspace).map(|(id, _)| id).collect();
        self.live.retain(|id, _| ids.contains(id));
    }

    pub fn subscriptions(&self, workspace: &Workspace) -> Subscription<(u64, Event)> {
        Subscription::batch(
            shell_ids(workspace).map(|data| Subscription::run_with(data, terminal_stream)),
        )
    }

    pub fn transcript<'a>(&'a self, session: &'a TerminalSession) -> &'a str {
        self.live
            .get(&session.id())
            .and_then(|live| live.snapshot.as_ref())
            .map_or(session.transcript(), |s| s.transcript.as_str())
    }

    pub fn send(&mut self, id: u64, request: Request) {
        let live = self.live.entry(id).or_default();
        let result = live
            .handle
            .as_ref()
            .ok_or_else(|| "Shell is not ready".to_owned())
            .and_then(|handle| handle.send(request));
        if let Err(error) = result {
            live.error = Some(error);
        }
    }

    pub fn paste(&mut self, id: u64, contents: &str) {
        let bracketed = self
            .live
            .get(&id)
            .and_then(|live| live.snapshot.as_ref())
            .is_some_and(|s| s.screen.bracketed_paste);
        let contents = contents.replace('\x1b', "");
        let bytes = if bracketed {
            format!("\x1b[200~{contents}\x1b[201~").into_bytes()
        } else {
            contents
                .replace("\r\n", "\r")
                .replace('\n', "\r")
                .into_bytes()
        };
        self.send(id, Request::Input(bytes));
    }

    pub fn update(&mut self, id: u64, event: Event) -> Task<Event> {
        let live = self.live.entry(id).or_default();
        match event {
            Event::Ready(Ok(handle)) => {
                live.handle = Some(handle);
                live.error = None;
            }
            Event::Ready(Err(error)) | Event::Backend(PtyEvent::Error(error)) => {
                live.error = Some(error)
            }
            Event::Backend(PtyEvent::Snapshot(snapshot)) => {
                if snapshot.screen.mode == RenderMode::Fullscreen {
                    live.finding = false;
                }
                if live
                    .selected
                    .is_some_and(|id| !snapshot.blocks.iter().any(|b| b.id == id))
                {
                    live.selected = None;
                }
                live.snapshot = Some(snapshot);
            }
            Event::Grid(terminal_view::Action::Input(bytes)) => {
                self.send(id, Request::Input(bytes))
            }
            Event::Grid(terminal_view::Action::Resize { columns, rows }) => {
                self.send(id, Request::Resize { columns, rows })
            }
            Event::Grid(terminal_view::Action::Focus) => {
                return iced::widget::operation::focus("terminal-input");
            }
            Event::Find => {
                if live
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.screen.mode == RenderMode::Fullscreen)
                {
                    return Task::none();
                }
                live.finding = !live.finding;
                live.match_index = 0;
            }
            Event::Query(query) => {
                live.query = query;
                live.match_index = 0;
            }
            Event::Match(delta) => {
                let count = live.matches().len();
                if count > 0 {
                    live.match_index = (live.match_index as i64 + i64::from(delta))
                        .rem_euclid(count as i64) as usize;
                }
            }
            Event::Select(block) => live.selected = Some(block),
            Event::All => live.selected = None,
            Event::Step(delta) => {
                if let Some(snapshot) = &live.snapshot {
                    let blocks = &snapshot.blocks;
                    if !blocks.is_empty() {
                        let current = blocks
                            .iter()
                            .position(|b| Some(b.id) == live.selected)
                            .unwrap_or(blocks.len() - 1);
                        let index = (current as i64 + i64::from(delta))
                            .clamp(0, blocks.len() as i64 - 1)
                            as usize;
                        live.selected = Some(blocks[index].id);
                        return iced::widget::operation::scroll_to(
                            history_id(id),
                            scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
                        );
                    }
                }
            }
            Event::Copy(block) => {
                if let Some(block) = live
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.blocks.iter().find(|b| b.id == block))
                {
                    return iced::clipboard::write(block.output.clone());
                }
            }
            Event::Clear => {
                live.selected = None;
                live.match_index = 0;
                self.send(id, Request::Clear);
            }
            Event::Restart => {
                live.error = None;
                self.send(id, Request::Restart);
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, session: &'a TerminalSession, focused: bool) -> Element<'a, Event> {
        let id = session.id();
        let Some(live) = self.live.get(&id) else {
            return text("Starting local shell…").size(13).into();
        };
        let Some(snapshot) = &live.snapshot else {
            return column![
                text(live.error.as_deref().unwrap_or("Starting local shell…")).size(13)
            ]
            .into();
        };
        let grid = terminal_view::view(id, &snapshot.screen, focused && !live.finding, Event::Grid);
        if snapshot.screen.mode == RenderMode::Fullscreen {
            return grid;
        }
        let mut contents = column![].spacing(8);
        if let Some(error) = &live.error {
            contents = contents.push(text(error).color(theme::DANGER).size(12));
        }
        if let Some(warning) = &snapshot.warning {
            contents = contents.push(ui::muted(warning));
        }

        // One continuous document: finished blocks, then the live tail cropped
        // to the rows after the last prompt. History and live share a single
        // scroll, and the grid no longer repeats a finished block's output.
        let mut document = column![].spacing(2);
        for block in &snapshot.blocks {
            if live.selected.is_some_and(|id| id != block.id) {
                continue;
            }
            let selected = Some(block.id) == live.selected;
            document = document.push(
                mouse_area(
                    container(block_card(block, selected))
                        .padding([6, 8])
                        .width(Fill)
                        .style(move |_| container::Style {
                            background: selected.then_some(theme::SELECTED.into()),
                            border: Border {
                                color: if selected {
                                    theme::ACCENT
                                } else {
                                    Color::TRANSPARENT
                                },
                                width: 1.0,
                                radius: 8.0.into(),
                            },
                            text_color: Some(theme::INK),
                            ..Default::default()
                        }),
                )
                .on_press(Event::Select(block.id)),
            );
        }
        if let Some(running) = &snapshot.running {
            document = document.push(
                container(
                    row![
                        text("›")
                            .font(ui::MONO)
                            .size(BLOCK_FONT)
                            .color(ui::ACCENT)
                            .width(MARKER),
                        text(&running.command)
                            .font(ui::MONO)
                            .size(BLOCK_FONT)
                            .color(ui::ACCENT)
                            .width(Fill),
                        text("running").size(11).color(ui::MUTED),
                        Space::new().width(ACTION_WIDTH),
                    ]
                    .align_y(iced::Center),
                )
                .padding([6, 8])
                .width(Fill),
            );
        }
        if !snapshot.blocks.is_empty() {
            // Warp keeps block navigation off the document: bare glyphs, no label.
            let nav = |glyph: &'static str, message: Event, hint: &'static str| {
                ui::action_button(text(glyph).size(13).color(ui::MUTED), hint, message)
            };
            contents = contents.push(
                row![
                    Space::new().width(Fill),
                    nav("↑", Event::Step(-1), "Previous block · ⇧⌘↑"),
                    nav("↓", Event::Step(1), "Next block · ⇧⌘↓"),
                    nav("⊡", Event::All, "Show all blocks"),
                ]
                .spacing(2)
                .padding(iced::Padding {
                    bottom: 4.0,
                    ..Default::default()
                })
                .align_y(iced::Center),
            );
        }
        document = document.push(
            container(grid)
                .height(terminal_view::content_height(&snapshot.screen))
                .width(Fill),
        );
        contents = contents.push(
            scrollable(document)
                .id(history_id(id))
                .anchor_bottom()
                .height(Fill),
        );
        if snapshot.exited {
            contents = contents.push(
                row![
                    ui::muted("Shell exited"),
                    button(text("Restart shell").size(12))
                        .on_press(Event::Restart)
                        .style(theme::quiet)
                ]
                .spacing(8),
            );
        }
        let base: Element<'_, Event> = contents.into();
        if !live.finding {
            return base;
        }
        // Scrollback search floats in the top-right corner instead of pushing
        // the document down while you type.
        let matches = live.matches();
        let count = matches.len();
        let current = if count > 0 {
            live.match_index.min(count - 1) + 1
        } else {
            0
        };
        let mut card = column![
            row![
                text_input("Find in scrollback…", &live.query)
                    .id(format!("find-{id}"))
                    .on_input(Event::Query)
                    .on_submit(Event::Match(1))
                    .padding([6, 8])
                    .size(12)
                    .style(theme::input)
                    .width(Fill),
                text(format!("{current}/{count}")).size(11).color(ui::MUTED),
                button(text("↑").size(12))
                    .on_press(Event::Match(-1))
                    .style(theme::quiet),
                button(text("↓").size(12))
                    .on_press(Event::Match(1))
                    .style(theme::quiet),
                ui::action(Icon::Close, "Close search", Event::Find),
            ]
            .spacing(5)
            .align_y(iced::Center),
        ]
        .spacing(6);
        if let Some((line, result)) = matches.get(live.match_index.min(count.saturating_sub(1))) {
            card = card.push(
                text(format!("{}  {}", line + 1, result))
                    .font(ui::MONO)
                    .size(12)
                    .color(ui::MUTED),
            );
        }
        stack![
            base,
            iced::widget::float(container(card).padding(10).width(440).style(theme::popover))
                // The float already starts at the pane origin, so only x needs a
                // viewport-relative offset; y stays inside the pane.
                .translate(|bounds, viewport| {
                    iced::Vector::new(
                        (viewport.width - bounds.width - 28.0).max(0.0) - bounds.x,
                        8.0,
                    )
                })
        ]
        .into()
    }
}

impl Live {
    fn matches(&self) -> Vec<(usize, &str)> {
        if self.query.is_empty() {
            return Vec::new();
        }
        let query = self.query.to_lowercase();
        self.snapshot
            .as_ref()
            .map(|s| {
                s.transcript
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| line.to_lowercase().contains(&query))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn history_id(id: u64) -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::from(format!("blocks-{id}"))
}

/// Blocks are drawn with the grid's own metrics so finished output and the
/// live tail read as one continuous document.
const BLOCK_FONT: f32 = terminal_view::FONT_SIZE;
const BLOCK_LINE: f32 = terminal_view::CELL_HEIGHT;
/// `›` marker column; the command and its output share this left edge.
const MARKER: f32 = 18.0;

/// Reserved column for the per-block copy action, so rows never shift.
const ACTION_WIDTH: f32 = 27.0;

/// Warp keeps a block's status to the essentials: elapsed time, and the exit
/// code only when the command failed.
fn block_status(block: &Block) -> (String, Color) {
    let elapsed = block
        .duration
        .map(|d| format!("{:.2}s", d.as_secs_f64()))
        .unwrap_or_default();
    match block.exit_code {
        Some(0) => (elapsed, ui::MUTED),
        Some(code) => (
            if elapsed.is_empty() {
                format!("exit {code}")
            } else {
                format!("exit {code} · {elapsed}")
            },
            theme::DANGER,
        ),
        None => (
            if elapsed.is_empty() {
                "interrupted".to_owned()
            } else {
                format!("interrupted · {elapsed}")
            },
            theme::DANGER,
        ),
    }
}

/// One Warp-style block: command line, then its output flush under it. The
/// whole card is clickable to isolate that one block.
fn block_card<'a>(block: &'a Block, selected: bool) -> iced::widget::Column<'a, Event> {
    let (status, status_color) = block_status(block);
    let mut lines = column![
        row![
            text("›")
                .font(ui::MONO)
                .size(BLOCK_FONT)
                .color(if selected { ui::ACCENT } else { ui::MUTED })
                .width(MARKER),
            text(&block.command)
                .font(ui::MONO)
                .size(BLOCK_FONT)
                .width(Fill),
            text(status).size(11).color(status_color),
            ui::action(Icon::Copy, "Copy block output", Event::Copy(block.id)),
        ]
        .align_y(iced::Center)
    ]
    .spacing(2);
    if !block.output.is_empty() {
        // Absolute line height keeps block output on the grid's 17 px rows.
        lines = lines.push(
            container(
                text(&block.output)
                    .font(ui::MONO)
                    .size(BLOCK_FONT)
                    .line_height(text::LineHeight::Absolute(Pixels(BLOCK_LINE)))
                    .width(Fill),
            )
            .padding(iced::Padding {
                left: MARKER,
                ..Default::default()
            }),
        );
    }
    lines
}

fn terminal_stream(data: &(u64, PathBuf)) -> impl Stream<Item = (u64, Event)> + use<> {
    let (id, directory) = data.clone();
    iced::stream::channel(16, async move |mut output| {
        let (events_tx, mut events_rx) = tokio::sync::mpsc::channel(2);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(mud_terminal::run_local(directory, events_tx, ready_tx));
        let result = ready_rx
            .await
            .unwrap_or_else(|_| Err("Shell startup cancelled".into()));
        if output.send((id, Event::Ready(result))).await.is_err() {
            return;
        }
        while let Some(event) = events_rx.recv().await {
            if output.send((id, Event::Backend(event))).await.is_err() {
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_status_shows_exit_code_only_when_the_command_failed() {
        let block = |exit_code| Block {
            id: 0,
            command: "true".into(),
            output: String::new(),
            exit_code,
            duration: Some(std::time::Duration::from_millis(1234)),
        };
        assert_eq!(block_status(&block(Some(0))), ("1.23s".into(), ui::MUTED));
        assert_eq!(
            block_status(&block(Some(2))),
            ("exit 2 · 1.23s".into(), theme::DANGER)
        );
        assert_eq!(
            block_status(&block(None)),
            ("interrupted · 1.23s".into(), theme::DANGER)
        );
    }
    #[test]
    fn only_local_shells_start_processes_and_moves_preserve_identity() {
        let mut workspace = Workspace::new(PathBuf::from("/tmp"));
        workspace.split(iced::widget::pane_grid::Axis::Vertical, LaunchKind::Shell);
        let before: BTreeSet<_> = shell_ids(&workspace).map(|(id, _)| id).collect();
        workspace.move_to_tab();
        assert_eq!(before, shell_ids(&workspace).map(|(id, _)| id).collect());
        workspace.new_terminal(LaunchKind::Agent, None);
        workspace.add(PaneBody::terminal(
            SessionTarget::Remote {
                host: "prod".into(),
                directory: "/".into(),
            },
            LaunchKind::Shell,
        ));
        assert_eq!(shell_ids(&workspace).count(), 2);
    }

    fn snapshot(mode: RenderMode) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            screen: mud_terminal::Screen {
                columns: 80,
                rows: 24,
                cells: Vec::new(),
                cursor: Some((0, 0)),
                hidden_rows: 0,
                live_rows: 24,
                mode,
                application_cursor: false,
                bracketed_paste: false,
                mouse_tracking: false,
                mouse_motion: false,
                mouse_drag: false,
                mouse_sgr: false,
            },
            blocks: Vec::new(),
            running: None,
            transcript: "Alpha\nBeta\nALPHA".into(),
            warning: None,
            exited: false,
            prompt_ready: true,
        })
    }

    #[test]
    fn search_is_case_insensitive_wraps_and_never_disables_fullscreen_input() {
        let mut terminals = Terminals::default();
        let _ = terminals.update(
            1,
            Event::Backend(PtyEvent::Snapshot(snapshot(RenderMode::Blocks))),
        );
        let _ = terminals.update(1, Event::Find);
        let _ = terminals.update(1, Event::Query("alpha".into()));
        assert_eq!(terminals.live[&1].matches().len(), 2);
        let _ = terminals.update(1, Event::Match(-1));
        assert_eq!(terminals.live[&1].match_index, 1);
        let _ = terminals.update(1, Event::Match(1));
        assert_eq!(terminals.live[&1].match_index, 0);
        let _ = terminals.update(
            1,
            Event::Backend(PtyEvent::Snapshot(snapshot(RenderMode::Fullscreen))),
        );
        assert!(!terminals.live[&1].finding);
        let _ = terminals.update(1, Event::Find);
        assert!(!terminals.live[&1].finding);
    }

    #[test]
    fn closing_session_discards_runtime_but_move_does_not() {
        let mut workspace = Workspace::new(PathBuf::from("/tmp"));
        let (id, _) = shell_ids(&workspace).next().expect("shell");
        let mut terminals = Terminals::default();
        let _ = terminals.update(
            id,
            Event::Backend(PtyEvent::Snapshot(snapshot(RenderMode::Blocks))),
        );
        terminals.retain(&workspace);
        assert!(terminals.live.contains_key(&id));
        workspace.close_tab(workspace.tab().id);
        terminals.retain(&workspace);
        assert!(!terminals.live.contains_key(&id));
    }
}
