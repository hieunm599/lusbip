# Background Client Agent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `lusbip client --background` manage a remote USB/IP session in the background, with later client TUI invocations reconnecting to it and `Ctrl+C` detaching only its managed ports before stopping it.

**Architecture:** Add a focused `client_agent` module that owns endpoint runtime files, a Unix control socket, periodic snapshots, and the managed-port set. Keep the existing direct client TUI unchanged for foreground sessions; route foreground commands to an agent-controller TUI only when a live agent exists for the same `remote:tcp-port`.

**Tech Stack:** Rust, Tokio runtime and Unix-domain sockets, `std::process`, existing `CommandRunner`, crossterm TUI, clap, Rust unit/integration tests.

---

## File structure

- Create `src/client_agent.rs`: endpoint identity, runtime-state lifecycle, request/response protocol, background-agent loop, managed-port bookkeeping, and agent-controller TUI adapters.
- Modify `src/client.rs`: expose the minimal state-loading/toggle primitives used by the agent; retain direct-session locking only for non-agent foreground mode.
- Modify `src/app_tui.rs`: select direct TUI, background-agent launch, or agent-controller TUI.
- Modify `src/cli.rs`: add `--background` to `TuiArgs`.
- Modify `src/main.rs`: pass `background` for both `client` and `tui` command aliases.
- Modify `src/lib.rs`: export `client_agent`.
- Modify `tests/cli_parse.rs`: parse coverage for client background mode.
- Create `tests/client_agent.rs`: black-box tests for endpoint paths, protocol, stale/live state, serial command ownership, and scoped shutdown.

### Task 1: CLI routing and endpoint runtime identity

**Files:**
- Create: `src/client_agent.rs`
- Modify: `src/cli.rs:74-82`
- Modify: `src/main.rs:21-30`
- Modify: `src/app_tui.rs:1-8`
- Modify: `src/lib.rs:1-8`
- Test: `tests/cli_parse.rs`
- Test: `tests/client_agent.rs`

- [ ] **Step 1: Write failing CLI and endpoint-path tests**

```rust
#[test]
fn parses_client_background_mode() {
    let cli = Cli::parse_from([
        "lusbip", "client", "--remote", "10.10.61.72", "--tcp-port", "3240", "--background",
    ]);
    assert!(matches!(cli.command, Commands::Client(args)
        if args.background && args.remote.as_deref() == Some("10.10.61.72") && args.tcp_port == 3240));
}

#[test]
fn endpoint_runtime_paths_are_stable_and_remote_scoped() {
    let first = ClientEndpoint::new("10.10.61.72", 3240);
    let second = ClientEndpoint::new("10.10.61.72", 3241);
    assert_ne!(first.runtime_dir(), second.runtime_dir());
    assert!(first.runtime_dir().starts_with(std::env::temp_dir()));
}
```

- [ ] **Step 2: Run the targeted tests and verify they fail because background state does not exist**

Run: `cargo test --test cli_parse parses_client_background_mode --test client_agent endpoint_runtime_paths_are_stable_and_remote_scoped`

Expected: compilation fails because `TuiArgs.background` and `ClientEndpoint` are absent.

- [ ] **Step 3: Add the smallest CLI and endpoint definitions**

```rust
// src/cli.rs
#[derive(Debug, Args)]
pub struct TuiArgs {
    #[arg(short, long)]
    pub remote: Option<String>,
    #[arg(long, default_value_t = 3240)]
    pub tcp_port: u16,
    #[arg(long)]
    pub background: bool,
}

// src/client_agent.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientEndpoint { remote: String, tcp_port: u16 }

impl ClientEndpoint {
    pub fn new(remote: &str, tcp_port: u16) -> Self { Self { remote: remote.into(), tcp_port } }
    pub fn runtime_dir(&self) -> PathBuf {
        let safe = self.remote.chars().map(|ch| if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') { ch } else { '_' }).collect::<String>();
        std::env::temp_dir().join(format!("lusbip-client-agent-{safe}-{}", self.tcp_port))
    }
}
```

Export the module from `lib.rs`; change `app_tui::run` to accept `background: bool`; pass `args.background` from both `Client` and `Tui` paths in `main.rs`.

- [ ] **Step 4: Run the targeted tests and verify they pass**

Run: `cargo test --test cli_parse parses_client_background_mode --test client_agent endpoint_runtime_paths_are_stable_and_remote_scoped`

Expected: both tests pass.

- [ ] **Step 5: Commit the isolated CLI/runtime identity change**

```bash
git add src/cli.rs src/main.rs src/app_tui.rs src/lib.rs src/client_agent.rs tests/cli_parse.rs tests/client_agent.rs
git commit -m "feat: add background client CLI routing"
```

### Task 2: Agent lifecycle, control protocol, and managed-port state

**Files:**
- Modify: `src/client_agent.rs`
- Modify: `src/client.rs:533-760`
- Test: `tests/client_agent.rs`

- [ ] **Step 1: Write failing lifecycle and protocol tests**

```rust
#[test]
fn stale_agent_state_is_removed_before_starting_a_new_agent() {
    let state = TestRuntimeState::with_pid(999_999);
    assert_eq!(state.prepare().unwrap(), AgentPresence::Absent);
    assert!(!state.runtime_dir().exists());
}

#[test]
fn shutdown_detaches_only_ports_managed_by_the_agent() {
    let runner = RecordingRunner::with_ports(["00", "01", "02"]);
    let mut managed = ManagedPorts::from(["01".to_string(), "02".to_string()]);
    shutdown_managed_ports(&runner, &mut managed).unwrap();
    assert_eq!(runner.detached_ports(), vec!["01", "02"]);
    assert!(managed.is_empty());
}

#[test]
fn malformed_control_request_returns_error_without_stopping_agent() {
    assert!(ControlRequest::parse("toggle\n").is_err());
    assert_eq!(ControlRequest::parse("status\n").unwrap(), ControlRequest::Status);
}
```

- [ ] **Step 2: Run the targeted test binary and verify the expected RED failure**

Run: `cargo test --test client_agent`

Expected: fails because `AgentPresence`, `ManagedPorts`, `shutdown_managed_ports`, and `ControlRequest` are absent.

- [ ] **Step 3: Implement lifecycle and protocol with a single agent owner**

Define the protocol and state types in `client_agent.rs`:

```rust
enum ControlRequest { Status, Toggle { bus_id: String }, Shutdown }
enum ControlResponse { Snapshot(ClientSnapshot), Message(String), Error(String) }
struct ManagedPorts(BTreeSet<String>);
struct ClientSnapshot { states: Vec<RemoteUsbDeviceState>, managed_ports: Vec<String> }
```

Use `tokio::net::UnixListener` and this exact newline-delimited, serde-free protocol:

```text
STATUS\n
TOGGLE\t<percent-encoded-bus-id>\n
SHUTDOWN\n
OK\n
STATE\t<encoded-bus-id>\t<encoded-description>\t<encoded-attached-port-or-empty>\t<encoded-occupied-by-or-empty>\n
END\n
ERR\t<percent-encoded-message>\n
```

Percent encoding must encode `%`, tab, carriage return, and newline as `%25`, `%09`, `%0D`, and `%0A`; decoding rejects every other malformed percent sequence. `ControlRequest::parse` and `ControlResponse::render`/`parse` are inverse operations. `run_background_agent(endpoint)` must:

1. create the runtime directory atomically and write the current PID;
2. bind its socket only inside that directory;
3. service one request at a time;
4. call existing `load_remote_device_states_cached`, `toggle_remote_device`, and `query_attached_ports` through narrowly exposed `pub(crate)` wrappers;
5. add a port to `ManagedPorts` only after a successful attach; and
6. remove all runtime files only after complete successful shutdown.

On a failed detach during shutdown, return `ControlResponse::Error`, preserve the remaining managed set and runtime directory, and continue serving retry requests. Do not call broad `usbip port` cleanup.

- [ ] **Step 4: Run lifecycle/protocol tests and the existing client parser tests**

Run: `cargo test --test client_agent && cargo test --test client_parsing`

Expected: all selected tests pass, including existing device-state parsing tests.

- [ ] **Step 5: Commit agent lifecycle and controlled cleanup**

```bash
git add src/client_agent.rs src/client.rs tests/client_agent.rs
git commit -m "feat: add background client agent lifecycle"
```

### Task 3: TUI re-entry and shutdown semantics

**Files:**
- Modify: `src/client_agent.rs`
- Modify: `src/app_tui.rs`
- Modify: `src/client.rs:104-143`
- Test: `tests/client_agent.rs`

- [ ] **Step 1: Write failing route-selection and command-ownership tests**

```rust
#[test]
fn live_agent_selects_controller_tui_instead_of_direct_lock() {
    let endpoint = live_test_agent_endpoint();
    assert_eq!(select_client_mode(&endpoint, false).unwrap(), ClientMode::AgentController);
}

#[test]
fn background_flag_starts_agent_when_none_is_live() {
    let endpoint = fresh_test_agent_endpoint();
    assert_eq!(select_client_mode(&endpoint, true).unwrap(), ClientMode::StartAgent);
}

#[test]
fn agent_toggle_calls_agent_and_not_local_usbip_process() {
    let response = run_controller_request(&test_socket(), ControlRequest::Toggle { bus_id: "5-1".into() }).unwrap();
    assert!(matches!(response, ControlResponse::Message(_)));
}
```

- [ ] **Step 2: Run targeted tests and verify they fail for missing route/controller functions**

Run: `cargo test --test client_agent live_agent_selects_controller_tui_instead_of_direct_lock --test client_agent background_flag_starts_agent_when_none_is_live --test client_agent agent_toggle_calls_agent_and_not_local_usbip_process`

Expected: compilation fails because `ClientMode`, `select_client_mode`, and `run_controller_request` are absent.

- [ ] **Step 3: Implement agent-aware TUI routing**

Implement exactly three client modes:

```rust
enum ClientMode { Direct, StartAgent, AgentController }
```

`app_tui::run(remote, tcp_port, background)` chooses `StartAgent` only for `--background` without a live agent, and chooses `AgentController` for an existing live agent regardless of the flag. `StartAgent` spawns the current executable with `client --remote ... --tcp-port ... --background --agent-child`; use a hidden clap argument (`#[arg(long, hide = true)]`) so a child runs `run_background_agent` instead of recursively spawning itself.

Build the controller with existing `run_action_list`:

- load closure sends `status` and renders its snapshot;
- activate closure sends `toggle <bus-id>`;
- exit closure sends `shutdown` only after `Ctrl+C`, while `Esc` returns without a control request.

If `run_action_list` cannot distinguish Esc and Ctrl+C in its `Exit` callback, change its callback contract to pass an explicit `ExitReason::{Escape, Interrupt}` and update its existing callers/tests so server semantics remain unchanged.

The direct TUI continues to acquire `ClientSessionLock`; the agent-controller never acquires it and never directly executes attach/detach commands.

- [ ] **Step 4: Run affected tests and verify the direct-mode regressions are absent**

Run: `cargo test --test client_agent && cargo test --test cli_parse && cargo test --test client_parsing`

Expected: all selected test suites pass.

- [ ] **Step 5: Commit the re-entry UI and Ctrl+C cleanup behaviour**

```bash
git add src/client_agent.rs src/client.rs src/app_tui.rs src/cli.rs src/main.rs src/tui.rs tests/client_agent.rs tests/cli_parse.rs
git commit -m "feat: reattach client UI to background agent"
```

### Task 4: Full verification and two-host acceptance

**Files:**
- Modify only if verification exposes a defect: the smallest affected source/test file from Tasks 1-3.

- [ ] **Step 1: Format and run the full required Rust quality gate**

Run:

```bash
cargo fmt --check
cargo clippy
cargo test
```

Expected: all commands exit 0.

- [ ] **Step 2: Build the release binary for the Linux client**

Run: `cargo build --release`

Expected: exit 0 and `target/release/lusbip` exists.

- [ ] **Step 3: Verify background persistence on the Ubuntu client**

Run on `hieunm@10.10.60.208` with Nano Pi available at `10.10.61.72`:

```bash
lusbip client --remote 10.10.61.72 --tcp-port 3240 --background
lusbip client --remote 10.10.61.72 --tcp-port 3240
```

In the first TUI, attach one exported CP2102, CH340, or ACM device; press `Esc`. In the second TUI, confirm the device remains marked attached. Confirm `usbip port` still shows the import.

- [ ] **Step 4: Verify scoped Ctrl+C shutdown on the Ubuntu client**

While the second TUI is open, press `Ctrl+C`, then run:

```bash
usbip port
test ! -e /tmp/lusbip-client-agent-10.10.61.72-3240/pid
```

Expected: the agent-managed port is detached, the agent PID state is absent, and any separately imported USB/IP port remains listed.

- [ ] **Step 5: Commit any verification-only fix, then record the evidence in the handoff**

If Steps 1-4 required a fix, run:

```bash
git status --short
git add src/client_agent.rs src/client.rs src/app_tui.rs src/cli.rs src/main.rs src/tui.rs tests/client_agent.rs tests/cli_parse.rs
git commit -m "fix: stabilize background client agent"
```

Report the exact command outcomes and the attached device tested. Do not claim completion without the fresh outputs from Steps 1-4.
