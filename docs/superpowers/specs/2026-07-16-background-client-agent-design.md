# Background Client Agent Design

## Goal

Allow a USB/IP client session to continue in the background and let later
`lusbip client` invocations reconnect to its TUI. The background session keeps
only the USB/IP ports it attached. Leaving the TUI keeps those ports; stopping
the background session detaches them and terminates the agent.

## Scope

This design adds background mode only to `lusbip client`. Existing foreground
client, `attach`, `detach`, `status`, and server behaviours remain unchanged.
The feature is scoped by the exact `(remote host, tcp port)` pair.

## Command Behaviour

`lusbip client --remote HOST --tcp-port PORT --background` starts one detached
client agent for `HOST:PORT` and returns after reporting its PID. If that agent
is already live, the command reports that fact and does not start a second one.

`lusbip client --remote HOST --tcp-port PORT` behaves as follows:

- With no background agent, it opens the existing foreground client TUI.
- With a background agent, it opens a viewer/controller TUI connected to the
  agent rather than failing on the session lock.

The attached TUI preserves these controls:

- Toggle a device: send the attach or detach request to the background agent.
- `Esc`: close only the TUI; the agent and its managed USB/IP ports remain.
- `Ctrl+C`: request a clean agent shutdown. The agent detaches every port it
  managed for this `HOST:PORT`, exits, and removes its runtime state.

The foreground TUI keeps its current `Ctrl+C` behaviour: detach ports attached
for that remote session before exiting.

## Architecture

### Runtime State

For each normalized `HOST:PORT`, create a private runtime directory below
`/tmp` containing:

- a PID file, used to find and validate the agent;
- a Unix-domain control socket;
- a status snapshot file, written atomically for diagnostics and TUI refreshes.

The state path must be derived from a filesystem-safe, collision-resistant
identifier for the remote and port. Stale state is removed only after verifying
that the PID no longer represents the expected client-agent process. A live
agent always wins; a second agent is never started for the same endpoint.

### Background Agent

The agent is the sole owner of mutations for its session. It:

1. accepts local control connections over the Unix socket;
2. polls remote exports and local `usbip port` state;
3. tracks the local port identifiers created by its successful attach actions;
4. handles toggle and shutdown commands serially; and
5. periodically writes a complete status snapshot.

The agent records a port only after attach succeeds. Detach removes it from the
managed set only after the detach operation succeeds or the port is verified
absent. On shutdown it detaches only this managed set, never every imported
USB/IP device and never a port belonging to another endpoint or session.

### Foreground Agent TUI

When a background agent exists, the foreground process is only a terminal UI.
It queries the agent for snapshots and sends commands through the socket. It
does not acquire the mutating client-session lock and does not invoke `usbip
attach` or `usbip detach` itself. This removes races between multiple terminal
views while retaining a single owner for the session.

The existing foreground implementation remains direct: it owns the current
`ClientSessionLock` and executes USB/IP commands itself.

## Control Protocol

Use a small line-delimited local protocol with explicit request and response
types:

- `status` returns the current remote-device states and managed-port list;
- `toggle <bus-id>` asks the agent to attach or detach that remote device;
- `shutdown` asks the agent to detach its managed ports and exit.

Responses include either an updated snapshot or a user-facing error. Requests
are handled one at a time, so attach/detach actions cannot overlap. Invalid or
malformed local input gets a structured error without terminating the agent.

## Failure Handling

- If the agent dies, the next invocation removes validated stale runtime state
  and falls back to normal foreground mode. Existing kernel USB/IP imports are
  not detached automatically after an unclean crash because ownership cannot be
  safely resumed without a live agent.
- If an agent shutdown cannot detach one or more managed ports, it reports the
  failures, keeps runtime state for retry, and exits only after all managed
  ports are detached. The TUI shows the failed ports and can retry `Ctrl+C`.
- If the control socket is unavailable while the PID is live, the command
  reports a recovering/unavailable agent rather than starting a competing
  session.
- Network and USB/IP command errors are returned to the TUI without killing the
  agent; the next refresh retries normal discovery.

## Test Strategy

Unit tests will cover runtime-path derivation, live versus stale PID state,
managed-port bookkeeping, parsed control commands, and shutdown detach scope.
Integration tests will use the command-runner abstraction to prove that an
agent accepts a TUI command, does not detach unrelated ports, and that a
foreground invocation selects the agent path when runtime state is live.

Manual LAN verification will use Nano Pi `pi@10.10.61.72` as server and Ubuntu
`hieunm@10.10.60.208` as client. It will verify background attach persistence,
TUI re-entry, `Esc` persistence, and `Ctrl+C` detachment of only the agent's
managed ports.
