# mud-terminal

A native Rust terminal workspace using Iced/wgpu, `portable-pty` and
`alacritty_terminal`. No WebView or external agent runtime is embedded.

## Run

Rust 1.88 or newer and the native build prerequisites for Iced/wgpu are required.

```sh
cargo run -p mud-app
```

The initial local shell starts in the process working directory. Each local shell
pane owns an independent PTY; moving a pane into a tab preserves the session.
Inactive tabs keep receiving output. Closing a shell pane releases its backend;
restarting a shell terminates the old foreground process and starts a fresh PTY
in the session's original directory. Restart discards the session history.

## Phase 2

- Local PTY input/output, resize and process lifecycle run outside the GUI thread
- ANSI/xterm emulation uses Alacritty's VTE/grid, including true color, wide and
  combining characters, cursor modes and mouse reporting
- Shell integration records command blocks with command, merged stdout/stderr,
  exit code and monotonic execution duration; blocks offer output copying and
  previous/next navigation
- Blocks mode is one scrolling document: finished blocks followed by the live
  tail. The tail is cropped to the rows after the last `OSC 133;A` prompt and
  sized to the rows that actually hold text, so a finished command's output is
  never shown twice and empty screen rows cannot push the prompt out of view
- Alternate-screen requests (`?1049`, `?47`, `?1047`) switch the pane to an
  exclusive terminal grid, removing Block/search layout and pane padding; leaving
  the alternate screen restores Blocks
- Scrollback search is case-insensitive, shows matching lines and wraps through
  results; it floats in the pane's top-right corner so it never reflows the
  document. Search is disabled during exclusive fullscreen mode so it cannot
  steal TUI input
- The context menu exposes search, clear screen/scrollback/blocks, restart and
  opening the current scrollback as a read-only native editor document
- Agent panes remain drafts for Phase 5; remote sessions are never started as
  local processes. Editable files, Git and SSH/SFTP remain later phases

### Shell integration

On Unix, the application selects `$SHELL` (falling back to `/bin/sh`). Windows
selects PowerShell. Bash, zsh, fish and PowerShell have session-scoped startup
hooks; unsupported shells run as raw terminals with a visible warning and no
command-block metadata. Hooks use temporary files and do not modify user config.
Bash/zsh command hooks also send authoritative command text rather than relying
solely on editable terminal echo. Shell startup configuration can delay the first
prompt or override hooks; command metadata depends on the integration remaining
installed. Bash uses DEBUG inheritance to cover subshell commands.

The current host's real-PTY tests cover bash and zsh. Fish and PowerShell hook
scripts have structural tests only; Windows/ConPTY and IME behavior still require
native target-platform testing. The GPU terminal renders live cell colors and
attributes; completed Block output and exported scrollback are plain text, not a
replay of a full-screen application's cursor-addressed display.

### Shortcuts

| Action | macOS | Windows/Linux |
| --- | --- | --- |
| Find in scrollback | `⌘F` | `Ctrl+Shift+F` |
| Clear screen and scrollback | `⇧⌘K` | `Ctrl+Shift+K` |
| Paste | `⌘V` | `Ctrl+Shift+V` |
| New tab | `⌘T` | `Ctrl+Shift+T` |
| Split right | `⌘D` | `Ctrl+Shift+D` |
| Split down | `⇧⌘D` | `Ctrl+Shift+E` |
| Close pane/tab | `⌘W` | `Ctrl+Shift+W` |

Ordinary Ctrl combinations remain terminal input, including Ctrl+C/D/Z. The
context menu also provides Restart shell and Open scrollback in editor.

### Resource limits

The VTE grid retains up to 10,000 history lines. Plain-text scrollback is capped
at 128 KiB plus 16 KiB trimming slack. Block history retains 200 commands, with
bounded head/tail output (16 KiB plus trimming/line slack per command) and an
explicit truncation marker. Immutable completed blocks are shared across frame
snapshots. Input/event queues are bounded, snapshot delivery coalesces under
backpressure, and queue failures remain visible instead of blocking the GUI.
Pastes larger than 1 MiB are rejected before anything is enqueued. Grid geometry
is limited to 1,000 columns × 500 rows.

These bounds and an 8 ms snapshot throttle are not a measured 120 fps guarantee.
GPU rendering and throughput must be profiled on the target hardware.

## Verify

```sh
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The real-PTY integration tests include exit status, final output/EOF ordering,
alternate-screen round trips, resize, clear, restart and shutdown, plus a blocked
512 KiB PTY write during resize/restart. Bash/zsh tests skip explicitly when their
shell is unavailable.

Run these on a graphical desktop; they create real native windows, save GPU
screenshots under `target/`, and exit automatically:

```sh
cargo run -p mud-app --example visual_check -- target/phase2-visual
cargo run -p mud-app --example terminal_check -- target/phase2-terminal
```

`terminal_check` requires shell integration and `vim` on PATH. It uses real PTY
output to verify Blocks, search, clear, scrollback export, restart, fullscreen vim,
raw TUI input and return to Blocks. It bypasses incidental foreground typing to
keep its scripted input deterministic; key/mouse encoders have separate unit
tests. This is not a substitute for manual keyboard/IME testing on each OS.
