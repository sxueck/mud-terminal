# Role & Objective
You are an elite Rust Systems Architect and GUI Engineer. Your task is to design and implement a high-performance, cross-platform (macOS & Windows) native AI Terminal & SSH IDE combining the best of **Warp** (native Block-based command flow & AI experience), **Xshell** (fast SSH session & SFTP management), and **iTerm2** (rock-solid local terminal experience).

You MUST build this as a **pure native Rust GUI application** (strictly NO Electron, NO Tauri/WebView).

---

# 1. Core Technical Stack & Reference Open-Source Projects
Actively draw architectural patterns from proven Rust open-source projects:
- **GUI Framework**: **GPUI** (`zed-industries/zed`) or **Iced** (`iced-rs/iced` with custom wgpu shaders). Prioritize GPU-accelerated 120fps rendering and crisp layout styling.
- **Terminal Emulation & VTE Core**: `alacritty_terminal` (`alacritty/alacritty`) or `wezterm-term` (`wez/wezterm`). Must support full VT100/xterm escape sequences, true color, mouse reporting, and rapid Alternate Screen buffer switching.
- **Local PTY Management**: `portable-pty` for cross-platform ConPTY (Windows) and POSIX PTY (macOS).
- **Async Runtime & HTTP**: `tokio` + `reqwest` (with `zstd`/`gzip` compression and SSE streaming).
- **SSH & SFTP Protocol**: `russh` and `russh-sftp` (pure Rust async SSH2 implementation).
- **Built-in Code Editor**: `ropey` (text rope buffer) + `tree-sitter` (syntax highlighting) for editing local and remote SFTP files directly in tabs/panes.

---

# 2. UI/UX & Visual Design Specification
Follow a **Clean, Modern Light-First Floating Island Design**[cite: 1, 2]:
1. **Window & Workspace Layout**:
   - **Unified Top Bar**: Integrates native window controls, a Workspace/Profile switcher (`Default`), rounded Pill-style Tabs (with active status indicator dots), a `+` new tab button, and right-aligned action icons (Split Horizontal, Split Vertical, Search, Command Palette `/`, Notifications)[cite: 1].
   - **Floating Left Sidebar**: Rounded container (`border-radius: 10px`, subtle border `#E5E7EB`, background `#F9FAFB`) detached slightly from the window edge[cite: 1]. Supports switching between:
     - **File Tree**: Local directory tree (when in Local Shell) or remote **SFTP File Tree** (when connected to SSH)[cite: 1]. Clicking a file opens it in the built-in Editor pane.
     - **SSH Host Manager**: Quick-connect list & saved session credentials.
     - **Git Status Panel**: Strictly operates on **local** repositories only (automatically disabled/hidden during remote SSH sessions).
   - **Main Content Area**: Clean canvas (`#FFFFFF`) with rounded corners, supporting flexible multi-pane splits (`Split right ⌘D`, `Split down ⇧⌘D`, `Move pane to tab..`)[cite: 1].
2. **Context Menus & Settings Modal**:
   - **Floating Context Menu**: Clean white popover with rounded corners, soft drop shadow, icon + label + right-aligned keybinding hints (`Paste ⌘V`, `Split right ⌘D`, `Split down ⇧⌘D`, `Move pane to tab..`, `Find in scrollback ⌘F`, `Open scrollback in editor`, `Clear screen and scrollback ⇧⌘K`, `Restart shell`, `Close block ⌘W`)[cite: 1].
   - **Settings UI**: Modal dialog with a top rounded search bar (`Search settings`), horizontal icon tab bar, monospace numbered section headers (e.g., `01 · General`), bold title (`Shell, tabs and selection`), and card-based setting items with segmented pill toggles (`[Shell | Agent]`, `[Current directory | Home]`, `[Click | ⌘-click]`)[cite: 2].

---

# 3. Core Functional Architecture

## 3.1 Hybrid Terminal Rendering: Warp Blocks + Full-Screen TUI Mode
- **Block Mode (Default for Shell & Agent)**:
  - Each command execution or AI interaction is encapsulated in a visual **Block** (Prompt + Input + Stdout/Stderr Output + Exit Code + Execution Time)[cite: 1, 2].
  - Users can navigate between blocks, copy output, or `Open scrollback in editor`[cite: 1].
- **Automatic Full-Screen TUI Promotion (Critical for TUI Agent Performance)**:
  - Monitor VTE escape sequences for **Alternate Screen Buffer (`?1049h` / `?47h`)** when running full-screen TUI apps (`vim`, `htop`, or external TUI Coding Agents).
  - Immediately bypass Block layout wrappers and promote the pane into **Full-Screen Exclusive GPU Terminal Mode**, passing raw keystrokes and mouse events at zero latency until `?1049l` exits back to Block Mode.

## 3.2 Native Rust Re-Implementation of `pi` Agent Core (System 2 Planner)
Do NOT embed external Node/CLI binaries, and do NOT implement bloatware like token billing, cost tracking, or dozens of third-party vendor adapters. Build a lean, pure-Rust **Pi Agent Core** inside the application:
- **Minimal Model & Protocol Support**:
  - **Model Discovery**: Support fetching model lists via `GET /v1/models` and allowing custom model ID overrides in Settings. No cost/billing calculation required.
  - **OpenAI Chat Completions API** (`POST /v1/chat/completions` with streaming SSE and tool calls).
  - **OpenAI Responses API** (`POST /v1/responses` with streaming SSE, reasoning items, and built-in tool call loops).
- **Warp-Style Native Agent Interaction**:
  - Support opening dedicated `Agent` blocks/panes (`⇧⌘A` or via `New tabs and splits open: [Shell | Agent]`)[cite: 2] or invoking AI inline within a terminal session.
  - Built-in Agent Tools: `run_shell_command`, `read_file`, `write_file`, `list_directory`, `search_scrollback`.

## 3.3 TypeSafe AI `jev` Integration (System 1 Autopilot & Ops Audit Engine)
Integrate **TypeSafe AI's `jev` (`jev-latest` / `jev-1.13.0`)** discriminative System One model to power ultra-low-latency (70–500ms), zero-hallucination operational auditing and Autopilot step gating.
- **`jev` Client Specification (`POST https://api.typesafe.ai/v1/systemone`)**:
  - Support optional `zstd` / `gzip` request payload compression for bodies >= 1 KiB.
  - Request payload structure:
    ```json
    {
      "model": "jev-latest",
      "state": {
        "host_type": "remote_ssh",
        "os_context": "Ubuntu 24.04 LTS",
        "user_goal": "Troubleshoot firewall port 443 drop",
        "proposed_command": "iptables -L -n -v",
        "recent_output": "..."
      },
      "questions": {
        "is_read_only": {
          "type": "noul",
          "instructions": "Is this command strictly read-only and non-disruptive to network or system state?",
          "criteria": {
            "true": "Only inspects state, logs, or configurations without modifying anything",
            "false": "Modifies rules, restarts services, writes files, or alters system state"
          }
        },
        "risk_score": {
          "type": "score",
          "instructions": "Rate the operational risk of executing this command on a production server.",
          "criteria": [
            "Safe diagnostic inspection (ls, cat, iptables -L, ss, journalctl)",
            "Low-risk state change (creating temp file, reloading non-critical config)",
            "Moderate-risk mutation (modifying firewall rules, restarting daemon)",
            "High-risk destructive or lockout action (iptables -F, ufw disable, rm -rf, sshd stop)"
          ]
        },
        "autopilot_decision": {
          "type": "choice",
          "instructions": "How should the terminal autopilot handle this step?",
          "choices": {
            "execute_immediately": "Safe to run automatically without bothering the user",
            "prompt_user_approval": "Requires explicit human confirmation before running",
            "block_dangerous": "Critically dangerous or likely to sever SSH connection; block and warn"
          }
        }
      }
    }
    ```
  - Parse `jev` typed responses (`noul` probability `[0, 1]`, `score` weighted average + `confidence`, and `choice` winner + `probabilities` + `confidence`).
- **User-Controllable Autopilot Modes**:
  - **Step-by-Step Approval Mode**: Every Agent step waits for user click (`[Run]`, `[Edit]`, `[Skip]`), while displaying `jev`'s real-time risk score badge and confidence percentage on the approval card.
  - **`jev` Smart Autopilot Mode**: Automatically executes steps where `jev` returns `autopilot_decision == "execute_immediately"` (with `confidence >= threshold` and `is_read_only >= 0.85`), and automatically pauses for human approval when `jev` flags moderate/high risk (e.g., when transitioning from firewall inspection `iptables -L` to firewall mutation `iptables -A`).
  - **Full Auto Mode**: Runs all steps unless `jev` flags `block_dangerous` (e.g., preventing accidental SSH self-lockout).

## 3.4 Zero-Dependency Remote SSH Ops, SFTP & Built-in Editor (MVP Scope)
- **Zero Remote Dependencies**:
  - When connected to an SSH host, require **ZERO installed packages** on the remote machine.
  - The local Pi Agent + local `jev` evaluator run on the client desktop and execute remote inspection/audit commands by dispatching POSIX commands over the active `russh` session channel, capturing stdout/stderr/exit codes back into the local loop.
- **MVP SSH Authentication**:
  - Support standard Password authentication and Private Key authentication (`id_ed25519` / `id_rsa` / `.pem` with optional passphrase).
  - Exclude ProxyJump, port forwarding, and multi-window broadcast from MVP.
- **SFTP File Tree & Native Editor**:
  - Automatically mount `russh-sftp` for the active SSH session to drive the Left Sidebar File Tree.
  - Clicking any local or SFTP file opens a native **Editor Tab/Pane** (`ropey` + `tree-sitter`) supporting editing and direct `⌘S` write-back (to local disk or over SFTP).
  - Support `Open scrollback in editor` from the terminal context menu[cite: 1].

---

# 4. Phased Implementation Roadmap (Execution Instructions)

Execute this project phase-by-phase. Ensure every phase compiles cleanly (`cargo check` / `cargo test`) with zero stubbed placeholders before moving to the next phase.

### Phase 1: Workspace Architecture, GPU UI Shell & Visual Fidelity
1. Initialize the Cargo workspace:
   - `crates/app` (Window management, layout tree, global state)
   - `crates/ui` (Theme tokens, Floating Sidebar, Unified Tab Bar, Context Menu, Settings Modal)
   - `crates/terminal` (PTY, `alacritty_terminal` VTE integration, Block vs. Fullscreen TUI renderer)
   - `crates/ssh` (`russh` & `russh-sftp` client, host credential store)
   - `crates/editor` (`ropey` buffer, `tree-sitter` highlighting, local/SFTP file I/O)
   - `crates/agent` (Lean Rust Pi Agent loop, OpenAI Chat/Responses adapters, TypeSafe `jev` System One client)
2. Build the UI shell matching the reference screenshots[cite: 1, 2]:
   - Top unified bar, rounded left sidebar (File Tree / SSH Hosts / Local Git), multi-pane binary split tree, right-click context menu[cite: 1], and the numbered Settings modal (`01 · General`, `02 · Models & API`, `03 · JEV Autopilot`)[cite: 2].

### Phase 2: Local PTY, Warp Block Engine & Full-Screen TUI Promotion
1. Wire `portable-pty` and `alacritty_terminal` into the terminal pane.
2. Implement **Block Mode** rendering for standard shell commands and **Automatic Full-Screen TUI Mode** upon detecting `?1049h` / `?47h` Alternate Screen sequences.
3. Implement scrollback search (`⌘F`), `Clear screen and scrollback ⇧⌘K`, `Restart shell`, and `Open scrollback in editor`[cite: 1].

### Phase 3: Built-in Code Editor & Local Git Sidebar
1. Implement the native Editor pane (`ropey` + `tree-sitter`) with line numbers, syntax highlighting, and `⌘S` save.
2. Connect the Left Sidebar Local File Tree and Local Git status view (ensuring Git is strictly scoped to local sessions).

### Phase 4: MVP SSH Manager & Remote SFTP Integration
1. Implement `crates/ssh` using `russh` (Password + Private Key auth) and connect remote PTY channels into the Hybrid Block/TUI Terminal pane.
2. Integrate `russh-sftp` with the Left Sidebar File Tree when an SSH tab is focused, enabling seamless remote file browsing and direct editing/saving in the built-in Editor.

### Phase 5: Lean Rust Pi Agent + TypeSafe `jev` Autopilot & Remote Ops Audit
1. Implement the OpenAI `/v1/models`, `/v1/chat/completions`, and `/v1/responses` streaming clients in `crates/agent`.
2. Implement the TypeSafe AI `jev` client (`POST /v1/systemone`) supporting `noul`, `choice`, and `score` primitives with confidence & probability parsing.
3. Build the embedded Pi Agent ReAct loop with local and remote SSH command execution tools.
4. Wire the **`jev` Autopilot Gatekeeper** into every Agent tool execution:
   - Display real-time `jev` risk badges, confidence scores, and probability bars inside the Agent Block UI.
   - Implement the 3 execution modes (`Step Approval`, `JEV Smart Autopilot`, `Full Auto`) and verify end-to-end zero-dependency remote SSH firewall/ops auditing.

---

# 5. Engineering Constraints
- **Render Thread Isolation**: Never block the GUI thread. All PTY reads, SSH/SFTP streams, OpenAI SSE streams, and `jev` API evaluations must run on `tokio` background tasks communicating via lock-free channels.
- **Strict Error Handling**: No `.unwrap()` in runtime paths. Surface SSH errors, SFTP permission denials, or API failures cleanly inside UI blocks or toast notifications.
- **Action**: Start immediately with **Phase 1** by creating the Cargo workspace structure and foundational UI layout.
