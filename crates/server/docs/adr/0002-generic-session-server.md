# ADR 0002: KelpServer is a generic session server

- Status: Accepted by Joan 2026-11-20 (America/Los_Angeles). Not implemented.
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

- Exact split of `server.rs`, `status.rs` (host vitals) and `discovery.rs` (agent and tmux discovery) between the two sides; decided during step 1 by the boundary rule.
- Whether closed-session records stay generic or become client-owned.
