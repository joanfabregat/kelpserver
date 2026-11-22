# Native client foundation

KelpServer owns long-running PTYs and Alacritty terminal interpretation. Clients own presentation. Native mode has no tmux or WezTerm runtime dependency. Since 0.3 ([ADR 0002](adr/0002-generic-session-server.md) step 3) the server speaks only the pane stream, version 1, on its local socket and through the SSH bridge: the composed-screen protocol 58 and all server-side UI (tabs, parking, dialogs, layout, frames) were removed. A record of the old protocol gets an `Error` reply and the connection closes. A graphical desktop/mobile client and MCP/device tools are not implemented.

The broader proposed contract is [ADR 0001](adr/0001-native-client-protocol-and-device-capabilities.md). It covers pane lifecycle, independent device sizes, live agent status, notifications, remote Vitals and a future MCP/device request channel. The ADR deliberately distinguishes those proposals from the implemented requests below.

KelpMux's TUI and terminals-only browser client are stream clients that render locally; the browser client and its `kelpmux web-bridge` are described in [KelpMux docs/web.md](https://github.com/joanfabregat/kelpmux/blob/main/docs/web.md). KelpHub keeps its own browser application and authenticated management adapter. Shared portable types and terminal client state live in [KelpClient](https://github.com/joanfabregat/kelpmux/blob/main/crates/client/README.md) in KelpMux. Desktop/mobile implementation choices remain deferred.

Screen input modes include an additive, default-zero `keyboard_flags` bitfield using the five Kitty keyboard enhancement flags. Native clients must preserve modified Enter events and distinguish key release from key press. Pane snapshots preserve default foreground/background colors so the client's terminal or renderer supplies its theme. Explicit application colors and OSC palette changes remain pane-local.

A pane's lease owner can supply its OSC 10/11 query fallbacks with the `controller-theme` request (below). Application OSC colors take priority, while default snapshot cells remain `Default`. A pane without a lease theme answers with documented emulator fallbacks. Theme reports do not force an existing application to repeat its startup probes or replace explicit application RGB colors.

## SSH command channel

`kelpserver bridge --workspace NAME` relays bounded native-stream records between standard input/output and the existing owned private Unix socket. It does not start a workspace. A future native client opens it through one authenticated SSH command channel without an outer PTY:

```sh
ssh -T HOST kelpserver bridge --workspace NAME
```

Use the installed binary's absolute path if it is not in the remote noninteractive PATH. SSH supplies authentication, encryption and transport; there is no additional public port. Shell banners must not appear on stdout. Bridge diagnostics go to stderr, and a TTY endpoint is rejected before terminal-mode changes. EOF detaches that connection and releases its leases, preserving workspace processes. Unacknowledged requests are not automatically replayed. The bridge does not provide mobile roaming or background push delivery.

## Framing and negotiation

Every record has a four-byte unsigned big-endian payload length followed by UTF-8 JSON, with a 16 MiB maximum payload. JSON examples below are payloads, not newline-delimited wire records. Use the same framing over the local Unix socket or SSH bridge.

```json
{"Stream":{"type":"hello","version":1}}
```

The server responds with `Stream` / `welcome`, containing stream version, connection/client ID, a random server epoch, and supported capabilities. Send Hello exactly once before other stream requests. Unsupported versions fail clearly. Current capabilities are `pane-snapshots`, `pane-patches`, `pane-leases`, `metadata`, `notifications`, `vitals`, `hyperlinks`, `program-status`, `controller-theme` and `clipboard-write`, plus the [session API](#session-api) capabilities `pane-lifecycle`, `closed-sessions`, `pane-history`, `pane-scroll`, `notification-ack-all`, `host-status`, `server-lifecycle`, `startup-policy`, `recovery-signal`, `spawn-shell`, `unused-shell`, `stream-limits`, `notification-run`, `spawned-by`, `diagnostics` and `uploads`; do not infer support for undeclared future capabilities. A server with `server-lifecycle` also puts its package version in Welcome `server_version`.

The socket envelope has three client records and three server records. A client sends `{"Stream":{…}}` requests, or the one-shot `"Diagnostics"` on a connection that has not sent Hello. The server sends `{"Stream":{…}}` events, `{"Diagnostics":{…}}` in reply to the one-shot request, and `{"Error":"…"}` for a record it cannot accept. A record it cannot decode, such as one of the removed composed protocol 58 (`Attach`, `Command`, `Inspect`, `SpawnPane`, …), gets an `Error` that names the removal and the connection closes, so an old client reports an incompatible server instead of hanging. A stream connection that sends `"Diagnostics"` gets an `Error` and keeps its connection. The bridge accepts only stream requests.

Clients recognizing `clipboard-write` may opt in with `{"Stream":{"type":"clipboard","enabled":true}}`. Application OSC 52 writes then produce `clipboard` events containing `pane` and UTF-8 `text` (at most 64 KiB), delivered only to that pane's current lease owner. Observers and clients that have not opted in receive no clipboard data. Writes from panes without a stream owner are dropped. Clipboard events are ephemeral: never replay them in snapshots, history or reconnect state. Clipboard reads remain disabled. Browser clients must provide an explicit copy fallback when platform user-activation rules prevent an asynchronous write.

The `pane-spawn` capability and its separate-connection `SpawnPane` request were removed in 0.3; create panes with the stream `spawn` request of the [session API](#pane-lifecycle-pane-lifecycle).

Pane IDs are scoped to the Welcome epoch. Within an epoch they increase monotonically and are never reused, so they also give creation order. A new epoch invalidates previous screen baselines and ownership. Clients must negotiate anew, obtain snapshots, and deliberately rebuild desired control state rather than replaying old mutations. Client UI state keyed by pane identity (epoch plus pane ID) does not survive a new epoch. The v1 operations carry no correlation ID or expected epoch; the session API requests carry a correlation ID (below). An expected epoch on every mutation remains part of public-contract maturation in the ADR.

## Pane subscriptions and updates

```json
{"Stream":{"type":"subscribe","panes":[1,2]}}
```

Subscribe replaces the complete subscription set. Updates contain an authoritative `subscribed` list, full metadata for current panes, leases, notifications, Vitals, and snapshots or patches for subscribed terminal screens. Metadata-only subscriptions use an empty list. The reference client prunes cached screens according to each received update's subscription list, safely processing older in-flight updates before a replacement set arrives.

A full pane snapshot establishes revision 1. A patch names the pane, base revision and resulting revision; it applies only to that exact pane baseline in the current server epoch. Geometry changes, explicit `resync` and backpressure can produce replacement snapshots. Snapshots contain cells with colors/attributes/widths, cursor and input modes; they do not contain native headers, tabs or footer cells. Alacritty remains the only emulator answering application terminal queries.

There are at most 64 subscribed panes and 100,000 aggregate subscribed screen cells per connection (ten panes before `stream-limits`); a server with `stream-limits` advertises both in Welcome `limits`, so clients should read them rather than hardcode them. The cell budget keeps a backpressured update, which carries every subscribed screen as a snapshot, under the 16 MiB record limit (a cell serializes to about 100–160 bytes); it does not limit grids, because the panes displayed on one terminal together cover at most that terminal (the largest native pane is 400 × 160 = 64,000 cells). Oversized subscriptions are rejected. A later size change exceeding an observer's aggregate budget disconnects that observer instead of retaining unbounded data. The server retains one in-flight packet and one replaceable aggregate update. The bridge has separate bounded queues in each direction and yields between decode batches. A permanently slow client can be disconnected and resynchronize on reconnect.

## Independent ownership and size

`claim {pane,takeover}` explicitly acquires one pane; `release {pane}` relinquishes it. `resize {pane,cols,rows}`, `input {pane,bytes}`, `paste {pane,bytes}` and `mouse {pane,event}` require the caller's lease. Input/paste are bounded to 64 KiB. Paste honors the pane's bracketed-paste mode; native mouse events currently support SGR encoding. An observer cannot write or resize a pane.

The optional `controller-theme` capability adds `theme {pane,foreground:[r,g,b],background:[r,g,b]}` with byte-valued RGB channels. It requires the caller's pane lease and sets that pane's OSC 10/11 query fallbacks. The browser reports its theme after claiming and when its appearance changes. Takeover or release drops the old lease theme, restoring the next controller's colors. Do not send this request to retained servers that omit the capability.

Client A can own pane 1 at 240 × 70 cells while client B owns pane 2 at 40 × 12 cells. Native pane sizes permit 2–400 columns and 1–160 rows. Each pane still has exactly one underlying PTY size. Observers of that same pane clip/scale locally and cannot impose a competing size.

Leases last until release, takeover or disconnect; release or disconnect returns the pane to the server's base size, set by `--size` (default 120 × 40). Stream clients issue explicit Claim messages, which a UI may send automatically on focus or input; background resize, mouse movement and focus-out should never steal a lease. The browser retains its last geometry and claims on blur or document hiding; actual foreground input or another explicit claim can still take over. Release is idempotent for an already-free pane. Workspace administration and explicit shutdown are same-user operations on the stream (`shutdown`, `diagnostics`); this is not isolation against hostile processes running as that user.

## State, notifications and Vitals

The `program-status` capability adds `terminal` observation evidence from pane-local OSC 7501 reports. Current dev clients must recognize this status-source value; the experimental stream remains v1 and is not a frozen compatibility contract. Terminal activity takes precedence over metadata polling without implying that a conversation ID is known.

Pane metadata carries identity, agent kind, session ID if known, status, activity times and status observation evidence. Evidence distinguishes process, lifecycle hook, live record, agent-server and transcript observations. `checked_at` records a check; optional `event_at` records an actual known event time. Reading an old transcript again does not invent a new lifecycle event. Unknown or stale evidence remains visible in the overview, and an idle observation does not become a completed turn automatically.

Updates include the latest 128 retained notifications and explicit `acknowledge {notification}`. This is a bounded snapshot of notification history, not exactly-once event delivery or a question/answer RPC. Existing seven-day retention and persisted unread state remain. No native device notification or push service is invoked.

Vitals are typed values from the existing in-process sampler, independent of footer styling: CPU, memory, swap and disk percentages, plus server-local date/time strings. Missing values stay unavailable. Sample timestamps, byte totals, mount labels, timezone-aware conversion and richer metrics are proposed additions; clients must not treat the current clock strings as timezone-independent timestamps.

## Reference client and inspection

The Rust `native::reference::State` validates welcome/epoch, membership, screen/cell bounds, leases and patch baselines before committing changes. Failed updates leave previous baselines intact. `stream-inspect` is a read-only one-shot reference client with an absolute operation deadline:

```sh
kelpserver stream-inspect --workspace NAME
kelpserver stream-inspect --workspace NAME 1 2
```

With no pane IDs it returns metadata only; explicit IDs add those panes' terminal cells to JSON stdout. It does not claim, resize, start a server or persist terminal contents. Automated tests use actual bridge subprocess pipes, disposable PTYs and typed reference state; they do not open a real SSH login or access a native device.

History and search are stream requests (`pane-history`, below). Search returns case-insensitive literal occurrences; queries must contain 1–256 bytes without control characters. Each match carries its source buffer, immutable context rows, and inclusive cell coordinates relative to that context, so ongoing output or alternate-screen transitions cannot move the target between search and reveal. Search does not change another client's viewport or input ownership. History pages use relative line indices and plain text; stable, styled native-history pagination is not implemented. Pane renaming is client UI. Device capability requests, MCP, native apps and final public-protocol compatibility guarantees remain future work described in ADR 0001.

### Hyperlink cells

The server's `hyperlinks` capability indicates support for an optional `hyperlink` URI string per cell. Missing fields mean no target; explicit targets and recognized plain HTTP(S) URLs are carried independently of visible text. Targets are bounded to 2,048 bytes, require a URI scheme and reject whitespace and control characters. Patches include target-only changes. Native clients open links through their own user interaction rather than requesting browser execution on the server. Older cell encodings remain decodable.

## Session API

The session API is step 1 of [ADR 0002](adr/0002-generic-session-server.md): the session-level operations a full TUI needs to run as a stream client with local rendering and local UI state. Server support for step 2 (the stream TUI's parity gaps) adds the [step 2 capabilities](#step-2-additions). The classification of every TUI capability, and what remains, is in [session-api.md](session-api.md). [Step 3](#step-3-generic-server-03) removed the composed protocol 58 and added `diagnostics`.

### Compatibility

The stream stays at version 1; nothing in v1 changed meaning. Every addition is either a new optional field or a new request or event type behind one of these Welcome capabilities:

| Capability | Requests | Events and fields |
| --- | --- | --- |
| `pane-lifecycle` | `session_events {enabled}`, `spawn`, `close` | `pane_created`, `pane_exited`, `pane_closed`, `spawned`, `done`, `Update.sessions` |
| `closed-sessions` | `closed_sessions`, `restore` | `closed_sessions`, `restored` |
| `pane-history` | `history`, `search` | `history`, `search` |
| `pane-scroll` | `scroll`, `reveal` | none |
| `notification-ack-all` | `acknowledge_all` | none |
| `host-status` | `host_status {enabled}` | `Update.host` |
| `server-lifecycle` | `shutdown` | `server_stopping`, Welcome `server_version` |
| `startup-policy` | none (server option `--startup-panes none`) | Welcome `startup_panes` (always `none` since 0.3) |
| `recovery-signal` | `dismiss_recovery` | Welcome `recovery_pending`, `done` |
| `spawn-shell` | `spawn` field `shell` | none |
| `unused-shell` | none | `PaneSession.unused_shell` |
| `stream-limits` | none | Welcome `limits` |
| `notification-run` | none | Welcome `notification_run` |
| `spawned-by` | none | `PaneSession.spawned_by` |
| `diagnostics` | `diagnostics` | `diagnostics` |
| `uploads` | `upload_begin`, `upload_chunk`, `upload_finish`, `upload_cancel` | `upload_started`, `upload_progress`, `upload_done`, `upload_failed`, `done`, Welcome `upload_limits` |

Rules that keep older peers working:

- A connection receives an event of a new type only after it opts in (`session_events`, `host_status`) or as the reply to a request it sent. Existing clients that reject unknown event types (KelpClient, KelpHub's bridge) therefore never see one. `Update.sessions` and `Update.host` are omitted unless enabled. Step 2 adds no event type; step 3 adds the `diagnostics` reply; `uploads` adds only replies to its own requests.
- New fields on existing messages (`Welcome.server_version`, `Welcome.startup_panes`, `Welcome.recovery_pending`, `Welcome.limits`, `Welcome.notification_run`, `Welcome.upload_limits`, `Error.request`, `Update.sessions`, `Update.host`, `PaneSession.unused_shell`, `PaneSession.spawned_by`) are optional; decoders that ignore unknown fields are unaffected. Absent fields are omitted from the wire (`recovery_pending` when false, `spawned_by` when unknown).
- `spawn` without a shell serializes exactly as before, so it stays valid for a server without `spawn-shell`; with `shell`, such a server rejects the request.
- A server without a capability disconnects a connection that sends an unknown request type. Check the Welcome capabilities before sending any request listed above.
- The SSH bridge relays every stream request after Hello, so the session API works through `kelpserver bridge` unchanged.

In the Rust API, `Event::Welcome`, `Event::Error` and `Update` gained fields and `Request`/`Event` gained variants, so exhaustive matches and struct literals in consumers need updating when they move their pin. Step 2 adds fields to `Event::Welcome`, `Request::Spawn`, `PaneSession` and `ServerConfig`, and the `Request::DismissRecovery` variant; patterns with `..` are unaffected. Step 3 (0.3.0, a breaking release) removes `StartupPanes::Layout`, the composed `ClientMessage`/`ServerMessage` variants (only `Stream`, `Error` and `Diagnostics` remain) and the UI fields of `ServerConfig` (it keeps `socket`, `state_dir`, `shell`, `cwd`, `cols` and `rows`), and adds `Request::Diagnostics` and `Event::Diagnostics`. The `uploads` capability adds `Event::Welcome.upload_limits`, `ServerConfig.uploads` (`uploads::Settings`), the `lifecycle::Settings` fields `upload_dir` and `upload_retention_days`, the `Request::Upload{Begin,Chunk,Finish,Cancel}` and `Event::Upload{Started,Progress,Done,Failed}` variants and the `uploads` module.

### Correlation and errors

Requests that have a reply carry a client-chosen `request` string of 1–128 ASCII letters, digits, `-` or `_`. The reply echoes it, and so does `{"type":"error","message":…,"request":…}` when such a request fails. A malformed identifier fails with an error without `request`. Errors of v1 requests still have no `request`. Fire-and-forget requests (`scroll`, `reveal`, `acknowledge_all`, the opt-ins) have no identifier and no reply, like `resize` or `acknowledge`.

### Pane lifecycle (`pane-lifecycle`)

```json
{"Stream":{"type":"session_events","enabled":true}}
```

After the opt-in, each update carries `sessions`, one `PaneSession` per live pane in ID order: `pane`, `created_at` (Unix seconds), `launch_cwd`, `shell`, `argv` (the `spawn` arguments, otherwise empty), `startup` (the command line typed into the shell at startup, if any) and `terminal_title` (the application's latest OSC title, unmodified). `Update.panes` keeps carrying the observations: current cwd, process, agent kind and session, status and evidence, activity times and shell command.

Lifecycle events follow, for every pane whatever created or closed it:

- `{"type":"pane_created","session":{…}}`;
- `{"type":"pane_exited","pane":3,"at":…,"code":0,"signal":null}`, when the process exits, always followed by `pane_closed`;
- `{"type":"pane_closed","pane":3,"at":…,"reason":"exited"|"closed","record":"<key>"|null}`, where `record` names the closed-session record created for the pane, if any.

Events are reliable messages. A replaceable update that was built before an event and is still pending is discarded with that connection's screen baselines, so the next update (a full one) already reflects the event. Updates that were already on the wire may precede it.

```json
{"Stream":{"type":"spawn","request":"r1","cwd":"/home/me/src","argv":["htop"]}}
{"Stream":{"type":"close","request":"r2","pane":3}}
```

`spawn` takes an optional absolute `cwd` and an `argv` list; `cwd` defaults to the server's launch directory and an empty `argv` opens an interactive shell. Arguments are quoted as data and run before `exec $SHELL -i`. The server removes agent and multiplexer session markers from the environment; clients cannot pass variables. A retry with the same `request`, directory and arguments returns the same pane (bounded 512-entry cache per epoch). The reply is `spawned {request, pane}`, after `pane_created`.

`close` terminates the pane's process group and removes the pane; the reply is `done {request}`, after `pane_closed`. It is refused while another connection holds the pane's lease. Asking the user for confirmation is client UI. Closing the last pane first creates a replacement shell, so a running server never drops to zero panes through `close`; the last pane's own exit stops the server.

### Closed sessions (`closed-sessions`)

```json
{"Stream":{"type":"closed_sessions","request":"r3"}}
{"Stream":{"type":"restore","request":"r4","record":"<key>","rerun":false}}
```

`closed_sessions` replies `closed_sessions {request, records}` once the existing background collection finishes (at most 256 records, newest first, from this workspace and sibling workspaces of the same user, seven-day retention). A record has `key`, `workspace`, `pane` (its old ID), `cwd`, `name`, `kind`, `session_id`, `argv`, `closed_at`, `reason`, and the derived `resume` (the agent resume command) and `rerun` (the program command line). `restore` addresses a record of the latest list by `key`; it resumes an agent conversation if known, reruns a program only with `rerun: true`, and otherwise opens a shell in the record's folder. The reply is `restored {request, record, pane, existing}`; `existing: true` means the conversation was already open in `pane` and nothing was started. A restored record is consumed. A conversation running in another workspace is refused.

### History, search and scrolling (`pane-history`, `pane-scroll`)

```json
{"Stream":{"type":"history","request":"r5","pane":1,"start":-500,"limit":200}}
{"Stream":{"type":"search","request":"r6","panes":[],"query":"error","limit":64}}
```

`history` returns `history {request, pane, lines}`: plain-text rows with relative line indices (at most 200). `search` returns `search {request, results, truncated}`; each result is `{pane, hits, truncated}` with the immutable-context matches described in [Reference client and inspection](#reference-client-and-inspection). An empty `panes` list searches every pane in ID (creation) order. One reply holds at most `limit` (1–64) matches and 4 MiB of context; `truncated` reports that a limit stopped the search. Neither request needs a lease or changes any viewport.

`scroll {pane, lines}` and `reveal {pane, line}` require the pane's lease and move its emulator viewport, which every subscriber of that pane sees in its snapshots. `reveal` takes a primary-history line (a `history` index, or a match's `first_line + start.y`) and fails while the application uses the alternate screen. They exist until styled history lets a client render scrollback itself.

### Notifications, host status and server lifecycle

`acknowledge_all` marks every retained notification read (`notification-ack-all`).

`{"type":"host_status","enabled":true}` adds `host` to that connection's updates (`host-status`): per-CPU usage, memory and swap bytes, load, uptime, task counts, network and disk I/O rates, mounted disks and the process table, sampled on demand about once a second while at least one connection wants it. `ready` is false until the first sample. Views, sorting and formatting are client UI.

`shutdown {request}` replies `done` and stops the server and its panes (`server-lifecycle`). A connection that enabled `session_events` receives `server_stopping {reason: "shutdown"|"last_pane_exited"}` before its socket closes; the server gives queued messages up to 200 ms. There is no upgrade event: a live server is never upgraded in place. A client detects a different server by its epoch and `server_version`, and its features by the capabilities.

### Step 2 additions

Server support for ADR 0002 step 2, so a stream TUI reaches parity with the composed one. All of it is additive under the capabilities in the table above.

```json
{"Stream":{"type":"welcome","version":1,"client":4,"epoch":"…","capabilities":[…],"server_version":"0.3.0-beta.1","startup_panes":"none","recovery_pending":true,"limits":{"subscriptions":64,"subscribed_cells":100000},"notification_run":"…"}}
```

**Startup policy (`startup-policy`).** In step 2, `kelpserver server` and `kelpserver start` accepted `--startup-panes layout|none`, where `layout` kept the composed UI's startup tabs. Since 0.3 the server always starts with zero panes and the client that started it creates its layout with `spawn`; `--startup-panes none` is still accepted and `layout` fails with an error. The server works without panes: only the last pane's exit (not an empty pane list) stops it. It never reads or writes `tab-layout.json` or client keys in `workspace.json`. Welcome `startup_panes` is always `none`. Closing the last pane with `close` still creates a replacement shell first.

**Recovery signal (`recovery-signal`).** Welcome `recovery_pending: true` means the previous server run ended without an orderly shutdown (its run marker was still present) and no client has shown or dismissed recovery yet. The candidates are the closed-session records, including those marked `Previous run`: list them with `closed_sessions`. `{"type":"dismiss_recovery","request":"r7"}` replies `done` and clears the signal for every client. The signal never becomes true during a run, so Welcome is its only carrier.

**Spawn with a shell (`spawn-shell`).** `spawn` takes an optional `shell`, an absolute path to an existing executable regular file (the same check `start` applies to the workspace shell). It runs instead of the server's shell, including after the arguments (`exec <shell> -i`), and becomes the pane's `PaneSession.shell`. Another value fails with an error naming the shell. The retry identity is the request, directory, arguments and shell.

```json
{"Stream":{"type":"spawn","request":"r8","cwd":"/home/me","argv":[],"shell":"/usr/bin/fish"}}
```

**Unused shell (`unused-shell`).** `PaneSession.unused_shell` is an observation: the pane's shell has no child process and no other process on its terminal, has seen no input for 200 ms with no input pending, and two separate idle observations agree. It is sampled every 500 ms (process table) and 100 ms (idle observations) while a stream connection has `session_events` enabled, and otherwise stays false. A lease does not hide it, because a stream TUI leases the panes it shows; clients combine it with `Update.leases` if they need to.

**Limits (`stream-limits`).** Welcome `limits {subscriptions, subscribed_cells}` advertises the subscription bounds described in [Pane subscriptions and updates](#pane-subscriptions-and-updates): 64 panes and 100,000 cells. Servers without the capability allow ten panes.

**Notification run (`notification-run`).** Notifications carry `origin {run, pid}`. Welcome `notification_run` is the current run: a notification refers to a live pane when `origin.run` equals it, `origin.pid` equals the pane's `pid` and `pane` equals the pane ID. Older notifications, or those from a previous run, no longer refer to a live pane even if a pane has the same ID. This lets a client jump to a notification's pane and badge panes without guessing the run from the notification list.

**Spawned by (`spawned-by`).** `PaneSession.spawned_by` is the Welcome `client` ID of the stream connection whose `spawn` or `restore` created the pane, so a client can recognize the panes it created (for example for its layout, or its scratch terminal). Client IDs are per connection: a client that reconnects gets a new ID, so it must remember its previous IDs for the epoch to recognize its earlier panes. It is absent for panes the server created itself (the replacement shell after closing the last pane). It is not persisted: a new epoch starts without it, and a client ID is not an identity across connections.

### Step 3: generic server (0.3)

KelpServer 0.3.0 (ADR 0002 step 3) removes the composed protocol 58 and the server-side UI. The stream stays at version 1 and every capability above keeps its meaning; the only stream addition is `diagnostics`.

```json
{"Stream":{"type":"diagnostics","request":"r9"}}
{"Stream":{"type":"diagnostics","request":"r9","report":{"version":"0.3.0-beta.1","protocol":1,"cols":120,"rows":40,"panes":2,"clients":1,"uptime_seconds":61,"pty_bytes":40960}}}
```

**Diagnostics (`diagnostics`).** `diagnostics {request}` replies with content-free counters: server version, stream version, base size, pane and connection counts, uptime and PTY bytes read. No terminal cells, prompts, paths or session IDs. The same report answers the one-shot `"Diagnostics"` record on a connection without Hello, for administration tools that do not open a stream session.

**Startup and base size.** A server starts with zero panes. `--size` sets the base size of panes without a lease (2–400 columns, 1–160 rows). Nothing reads or writes client UI files: tab layouts, templates, sidebar preferences and UI settings in `workspace.json` belong to the client. `kelpserver start` stores only `shell` and `cwd` (and, when given, the [upload](#uploads-uploads) options `upload_dir` and `upload_retention_days`) in `workspace.json` and preserves its other keys.

**Administration.** `kelpserver start`, `status`, `stop` and `ctl list|diagnostics|shutdown` use the stream (`status` and `ctl list` read the first update; `stop` and `ctl shutdown` send `shutdown`). `stop` falls back to the legacy shutdown command only for a running server that lacks `server-lifecycle`. The upgrade identity file `server.identity.json` records the stream version in `protocol` for 0.3 servers (58 means a composed-protocol server), and `kelpserver status` prints it as "stream protocol 1" or "local protocol 58". A running 0.2 server (protocol 58) also speaks stream version 1, so a 0.3 stream client keeps using it within its advertised capabilities; an old composed client connecting to a 0.3 server gets the `Error` described under [Framing and negotiation](#framing-and-negotiation). `upgrade::attach_matching` rejects a `kelpserver` whose identity names any other protocol with an error naming both versions.

### Uploads (`uploads`)

A client that lets the user drop files onto a terminal sends them to the server with these requests; the files land on the host where the server and its shells run (the user's machine for a local server, the workspace pod for a containerized one, which needs no SSH daemon). The client owns the drop UI and, once the batch is done, pastes the shell-quoted paths into the pane itself (for example with `paste`). Uploads are not tied to a pane and need no lease. Decision 151a, [ADR 0002](adr/0002-generic-session-server.md#uploads-capability-2026-11-21-decision-151a).

**Authorization.** Any stream connection may upload. This is the same trust as typing into a shell, which any stream connection may do after claiming a pane: a client able to send input can already write any file the server's user can write. Clients that relay other users' input (a multi-user web front end) must apply their own policy before relaying upload requests.

**Limits.** Welcome carries `upload_limits`; read them rather than hardcoding them:

```json
{"Stream":{"type":"welcome",…,"capabilities":[…,"uploads"],"upload_limits":{"max_files":64,"max_file_bytes":104857600,"max_batch_bytes":1073741824,"max_chunk_bytes":262144,"max_chunks_in_flight":8,"max_batches":4}}}
```

- `max_files`: files per batch (at least one).
- `max_file_bytes`: largest declared file (100 MiB); `max_batch_bytes`: sum of a batch's sizes (1 GiB).
- `max_chunk_bytes`: largest decoded chunk (256 KiB). Its base64 record is about 342 KiB, far below the 16 MiB record limit, so a chunk never delays pane updates for long.
- `max_chunks_in_flight`: the flow-control window (8), below.
- `max_batches`: unfinished batches per connection (4). The server also allows at most 16 unfinished batches in total.

**Begin.** Declare every file with its exact size; `dest` is optional.

```json
{"Stream":{"type":"upload_begin","request":"u1","files":[{"name":"photo.png","size":600000},{"name":"notes.txt","size":13}],"dest":"/home/me/project"}}
{"Stream":{"type":"upload_started","request":"u1","batch":1,"dir":"/home/me/.cache/kelpserver/uploads/1795300000-3f9c0a1b2c3d4e5f"}}
```

The server creates a new private (0700) directory for the batch, with a unique name (`<unix seconds>-<random hex>`), created and opened without following a symlink. Without `dest` it is created in the server's upload directory: `$XDG_CACHE_HOME/kelpserver/uploads` or `~/.cache/kelpserver/uploads` (created if needed and made 0700), or the directory set with `kelpserver server|start --upload-dir PATH` or `upload_dir` in the configuration file. `dest` must be an absolute path without `.` or `..` components naming an existing directory owned by the server's user; a symlink at its final component is refused. `batch` is a number unique within the server run, addressed only by the connection that began it. Refusals (limits, sizes, destination, too many batches, not enough free space for the declared total) are `error {request}` replies and create nothing.

File names are sanitized: only the base name is kept (after the last `/` or `\`); NUL, control and bidirectional formatting characters are removed; surrounding whitespace and leading dots are removed, so a name is never `.`, `..` or hidden; an empty result becomes `upload`; names are bounded to 200 bytes, keeping an extension of up to 32 bytes. Duplicates within a batch get `-2`, `-3`, … before the extension (`a.txt`, `a-2.txt`). The final names are those of the `upload_done` paths.

**Chunks and flow control.** Send each file's bytes as base64 (standard alphabet, padded) in order: files in request order (zero-byte files need no chunk), and each file's chunks at strictly sequential offsets starting at 0. Each chunk holds 1 to `max_chunk_bytes` decoded bytes.

```json
{"Stream":{"type":"upload_chunk","batch":1,"file_index":0,"offset":0,"data":"iVBORw0KGgo…"}}
{"Stream":{"type":"upload_progress","batch":1,"file_index":0,"received":262144}}
```

The server acknowledges every accepted chunk with exactly one `upload_progress`, in order, carrying the bytes of that file received so far. **Flow-control rule:** a client sends at most `max_chunks_in_flight` chunks that have not yet been acknowledged; after each `upload_progress` it may send one more. The server queues at most 32 reliable messages per connection, so a client that ignores the window and stops reading can be disconnected as a slow peer (which removes its unfinished batches). Chunks carry no `request` identifier.

A chunk that is out of order, at the wrong offset, not valid base64, empty or too large, beyond its file's declared size, or for a file index that does not exist fails the batch: the server removes the batch with its files and replies

```json
{"Stream":{"type":"upload_failed","batch":1,"message":"upload exceeds the declared size of file 0"}}
```

Chunks of that batch still in flight when it failed or was cancelled are dropped without a reply. A chunk for a batch this connection never had (or another connection's batch) gets `upload_failed` and changes nothing.

**Finish.** When every file has its declared size:

```json
{"Stream":{"type":"upload_finish","request":"u2","batch":1}}
{"Stream":{"type":"upload_done","request":"u2","batch":1,"paths":["/home/me/project/1795300000-3f9c0a1b2c3d4e5f/photo.png","/home/me/project/1795300000-3f9c0a1b2c3d4e5f/notes.txt"]}}
```

`paths` are absolute, in request order. Files are written to private temporary names inside the batch directory (`.kelpserver-upload-<index>.part`, mode 0600, created with `O_EXCL` and `O_NOFOLLOW`), synced when complete, and renamed to their final names on finish without replacing anything (`RENAME_NOREPLACE`), then the directory is synced. Published files keep mode 0600. A finish with a file still missing bytes is an `error {request}` ("upload is incomplete") and removes the batch; any other failure of a finish also removes it. Quote the paths for the shell before pasting them (for example `'…'` with `'` written as `'\''`).

**Cancel.**

```json
{"Stream":{"type":"upload_cancel","request":"u3","batch":1}}
{"Stream":{"type":"done","request":"u3"}}
```

removes the batch and its files. Cancelling a batch of this connection that already failed or was cancelled also replies `done`; an unknown batch is an `error {request}`.

**Connection scope.** A batch belongs to its connection. When the connection ends (disconnect, slow-peer disconnect, server shutdown), its unfinished batches are removed with their partial files. A completed batch stays on disk.

**Retention.** The server records each batch it creates in `uploads.json` in its state directory: the batch directory's path, device and inode, creation time and, once finished, completion time. At startup and then hourly it deletes completed batches older than the retention (default 7 days; `--upload-retention-days N` or `upload_retention_days` in the configuration file; `0` keeps them forever) and unfinished batches left by a previous run. Only registered directories whose device and inode still match, owned by the server's user, are deleted; a registered path that is now missing, a symlink or another directory is forgotten without deleting anything. Deletion works through directory descriptors and never follows symlinks (a symlink inside a batch is removed, not its target). Files that are not registered batches, including anything else in the upload directory or in a `dest`, are never deleted. `kelpserver start --upload-dir` and `--upload-retention-days` store `upload_dir` and `upload_retention_days` in `workspace.json`; they take effect when the server next starts.

### Parking and hidden panes

The server has no parking concept. A client that hides a pane unsubscribes from it and either keeps its lease, so its size stays fixed while hidden, or releases it, so the pane returns to the server's base size.
