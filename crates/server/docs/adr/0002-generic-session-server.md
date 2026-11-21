# ADR 0002: KelpServer is a generic session server

- Status: Accepted by Joan 2026-11-20 (America/Los_Angeles). Step 1 implemented; steps 2–4 not started.
- Related: [ADR 0001](0001-native-client-protocol-and-device-capabilities.md), KelpMux [ADR 0003 (web client)](https://github.com/joanfabregat/kelpmux/blob/main/docs/adr/0003-web-client-in-kelpmux.md), KelpHub ADR 0007 (multi-tenant, Kubernetes-only; private repository).

## Context

KelpServer was extracted from KelpMux on 2026-11-20 so that KelpMux (TUI and web client) and KelpHub depend on one server contract. The extracted code still works like tmux: besides holding the sessions, it owns the TUI's user interface. The TUI is a thin client of frames the server composes (local protocol 58). Tabs and their order, parking, focus, dialogs, grid geometry, scratch and the screen rendering all live in the server, about 6,500 of 18,200 lines (`render.rs`, `workspace.rs`, `ui.rs`) plus a large part of `server.rs`.

The web client already uses the per-pane stream protocol and keeps its own layout. KelpHub will run KelpServer inside each workspace pod and has its own UI.

## Decision

**Boundary rule:** KelpServer contains only what KelpMux and KelpHub share. Everything specific to KelpMux lives in KelpMux; everything specific to KelpHub lives in KelpHub.

KelpServer keeps:

- Sessions: PTYs and processes, pane create/close/list with working directory, arguments and a sanitized environment, terminal emulation, scrollback/history, search, the per-pane stream protocol with per-client leases and sizes, resize.
- Records of closed sessions needed to resume or rerun them.
- Pane observations: running program, agent detection, activity and notifications, shared by every client (143a).
- Server lifecycle: start, stop, upgrade, status, journal.

KelpServer drops, moving them to KelpMux:

- The composed protocol (local protocol 58), `render.rs` and the server-side TUI state: tabs, tab order, names, colours, parking, focus and focus history, dialogs, grid geometry, scratch, templates and TUI settings. The TUI becomes a stream-protocol client that renders locally, like the web client.
- The web relay (`web_relay.rs`), which serves KelpMux's web UI (145a).

Client UI state is owned by each client, keyed by pane identity (server epoch plus pane ID): names, colours, order and parking are not shared between clients (144b). The TUI persists its own state on disk; the web keeps it per browser. Consequence: a name or colour set in the TUI does not appear in the web, and two TUI instances share only what they persist in the same file.

The composed protocol is removed once the new TUI works (146a). Running pre-prod installations keep their current server and TUI build until they are switched deliberately; a new TUI requires a new server.

## Migration

1. Define the session API needed by a full TUI on the stream protocol (pane lifecycle, observations, closed-session records), additively, while protocol 58 still works. Version the stream protocol if the contract changes.
2. In KelpMux, rebuild the TUI as a stream client with local rendering and local UI state, moving `render.rs`, `workspace.rs` and `ui.rs` logic there. Move the web relay to KelpMux.
3. Remove protocol 58 and the UI code from KelpServer; bump the major version.
4. KelpHub consumes the session API from its pods.

The tab behavior merged in [kelpserver#1](https://github.com/joanfabregat/kelpserver/pull/1) (unparked tabs go last, closing or parking refocuses the previous tab) moves with the TUI code in step 2.

## Open points

Both points were decided during step 1; see [Implementation notes](#implementation-notes).

- Exact split of `server.rs`, `status.rs` (host vitals) and `discovery.rs` (agent and tmux discovery) between the two sides; decided during step 1 by the boundary rule.
- Whether closed-session records stay generic or become client-owned.

## Implementation notes

### Step 1: session API (2026-11-20)

The pane stream now carries the session API: pane lifecycle events and per-pane session data, `spawn` and `close`, closed-session records (`closed_sessions`, `restore`), history and multi-pane search, lease-bound scrolling, `acknowledge_all`, opt-in host status, `shutdown` and `server_stopping`, and the server version in Welcome. Everything is additive and capability-gated; the stream stays at version 1, events of new types reach only connections that opted in or sent the request, and protocol 58 is unchanged. The contract is in [native-client-protocol.md](../native-client-protocol.md#session-api); the capability inventory, the decisions and the gaps left for step 2 are in [session-api.md](../session-api.md). No UI code moved and the server stores no new client UI state.

Parking needs no server concept: a client unsubscribes from a hidden pane and keeps its lease (fixed size) or releases it (base geometry). The scratch terminal is an ordinary pane created with `spawn`. Tab order can default to pane ID order, which is creation order within an epoch.

The open points were decided by the boundary rule:

- **`server.rs`, `status.rs` and `discovery.rs`.** `discovery.rs` (Claude/Codex agent discovery and polling) produces pane observations shared by every client and stays. `status.rs` splits: host sampling (`Monitor`, `Snapshot`, `Process`, `Disk`) stays and is exposed by `host-status`; its presentation (`View`, `Sort`, column and layout helpers, formatting) moves to KelpMux with the Status dialog in step 2. `server.rs` keeps PTYs, the stream and its leases, spawn/close/restore, discovery, history and activity; composed-frame production, `mouse`, the `Action` handling, the TUI controller and theme fallback, title animation, unused-shell hints and the UI parts of `persist_geometry` leave in step 3, after the TUI no longer needs them.
- **Closed-session records stay in the server and stay generic.** They are derived from server observations (foreground programs, agent sessions), are useful to KelpMux and KelpHub alike, and are addressed by an opaque `key`; only their presentation (restore-choice labels, copying the resume command) is client UI.
