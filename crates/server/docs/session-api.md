# Session API inventory

This is step 1 of [ADR 0002](adr/0002-generic-session-server.md): the inventory of what today's KelpMux TUI obtains from the composed protocol (local protocol 58), sorted by the boundary rule, and what the pane stream now carries so a TUI can run as a stream client with local rendering and local UI state. The wire contract is in [native-client-protocol.md](native-client-protocol.md#session-api); this page records the classification and the gaps. The [step 2 server support](#step-2-server-support-2026-11-21) closes the gaps the stream TUI found; rows marked "Step 2" record it. [Step 3](#step-3-2026-11-21) removed the composed protocol from the server (KelpServer 0.3); the "Before" column now describes history.

"Session" means the server keeps it. "Client UI" means each client owns it, keyed by pane identity (server epoch plus pane ID), and the server stores none of it. "Before" is the stream protocol at `0837dd2`; "Step 1" is what this change adds. All additions are additive and capability-gated; the stream version stays 1.

| Capability | Session or client UI | On the stream before step 1 | Step 1 addition |
| --- | --- | --- | --- |
| Server epoch and pane identity | Session | Yes: Welcome `epoch`; pane IDs scoped to it | Documented that pane IDs increase monotonically and are never reused within an epoch, so they also give creation order. |
| Server version | Session (lifecycle) | No | Welcome `server_version` (`server-lifecycle`). |
| Pane list with process, cwd, agent kind, session ID, status, evidence, activity times and shell command | Session (observations) | Yes: `Update.panes` on every update, including a metadata-only subscription (`subscribe []`) | None. Since 0.3.0-beta.2 `PaneInfo` has no composed-UI fields (`parked`, `number`, `palette` and title aliases were removed); tab numbers, parking and colours are client state keyed by pane ID. |
| Pane session facts: creation time, launch directory, shell, spawn arguments, startup command, raw terminal title | Session | No | `Update.sessions` (`PaneSession`) after `session_events` opt-in (`pane-lifecycle`). |
| Pane created | Session | No (a new ID appeared in `Update.panes`) | `pane_created {session}` event. |
| Pane exited (exit code or signal) | Session | No (`dead`/`Exited` status, briefly) | `pane_exited {pane, at, code, signal}` event. |
| Pane closed (by exit or by a client), with its closed-session record | Session | No (the ID disappeared) | `pane_closed {pane, at, reason, record}` event. Events invalidate a pending update so the next update follows them. |
| Create a pane (cwd, argv; sanitized environment) | Session | Only `SpawnPane` on a separate same-UID management connection, not through the SSH bridge (removed in step 3) | `spawn {request, cwd?, argv}` → `spawned {request, pane}`, with the validation and retry cache `SpawnPane` had. The environment is sanitized by the server (session markers removed); clients cannot pass variables. Step 2: optional `shell` (`spawn-shell`), part of the retry identity; `PaneSession.spawned_by` names the creating connection (`spawned-by`). |
| Close (terminate) a pane | Session | No | `close {request, pane}` → `done {request}`; refused while another connection holds the lease. The confirmation dialog is client UI. |
| Closed-session records: list | Session | No (composed Recovery dialog only) | `closed_sessions {request}` → `closed_sessions {request, records}`, collected by the existing background job. Records carry `key`, workspace, cwd, name, kind, session ID, argv, close time, reason, and the derived `resume` and `rerun` commands. |
| Closed-session records: restore or rerun | Session | No | `restore {request, record, rerun}` → `restored {request, record, pane, existing}`. Records are addressed by `key`, never by a list index. |
| Restore-choice labels, copy resume command | Client UI | n/a | Derived by the client from `resume`/`rerun`. |
| Notifications feed (latest 128) | Session | Yes: `Update.notifications` | None. Step 2: Welcome `notification_run` (`notification-run`), the run that notification origins are matched against. |
| Acknowledge one notification | Session | Yes: `acknowledge` | None. |
| Acknowledge all | Session | No | `acknowledge_all` (`notification-ack-all`). |
| Notification sound, "next attention/finished" navigation, unread badges | Client UI | n/a | Computed from notifications and pane status. |
| History | Session | No on the stream (`History` on a separate connection only) | `history {request, pane, start, limit}` → `history {request, pane, lines}` (plain text, at most 200 rows). |
| Search one pane or across panes | Session | No on the stream (`SearchPane` on a separate connection only) | `search {request, panes, query, limit}` → `search {request, results, truncated}`; an empty pane list searches every pane in creation order; 64 matches and 4 MiB of context per reply. |
| Scrollback viewing and revealing a search match | Session today (emulator viewport) | No | `scroll {pane, lines}` and `reveal {pane, line}`, both lease-bound (`pane-scroll`). See "Remaining gaps" for styled history. |
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
| Agent conversation link (Codex picker) | Session (discovery input) | No | Dropped: not planned (step 2 decision). |
| Unused-shell hint | Session (observation) | No (composed UI only) | Step 2: `PaneSession.unused_shell` (`unused-shell`). |
| Startup panes (composed startup tabs, `tab-layout.json`) | Session (startup policy); the layout itself is client UI | No | Step 2: `--startup-panes layout\|none` and Welcome `startup_panes` (`startup-policy`). Step 3: always zero panes; `layout` removed. |
| Recovery popup after an unclean shutdown | Session (lifecycle signal); the popup is client UI | No (composed UI only) | Step 2: Welcome `recovery_pending` and `dismiss_recovery` (`recovery-signal`). |
| Subscription bounds | Session (resource limits) | Hardcoded: 10 panes, 100,000 cells | Step 2: 64 panes, Welcome `limits` (`stream-limits`). |

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
4. **Closing the last pane** keeps the composed protocol's policy (confirmed in step 3): `close` on the last main pane first spawns a replacement shell (reported by `pane_created`), and the last pane's exit stops the server (`server_stopping {reason: last_pane_exited}`). A server starts with zero panes (decision 8), but `close` never leaves zero panes: a client that wants an empty server closes its panes and lets the last one exit, or stops the server.
5. **Closed-session records stay in the server** and stay generic: they are derived from server observations (foreground programs, agent sessions), shared by every client, and restored by opaque `key`. Only their presentation is client UI.
6. **Scroll stays a lease-bound server operation in step 1** because local rendering of scrollback needs styled history, which does not exist yet.
7. **Host status data stays in the server** (`status.rs` sampling); its presentation (views, sorting, columns, formatting) is client UI.
8. **Zero panes (step 2; the only startup since step 3).** A server runs with zero panes until a client spawns; nothing in the server needs a pane, and only the last pane's exit stops it. `close` on the last pane still creates a replacement shell first, so a running server never drops back to zero panes through `close`.
9. **Unused shell is an observation, recovery is a lifecycle signal (step 2).** Both are derived from server state and useful to any client; the hint's presentation and the recovery popup stay client UI. Neither depends on leases or on which client asked.
10. **Subscription bounds (step 2).** The pane bound rose from 10 to 64; the 100,000-cell bound stays, because a backpressured update carries every subscribed screen in one record capped at 16 MiB (about 100–160 JSON bytes per cell), and a grid on one terminal never covers more than that terminal (at most 64,000 cells). Hidden panes are unsubscribed rather than counted.

## Step 2 server support (2026-11-21)

These gaps found by the stream TUI are closed, each behind its own capability (contract in [native-client-protocol.md](native-client-protocol.md#step-2-additions)):

- **Initial panes** (`startup-policy`): `--startup-panes none` starts without panes and leaves the composed UI's `tab-layout.json` and saved tab count untouched; Welcome `startup_panes` reports the policy.
- **Recovery popup** (`recovery-signal`): Welcome `recovery_pending` after an unclean shutdown, cleared for every client by `dismiss_recovery` or by the composed popup.
- **Spawn with another shell** (`spawn-shell`): `spawn {shell}`, validated like the workspace shell and part of the retry identity.
- **Unused-shell hint** (`unused-shell`): `PaneSession.unused_shell`, the composed UI's logic and rate limits, sampled while a `session_events` connection is attached.
- **Subscription limit** (`stream-limits`): 64 panes, 100,000 cells, advertised in Welcome `limits`.
- **Notification run** (`notification-run`): Welcome `notification_run`, the run that notification `origin.run` is compared with, so a client matches notifications to live panes like the composed UI instead of guessing the run.
- **Pane creator** (`spawned-by`): `PaneSession.spawned_by`, the Welcome `client` ID of the connection whose `spawn` or `restore` created the pane; kept for the epoch only.

## Remaining gaps

- **Styled history.** `history` returns plain text. A locally rendered scrollback with colours needs paged rows of cells with stable row anchors (ADR 0001, "Terminal history and synchronization"). Until then, a TUI scrolls with `scroll`/`reveal` and renders the scrolled snapshot. Not planned for step 2.
- **Agent conversation link.** The Codex conversation picker (`AgentLinkPicker`/`AgentLinkChoose`) is dropped from the stream TUI; no stream request is planned.
- **Diagnostics** became the stream `diagnostics` request in step 3 (the one-shot `"Diagnostics"` record stays for tools without a stream session).


## Step 3 (2026-11-21)

KelpServer 0.3.0 removes the composed protocol 58 and the server-side UI. Contract details are in [native-client-protocol.md](native-client-protocol.md#step-3-generic-server-03).

- **Removed:** the composed frames and their deltas, `Action` handling and the controller, mouse routing, title animation, the controller theme fallback, the tab/grid/parking/focus/dialog/scratch state, templates as a startup source, writing `tab-layout.json` and UI keys of `workspace.json`, the composed startup layout (`--startup-panes layout`), the unused-shell UI hint (the observation stays), the recovery popup (the signal stays), the DEV badge, sound and visual notifications, the `SpawnPane`/`InspectPane`/`History`/`SearchPane`/`Command` management requests and the `pane-spawn` capability.
- **Administration on the stream:** `shutdown`, the new `diagnostics` request (`diagnostics` capability), and the pane list from the first `Update`. `kelpserver start`, `status`, `stop` and `ctl` use them; `stop` keeps a legacy shutdown fallback for a running server without `server-lifecycle`. Diagnostics also answer a one-shot `"Diagnostics"` record without Hello.
- **Errors instead of hangs:** an undecodable record, including every composed-protocol message, gets `{"Error":"…"}` naming the removal, then the connection closes.
- **Versions:** crate `kelpserver` 0.3.0-beta.1 and `kelpclient-protocol` 0.2.0 (without `LOCAL_PROTOCOL_VERSION`); the stream stays at version 1. `server.identity.json` records stream version 1 in `protocol`, where 0.2 servers recorded 58.
- **Kept for clients, not used by the server:** `templates` (a client startup layout format), the vitals presentation helpers, `history::RestoreChoice` and the `PaneInfo` fields `parked`, `number` and `palette`, which the server now fills with neutral values (never parked, creation-order position, ID-derived palette).

## Uploads (2026-11-21, decision 151a)

File and image uploads dropped onto a terminal in a client are a server capability (`uploads`), because KelpMux and KelpHub both need them and the files must land where the shells run: the user's machine for KelpMux, the workspace pod for KelpHub, which may have no SSH daemon. Clients own the drop UI and paste the resulting shell-quoted paths into the pane. Contract: [native-client-protocol.md](native-client-protocol.md#uploads-uploads).

| Capability | Session or client UI | Addition |
| --- | --- | --- |
| Upload files to the server host | Session (files where the shells run) | `upload_begin {request, files, dest?}` → `upload_started {request, batch, dir}`; `upload_chunk {batch, file_index, offset, data}` → `upload_progress {batch, file_index, received}` (window of `max_chunks_in_flight` unacknowledged chunks); `upload_finish` → `upload_done {request, batch, paths}`; `upload_cancel` → `done`; `upload_failed {batch, message}` when a chunk fails. Welcome `upload_limits`. Batches are private per-batch directories scoped to the connection; completed batches are kept for a configurable retention (default 7 days). |
| Drop target, progress display, pasting the quoted paths | Client UI | None: the client pastes the `upload_done` paths with `paste` or `input`. |

Decisions:

1. **Encoding.** Chunks are base64 strings in the JSON record: the stream has no binary framing, and the byte arrays `input` uses cost up to four bytes per byte. A 256 KiB chunk is about 342 KiB on the wire, well under the 16 MiB record cap, so pane updates are not starved. Measured locally through the Unix socket (release build, 100 MiB file, tmpfs): about 420–540 MB/s; a debug build about 15 MB/s. No binary framing is needed.
2. **Authorization.** Any stream connection may upload: the same trust as typing into a shell, which any connection may do after claiming a pane.
3. **Order.** Files are sent in request order and each file at strictly sequential offsets, so the server holds one open file per batch.
4. **Failure.** Any failing chunk or finish removes the whole batch; there is no resume. A disconnect removes the connection's unfinished batches.
5. **Retention.** Only batches recorded in the server's `uploads.json` (path, device, inode) are ever deleted, without following symlinks. Unfinished batches of a previous run are removed at startup whatever the retention.

- **Kept for clients, not used by the server:** `templates` (a client startup layout format), the vitals presentation helpers, `history::RestoreChoice` and the `PaneInfo` fields `parked`, `number` and `palette`, which the server filled with neutral values (never parked, creation-order position, ID-derived palette). All of them were removed in 0.3.0-beta.2; see [below](#client-leftovers-removed-2026-11-21).

## Client leftovers removed (2026-11-21)

KelpServer 0.3.0-beta.2 and `kelpclient-protocol` 0.3.0 remove what step 3 kept only because KelpMux imported it; KelpMux keeps its own copies.

- **Removed:** `native::web_relay` and `kelpserver web-relay` (KelpMux has `kelpmux web-relay`), `native::templates`, the vitals presentation functions (`bar_glyph`, `pie_glyph`, `render_segments`, `is_clock_segment`), `history::RestoreChoice` and `Entry::restore_choices`, `upgrade::attach_matching` and `upgrade::matching_binary` (client binary selection), the tmux label escape `label`, the always-empty alias parameter of `tracking::tab_name` and `tracking::shell_tab_name`, and in the protocol crate the `PaneInfo` fields `parked`, `number` and `palette`, `Status::text`, `Status::marker`, `WORKING_FRAMES` and `StatusSource::label`.
- **Kept:** the structured `Vitals` sent in every `Update` and the `Sampler` (including `disk_readings`, the mount paths behind the disk percentages); closed-session records with `Entry::command` and `Entry::previous_command`, which give `resume`/`rerun` their meaning and `ClosedSession.resume`/`rerun` their values; `upgrade::Identity`, `save`, `identify`, `protocol_label` (printed by `kelpserver status`), `speaks_stream` and `LEGACY_COMPOSED_PROTOCOL`, which any client needs to check a running server; `PaneInfo.title`, which is the server's observation of the pane title, not a client name.
- **Wire effect:** `Update.panes` no longer carries `parked`, `number` or `palette`. The stream version stays 1; a client built against `kelpclient-protocol` 0.2.0 requires `number` and `palette` and must move its pin together with the server. See [native-client-protocol.md](native-client-protocol.md#step-3-generic-server-03).
- **Tolerant Welcome:** `StartupPanes` gains `Layout` (sent only by 0.2 servers started with `--startup-panes layout`) and `Unknown` (any other value), so a 0.3 client can still attach to, list and stop such a workspace. A 0.3 server still sends only `none`, and `--startup-panes` still accepts only `none`.
