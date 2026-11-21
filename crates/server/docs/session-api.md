# Session API inventory

This is step 1 of [ADR 0002](adr/0002-generic-session-server.md): the inventory of what today's KelpMux TUI obtains from the composed protocol (local protocol 58), sorted by the boundary rule, and what the pane stream now carries so a TUI can run as a stream client with local rendering and local UI state. The wire contract is in [native-client-protocol.md](native-client-protocol.md#session-api); this page records the classification and the gaps.

"Session" means the server keeps it. "Client UI" means each client owns it, keyed by pane identity (server epoch plus pane ID), and the server stores none of it. "Before" is the stream protocol at `0837dd2`; "Step 1" is what this change adds. All additions are additive and capability-gated; the stream version stays 1.

| Capability | Session or client UI | On the stream before step 1 | Step 1 addition |
| --- | --- | --- | --- |
| Server epoch and pane identity | Session | Yes: Welcome `epoch`; pane IDs scoped to it | Documented that pane IDs increase monotonically and are never reused within an epoch, so they also give creation order. |
| Server version | Session (lifecycle) | No | Welcome `server_version` (`server-lifecycle`). |
| Pane list with process, cwd, agent kind, session ID, status, evidence, activity times and shell command | Session (observations) | Yes: `Update.panes` on every update, including a metadata-only subscription (`subscribe []`) | None. `PaneInfo.parked`, `number` and `palette` and the alias part of `title` are composed-UI fields; stream clients ignore them (removed in step 3). |
| Pane session facts: creation time, launch directory, shell, spawn arguments, startup command, raw terminal title | Session | No | `Update.sessions` (`PaneSession`) after `session_events` opt-in (`pane-lifecycle`). |
| Pane created | Session | No (a new ID appeared in `Update.panes`) | `pane_created {session}` event. |
| Pane exited (exit code or signal) | Session | No (`dead`/`Exited` status, briefly) | `pane_exited {pane, at, code, signal}` event. |
| Pane closed (by exit or by a client), with its closed-session record | Session | No (the ID disappeared) | `pane_closed {pane, at, reason, record}` event. Events invalidate a pending update so the next update follows them. |
| Create a pane (cwd, argv; sanitized environment) | Session | Only `SpawnPane` on a separate same-UID management connection, not through the SSH bridge | `spawn {request, cwd?, argv}` → `spawned {request, pane}`, same validation and retry cache as `SpawnPane`. The environment is sanitized by the server (session markers removed); clients cannot pass variables. |
| Close (terminate) a pane | Session | No | `close {request, pane}` → `done {request}`; refused while another connection holds the lease. The confirmation dialog is client UI. |
| Closed-session records: list | Session | No (composed Recovery dialog only) | `closed_sessions {request}` → `closed_sessions {request, records}`, collected by the existing background job. Records carry `key`, workspace, cwd, name, kind, session ID, argv, close time, reason, and the derived `resume` and `rerun` commands. |
| Closed-session records: restore or rerun | Session | No | `restore {request, record, rerun}` → `restored {request, record, pane, existing}`. Records are addressed by `key`, never by a list index. |
| Restore-choice labels, copy resume command | Client UI | n/a | Derived by the client from `resume`/`rerun`. |
| Notifications feed (latest 128) | Session | Yes: `Update.notifications` | None. |
| Acknowledge one notification | Session | Yes: `acknowledge` | None. |
| Acknowledge all | Session | No | `acknowledge_all` (`notification-ack-all`). |
| Notification sound, "next attention/finished" navigation, unread badges | Client UI | n/a | Computed from notifications and pane status. |
| History | Session | No on the stream (`History` on a separate connection only) | `history {request, pane, start, limit}` → `history {request, pane, lines}` (plain text, at most 200 rows). |
| Search one pane or across panes | Session | No on the stream (`SearchPane` on a separate connection only) | `search {request, panes, query, limit}` → `search {request, results, truncated}`; an empty pane list searches every pane in creation order; 64 matches and 4 MiB of context per reply. |
| Scrollback viewing and revealing a search match | Session today (emulator viewport) | No | `scroll {pane, lines}` and `reveal {pane, line}`, both lease-bound (`pane-scroll`). See "Gaps" for styled history. |
| Selection and copy | Client UI | Snapshots and OSC 52 `clipboard-write` | None: selection is computed on the client's own snapshot and history text. |
| Terminal screen, patches, input, paste, mouse, resize, per-pane leases and sizes, controller theme | Session | Yes | None. |
| Host vitals (CPU, memory, swap, disk %, clock) | Session (host observation) | Yes: `Update.vitals` | None. |
| Host status (per-CPU, memory and swap bytes, load, uptime, tasks, network and disk I/O, mounts, process table) | Session (host observation) | No (composed Status dialog only) | `host_status {enabled}` opt-in adds `Update.host` (`host-status`). Sorting, views and layout are client UI. |
| Shutdown request | Session (lifecycle) | No (separate management `Command(Shutdown)`) | `shutdown {request}` → `done`, then `server_stopping` (`server-lifecycle`). |
| Shutdown and last-pane-exit signals | Session (lifecycle) | No (the socket closed) | `server_stopping {reason}` after `session_events` opt-in; queued events get a bounded 200 ms flush before the sockets close. |
| Upgrade | Session (lifecycle) | No | No server event: a live server is never upgraded in place. Clients compare Welcome `server_version` and stream capabilities, and a new server is a new epoch. |
| Tabs, tab order, names, colours, focus and focus history, reading/grid modes, grid geometry, sidebar, dialogs, help, title animation, spinner, templates, TUI settings | Client UI | n/a | None. Tab order defaults to pane ID (creation) order; names start from `PaneInfo.title` / `PaneSession.terminal_title`. |
| Parking | Client UI | n/a | None. The server needs no "parked" concept: see below. |
| Scratch terminal | Client UI (which pane is "scratch") | n/a | None. A client creates it with `spawn` and marks it locally; for the server it is an ordinary pane. |
| Agent conversation link (Codex picker) | Session (discovery input) | No | Not in step 1; see "Gaps". |
| Unused-shell hint | Session (observation) | No (composed UI only) | Not in step 1; see "Gaps". |

## Parking and display leases

Parking needs no server state. The composed server never resized a parked pane: it simply left it out of the grid geometry. A stream client gets the same result from two existing generic operations:

- To stop receiving a pane's screen, remove it from `subscribe`. Metadata, notifications and lifecycle events keep flowing.
- To keep a hidden pane's size stable, keep its lease. A lease does not require a subscription, so an application in a hidden pane sees no `SIGWINCH`.
- To give up the size, `release` it; the pane returns to the server's base geometry.

A dedicated "this client doesn't display this pane" request would duplicate `subscribe` plus `release`, so none was added. The test `a_leased_but_unsubscribed_pane_keeps_its_size` covers this.

## Decisions

1. **Versioning.** The stream stays at version 1. Every addition is a new optional field or a new request/event type behind a new Welcome capability. Events of new types are sent only to a connection that opted in (`session_events`, `host_status`) or sent the correlated request, because current KelpClient and KelpHub decoders reject unknown event types. An older server disconnects on an unknown request type, so clients check capabilities first.
2. **Correlation.** New request/reply operations carry a client-chosen `request` string (1–128 of `A–Z a–z 0–9 - _`), echoed in the reply and in `error.request`. Fire-and-forget operations (`scroll`, `reveal`, `acknowledge_all`, opt-ins) have none, like the v1 operations.
3. **Lifecycle ordering.** Lifecycle events are reliable messages; a pending replaceable update built before an event is discarded together with that connection's screen baselines, so the next update is a fresh full update that already reflects the event.
4. **Closing the last pane** keeps the composed protocol's policy for now: `close` on the last main pane first spawns a replacement shell (reported by `pane_created`), and the last pane's exit stops the server (`server_stopping {reason: last_pane_exited}`). Whether a session server may run with zero panes is a step 3 decision.
5. **Closed-session records stay in the server** and stay generic: they are derived from server observations (foreground programs, agent sessions), shared by every client, and restored by opaque `key`. Only their presentation is client UI.
6. **Scroll stays a lease-bound server operation in step 1** because local rendering of scrollback needs styled history, which does not exist yet.
7. **Host status data stays in the server** (`status.rs` sampling); its presentation (views, sorting, columns, formatting) is client UI.

## Gaps for step 2

- **Styled history.** `history` returns plain text. A locally rendered scrollback with colours needs paged rows of cells with stable row anchors (ADR 0001, "Terminal history and synchronization"). Until then, a TUI scrolls with `scroll`/`reveal` and renders the scrolled snapshot.
- **Agent conversation link.** The Codex conversation picker (`AgentLinkPicker`/`AgentLinkChoose`) lists sessions through the discovery worker and binds one to a pane. A stream version would be `agent_sessions {request, pane}` and `link_agent {request, pane, session}`.
- **Unused-shell hint.** The composed UI marks shells that have no child process and no recent input. It could become an observation field (for example `PaneSession.idle_shell`); a client can approximate it with `status == idle` and no `command`.
- **Spawn with another shell.** Templates may name a shell; `spawn` uses the server's shell (a template can still run a program through `argv`).
- **Initial panes.** The server still creates the composed grid's initial panes and restores `tab-layout.json` on start. A stream TUI restores its own layout by spawning; the server's startup policy changes in step 3.
- **Diagnostics** remain a separate same-UID management request.
