//! Live PTY integration tests using real bash/zsh when available.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mud_terminal::{PtyEvent, PtyHandle, RenderMode, Request, Snapshot, run_local_with_shell};
use tokio::sync::{mpsc, oneshot};

const TIMEOUT: Duration = Duration::from_secs(20);

fn which(program: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("which")
        .arg(program)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

struct Session {
    handle: PtyHandle,
    events: mpsc::Receiver<PtyEvent>,
    _dir: tempfile::TempDir,
}

async fn spawn_shell_session(shell: PathBuf) -> Session {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let (event_tx, event_rx) = mpsc::channel(64);
    let (ready_tx, ready_rx) = oneshot::channel();
    tokio::spawn(run_local_with_shell(
        dir.path().to_path_buf(),
        Some(shell),
        event_tx,
        ready_tx,
    ));
    match ready_rx.await {
        Ok(Ok(handle)) => Session {
            handle,
            events: event_rx,
            _dir: dir,
        },
        Ok(Err(err)) => panic!("backend spawn failed: {err}"),
        Err(_) => panic!("backend dropped ready channel"),
    }
}

impl Session {
    async fn snapshot(&mut self) -> Option<Snapshot> {
        match tokio::time::timeout(TIMEOUT, self.events.recv()).await {
            Ok(Some(PtyEvent::Snapshot(snapshot))) => Some((*snapshot).clone()),
            Ok(Some(PtyEvent::Error(err))) => panic!("backend error: {err}"),
            Ok(None) | Err(_) => None,
        }
    }

    async fn wait_for(&mut self, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let deadline = Instant::now() + TIMEOUT;
        let mut last = None;
        while Instant::now() < deadline {
            match tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                self.events.recv(),
            )
            .await
            {
                Ok(Some(PtyEvent::Snapshot(snapshot))) => {
                    if predicate(&snapshot) {
                        return (*snapshot).clone();
                    }
                    last = Some(snapshot);
                }
                Ok(Some(PtyEvent::Error(err))) => panic!("backend error: {err}"),
                Ok(None) => panic!("backend closed events before predicate matched"),
                Err(_) => break,
            }
        }
        panic!(
            "timed out waiting for snapshot: {:?}",
            last.map(|s| (s.transcript.clone(), s.blocks.clone(), s.warning.clone()))
        );
    }

    async fn run(&self, command: &str) {
        let mut bytes = command.as_bytes().to_vec();
        bytes.push(b'\n');
        self.handle
            .send(Request::Input(bytes))
            .expect("send command");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bash_block_capture_with_exit_code() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    let _ = session.snapshot().await;
    session.run("echo mud-bash-ok").await;
    let snapshot = session
        .wait_for(|s| {
            s.blocks
                .iter()
                .any(|b| b.command == "echo mud-bash-ok" && b.exit_code.is_some())
        })
        .await;
    let block = snapshot
        .blocks
        .iter()
        .find(|b| b.command == "echo mud-bash-ok")
        .expect("bash block must be recorded with an exit code");
    assert_eq!(block.exit_code, Some(0));
    assert!(
        block.output.contains("mud-bash-ok"),
        "output: {}",
        block.output
    );
    assert!(snapshot.transcript.contains("mud-bash-ok"));
}

/// The Blocks view crops the grid to the rows after the last prompt, so a
/// finished command's output must never appear twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_tail_does_not_repeat_finished_block_output() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    let _ = session.snapshot().await;
    session.run("echo mud-crop-probe").await;
    let snapshot = session
        .wait_for(|s| {
            s.prompt_ready
                && s.blocks
                    .iter()
                    .any(|b| b.command == "echo mud-crop-probe" && b.exit_code.is_some())
        })
        .await;
    let block = snapshot
        .blocks
        .iter()
        .find(|b| b.command == "echo mud-crop-probe")
        .expect("block");
    assert!(block.output.contains("mud-crop-probe"));
    assert!(
        snapshot.screen.hidden_rows > 0,
        "the prompt row must be cropped out of the live tail"
    );
    let tail: String = snapshot
        .screen
        .cells
        .iter()
        .filter(|cell| cell.row >= snapshot.screen.hidden_rows)
        .map(|cell| cell.text.as_str())
        .collect();
    assert!(
        !tail.contains("mud-crop-probe"),
        "live tail repeats block output: {tail}"
    );
    assert!(
        !tail.trim().is_empty(),
        "live tail must still carry the new prompt"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bash_nonzero_exit_code_captured() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    let _ = session.snapshot().await;
    session.run("(exit 7)").await;
    let snapshot = session
        .wait_for(|s| {
            s.blocks
                .iter()
                .any(|b| b.exit_code == Some(7) && b.duration.is_some())
        })
        .await;
    assert!(snapshot.blocks.iter().any(|b| b.exit_code == Some(7)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zsh_block_capture_with_exit_code() {
    let Some(zsh) = which("zsh") else {
        eprintln!("skipping: zsh not found");
        return;
    };
    let mut session = spawn_shell_session(zsh).await;
    let _ = session.snapshot().await;
    session.run("echo mud-zsh-ok").await;
    let snapshot = session
        .wait_for(|s| {
            s.blocks
                .iter()
                .any(|b| b.command == "echo mud-zsh-ok" && b.exit_code.is_some())
        })
        .await;
    let block = snapshot
        .blocks
        .iter()
        .find(|b| b.command == "echo mud-zsh-ok")
        .expect("zsh block must be recorded with an exit code");
    assert_eq!(block.exit_code, Some(0));
    assert!(
        block.output.contains("mud-zsh-ok"),
        "output: {}",
        block.output
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alt_screen_47_and_1049_round_trip() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    let _ = session.snapshot().await;
    session.run("printf '\\033[?47h'").await;
    session
        .wait_for(|s| s.screen.mode == RenderMode::Fullscreen)
        .await;
    session.run("printf '\\033[?47l'").await;
    session
        .wait_for(|s| s.screen.mode == RenderMode::Blocks)
        .await;
    session.run("printf '\\033[?1049h'").await;
    session
        .wait_for(|s| s.screen.mode == RenderMode::Fullscreen)
        .await;
    session.run("printf '\\033[?1049l'").await;
    session
        .wait_for(|s| s.screen.mode == RenderMode::Blocks)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clear_wipes_blocks_scrollback_and_transcript() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    let _ = session.snapshot().await;
    session.run("echo mud-clear-me").await;
    session
        .wait_for(|s| s.blocks.iter().any(|b| b.exit_code.is_some()))
        .await;
    session
        .handle
        .send(Request::Input(b"seq 1 200\n".to_vec()))
        .expect("send seq");
    session
        .wait_for(|s| s.blocks.len() >= 2 && s.blocks[1].exit_code.is_some())
        .await;
    session.handle.send(Request::Clear).expect("send clear");
    let snapshot = session
        .wait_for(|s| s.blocks.is_empty() && s.transcript.is_empty())
        .await;
    assert!(
        snapshot
            .screen
            .cells
            .iter()
            .all(|cell| cell.text.trim().is_empty())
    );
    assert!(!snapshot.exited);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_respawns_shell_and_works_after() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    let _ = session.snapshot().await;
    session.run("echo mud-before-restart").await;
    session.wait_for(|s| !s.blocks.is_empty()).await;
    session.handle.send(Request::Restart).expect("send restart");
    session
        .wait_for(|s| s.blocks.is_empty() && !s.exited && s.transcript.is_empty())
        .await;
    session.run("echo mud-after-restart").await;
    let snapshot = session
        .wait_for(|s| {
            s.blocks
                .iter()
                .any(|b| b.command == "echo mud-after-restart" && b.exit_code.is_some())
        })
        .await;
    let block = snapshot
        .blocks
        .iter()
        .find(|b| b.command == "echo mud-after-restart")
        .expect("block created after restart must be recorded");
    assert_eq!(block.exit_code, Some(0));
    assert!(block.output.contains("mud-after-restart"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_all_handles_shuts_down_backend() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let session = spawn_shell_session(bash).await;
    let mut events = session.events;
    let handle = session.handle;
    let dropped = session._dir; // keep temp dir alive; drop the handle below
    handle
        .send(Request::Input(b"sleep 30\n".to_vec()))
        .expect("send");
    drop(handle);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), events.recv()).await
        {
            Ok(None) => break,
            Ok(Some(_)) => continue,
            Err(_) => panic!("backend did not shut down after handles dropped"),
        }
    }
    drop(dropped);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resize_updates_screen_dimensions() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    let _ = session.snapshot().await;
    session
        .handle
        .send(Request::Resize {
            columns: 90,
            rows: 40,
        })
        .expect("send resize");
    let snapshot = session
        .wait_for(|s| s.screen.columns == 90 && s.screen.rows == 40)
        .await;
    assert_eq!(snapshot.screen.cells.len(), 90 * 40);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shell_exit_reports_exited_snapshot() {
    let Some(bash) = which("bash") else {
        eprintln!("skipping: bash not found");
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    let _ = session.snapshot().await;
    session.run("exit 0").await;
    session.wait_for(|s| s.exited).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocked_writer_does_not_pin_resize_or_restart() {
    let Some(bash) = which("bash") else {
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    session.wait_for(|s| s.prompt_ready).await;
    session.run("stty -echo -icanon; sleep 60").await;
    session
        .wait_for(|s| {
            s.running
                .as_ref()
                .is_some_and(|b| b.command.contains("sleep 60"))
        })
        .await;
    session
        .handle
        .send(Request::Input(vec![b'x'; 512 * 1024]))
        .expect("large input");
    session
        .handle
        .send(Request::Resize {
            columns: 93,
            rows: 37,
        })
        .expect("resize while writer blocked");
    session
        .wait_for(|s| s.screen.columns == 93 && s.screen.rows == 37)
        .await;
    session
        .handle
        .send(Request::Restart)
        .expect("restart while writer blocked");
    session
        .wait_for(|s| s.prompt_ready && s.blocks.is_empty() && s.running.is_none())
        .await;
    session.run("echo mud-recovered").await;
    session
        .wait_for(|s| {
            s.blocks
                .iter()
                .any(|b| b.exit_code == Some(0) && b.output.contains("mud-recovered"))
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn waiter_exit_code_and_final_output_survive_reader_eof() {
    let Some(bash) = which("bash") else {
        return;
    };
    let mut session = spawn_shell_session(bash).await;
    session.wait_for(|s| s.prompt_ready).await;
    session.run("printf 'final-pty-output'; exit 7").await;
    let snapshot = session.wait_for(|s| s.exited).await;
    let block = snapshot.blocks.last().expect("final block");
    assert_eq!(block.exit_code, Some(7));
    assert!(block.output.contains("final-pty-output"), "{block:?}");
}
