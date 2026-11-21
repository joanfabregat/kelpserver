# Native client foundation

Kelpmux owns long-running PTYs and Alacritty terminal interpretation. Clients own presentation. Native mode has no tmux or WezTerm runtime dependency. The composed ANSI frontend is protocol v58; the separate experimental pane stream remains v1. Pane metadata adds a default-false `parked` field. Parking leaves PTY contents, status and stream subscriptions alive; restoring a pane returns it to the composed UI. A matching frontend/server pair is required because mouse routing shares their UI geometry and dialog types. Pane streaming shipped in beta.3; the updated composed status geometry requires a current dev build. A graphical desktop/mobile client and MCP/device tools are not implemented.

The broader proposed contract is [ADR 0001](adr/0001-native-client-protocol-and-device-capabilities.md). It covers pane lifecycle, independent device sizes, live agent status, notifications, remote Vitals and a future MCP/device request channel. The ADR deliberately distinguishes those proposals from the implemented requests below.

KelpMux's terminals-only browser client and its `kelpmux web-bridge` are described in [KelpMux docs/web.md](https://github.com/joanfabregat/kelpmux/blob/main/docs/web.md); KelpHub keeps its own browser application and authenticated management adapter. Shared portable types and terminal client state live in [KelpClient](https://github.com/joanfabregat/kelpmux/blob/main/crates/client/README.md) in KelpMux. Desktop/mobile implementation choices remain deferred. The composed-screen TUI compatibility implementation is retained; see the [KelpServer README](../../../README.md) and KelpMux [MIGRATION.md](https://github.com/joanfabregat/kelpmux/blob/main/MIGRATION.md) for current component ownership.

Screen input modes include an additive, default-zero `keyboard_flags` bitfield using the five Kitty keyboard enhancement flags. The composed frontend negotiates those flags with the outer terminal while retaining key disambiguation for its own controls. Native clients must preserve modified Enter events and distinguish key release from key press. Pane snapshots preserve default foreground/background colors so the attached terminal supplies its theme. Explicit application colors and OSC palette changes remain pane-local.

The composed frontend queries OSC 10/11 and sends bounded `ThemeColor { background: bool, rgb: [u8; 3] }` reports after attachment. These are metadata, never PTY input, and work independently of active dialogs. The server retains each attached client's reports and applies the TUI controller's colors as emulator query fallbacks except where a stream lease supplies its own theme. Application OSC colors take priority, while default snapshot cells remain `Default`. An unsupported or silent terminal retains documented emulator fallbacks. Theme reports do not force an existing application to repeat its startup probes or replace explicit application RGB colors.

## SSH command channel

`kelpserver bridge --workspace NAME` relays bounded native-stream records between standard input/output and the existing owned private Unix socket. It does not start a workspace or attach an ANSI frontend. A future native client opens it through one authenticated SSH command channel without an outer PTY:

```sh
ssh -T HOST kelpserver bridge --workspace NAME
```

Use the installed binary's absolute path if it is not in the remote noninteractive PATH. SSH supplies authentication, encryption and transport; there is no additional public port. Shell banners must not appear on stdout. Bridge diagnostics go to stderr, and a TTY endpoint is rejected before terminal-mode changes. EOF detaches that connection and releases its leases, preserving workspace processes. Unacknowledged requests are not automatically replayed. The bridge does not provide mobile roaming or background push delivery.

## Framing and negotiation

Every record has a four-byte unsigned big-endian payload length followed by UTF-8 JSON, with a 16 MiB maximum payload. JSON examples below are payloads, not newline-delimited wire records. Use the same framing over the local Unix socket or SSH bridge.

```json
{"Stream":{"type":"hello","version":1}}
```

The server responds with `Stream` / `welcome`, containing stream version, connection/client ID, a random server epoch, and supported capabilities. Send Hello exactly once before other stream requests. Legacy TUI/management messages are rejected on a negotiated stream connection and by the bridge. Unsupported versions fail clearly. Current capabilities are `pane-snapshots`, `pane-patches`, `pane-leases`, `metadata`, `notifications`, `vitals`, `hyperlinks`, `program-status`, `controller-theme`, `clipboard-write` and `pane-spawn`, plus the [session API](#session-api) capabilities `pane-lifecycle`, `closed-sessions`, `pane-history`, `pane-scroll`, `notification-ack-all`, `host-status` and `server-lifecycle`; do not infer support for undeclared future capabilities. A server with `server-lifecycle` also puts its package version in Welcome `server_version`.

Clients recognizing `clipboard-write` may opt in with `{"Stream":{"type":"clipboard","enabled":true}}`. Application OSC 52 writes then produce `clipboard` events containing `pane` and UTF-8 `text` (at most 64 KiB), delivered only to that pane's current lease owner. Observers and clients that have not opted in receive no clipboard data. Writes from panes without a stream owner retain their existing TUI-controller route. Clipboard events are ephemeral: never replay them in snapshots, history or reconnect state. Clipboard reads remain disabled. Browser clients must provide an explicit copy fallback when platform user-activation rules prevent an asynchronous write.

The `pane-spawn` capability advertises structured local administration on a separate, same-UID socket connection: `SpawnPane { request, cwd, argv }` returns `SpawnedPane { request, pane }`. The initial absolute working directory is set before the interactive shell starts; argument strings are quoted as data, not accepted as a shell command. An empty argument list opens a shell. This does not transfer input ownership or change the current active pane. Retry identities are checked against the directory and arguments in a bounded 512-entry cache for the current server epoch; durable application-level request records belong to the management caller. Attached observers cannot invoke this operation. This primitive introduces no project model or HTTP management endpoint into the terminal server.

Pane IDs are scoped to the Welcome epoch. Within an epoch they increase monotonically and are never reused, so they also give creation order. A new epoch invalidates previous screen baselines and ownership. Clients must negotiate anew, obtain snapshots, and deliberately rebuild desired control state rather than replaying old mutations. Client UI state keyed by pane identity (epoch plus pane ID) does not survive a new epoch. The v1 operations carry no correlation ID or expected epoch; the session API requests carry a correlation ID (below). An expected epoch on every mutation remains part of public-contract maturation in the ADR.

## Pane subscriptions and updates

```json
{"Stream":{"type":"subscribe","panes":[1,2]}}
```

Subscribe replaces the complete subscription set. Updates contain an authoritative `subscribed` list, full metadata for current panes, leases, notifications, Vitals, and snapshots or patches for subscribed terminal screens. Metadata-only subscriptions use an empty list. The reference client prunes cached screens according to each received update's subscription list, safely processing older in-flight updates before a replacement set arrives.

A full pane snapshot establishes revision 1. A patch names the pane, base revision and resulting revision; it applies only to that exact pane baseline in the current server epoch. Geometry changes, explicit `resync` and backpressure can produce replacement snapshots. Snapshots contain cells with colors/attributes/widths, cursor and input modes; they do not contain native headers, tabs or footer cells. Alacritty remains the only emulator answering application terminal queries.

There are at most ten subscribed panes and 100,000 aggregate subscribed screen cells per connection. Oversized subscriptions are rejected. A later size change exceeding an observer's aggregate budget disconnects that observer instead of retaining unbounded data. The server retains one in-flight packet and one replaceable aggregate update. The bridge has separate bounded queues in each direction and yields between decode batches. A permanently slow client can be disconnected and resynchronize on reconnect.

## Independent ownership and size

`claim {pane,takeover}` explicitly acquires one pane; `release {pane}` relinquishes it. `resize {pane,cols,rows}`, `input {pane,bytes}`, `paste {pane,bytes}` and `mouse {pane,event}` require the caller's lease. Input/paste are bounded to 64 KiB. Paste honors the pane's bracketed-paste mode; native mouse events currently support SGR encoding. An observer cannot write or resize a pane.

The optional `controller-theme` capability adds `theme {pane,foreground:[r,g,b],background:[r,g,b]}` with byte-valued RGB channels. It requires the caller's pane lease and sets that pane's OSC 10/11 query fallbacks. The browser reports its theme after claiming and when its appearance changes. Takeover or release drops the old lease theme, restoring the next controller's colors. Do not send this request to retained servers that omit the capability.

Client A can own pane 1 at 240 × 70 cells while client B owns pane 2 at 40 × 12 cells. Native pane sizes permit 2–400 columns and 1–160 rows, independently of the ANSI frontend's 40 × 10 minimum window. Each pane still has exactly one underlying PTY size. Observers of that same pane clip/scale locally and cannot impose a competing size.

Leases last until release, takeover or disconnect; release/disconnect restores the current base workspace geometry. Stream clients issue explicit Claim messages, which a UI may send automatically on focus or input. The active attached TUI controller reclaims a pane when the user types, pastes, focuses it or interacts with its mouse; background resize, mouse movement and focus-out never steal a lease. The browser retains its last geometry and claims on blur or document hiding; actual foreground input or another explicit claim can still take over. Release is idempotent for an already-free pane, including a blur racing a TUI reclaim. Safe grid shrink excludes leased panes. `kelpmux ctl --workspace NAME reclaim ID` remains an explicit administration shortcut. Attached TUI observers cannot bypass control using management messages. Workspace administration and explicit shutdown remain separate same-user local operations; this is not isolation against hostile processes running as that user.

## State, notifications and Vitals

The `program-status` capability adds `terminal` observation evidence from pane-local OSC 7501 reports. Current dev clients must recognize this status-source value; the experimental stream remains v1 and is not a frozen compatibility contract. The composed frontend/server pair uses v58. Terminal activity takes precedence over metadata polling without implying that a conversation ID is known.

Pane metadata carries identity, agent kind, session ID if known, status, activity times and status observation evidence. Evidence distinguishes process, lifecycle hook, live record, agent-server and transcript observations. `checked_at` records a check; optional `event_at` records an actual known event time. Reading an old transcript again does not invent a new lifecycle event. Unknown or stale evidence remains visible in the overview, and an idle observation does not become a completed turn automatically.

Updates include the latest 128 retained notifications and explicit `acknowledge {notification}`. This is a bounded snapshot of notification history, not exactly-once event delivery or a question/answer RPC. Existing seven-day retention and persisted unread state remain. No native device notification or push service is invoked.

Vitals are typed values from the existing in-process sampler, independent of footer styling: CPU, memory, swap and disk percentages, plus server-local date/time strings. Missing values stay unavailable. Sample timestamps, byte totals, mount labels, timezone-aware conversion and richer metrics are proposed additions; clients must not treat the current clock strings as timezone-independent timestamps.

## Reference client and inspection

The Rust `native::reference::State` validates welcome/epoch, membership, screen/cell bounds, leases and patch baselines before committing changes. Failed updates leave previous baselines intact. `stream-inspect` is a read-only one-shot reference client with an absolute operation deadline:

```sh
kelpserver stream-inspect --workspace NAME
kelpserver stream-inspect --workspace NAME 1 2
```

With no pane IDs it returns metadata only; explicit IDs add those panes' terminal cells to JSON stdout. It does not claim, resize, become the TUI controller, start a server or persist terminal contents. Automated tests use actual bridge subprocess pipes, disposable PTYs and typed reference state; they do not open a real SSH login or access a native device.

The older `InspectPane` and bounded `History {pane,start,limit}` requests remain local inspection seams. `SearchPane {pane,query,limit}` returns `PaneSearch {pane,hits,truncated}` with case-insensitive literal occurrences; queries must contain 1–256 bytes without control characters and the limit is at most 64 matches. Each match carries its source buffer, immutable context rows, and inclusive cell coordinates relative to that context. This prevents ongoing output or alternate-screen transitions from moving the target between search and reveal. Context text has a combined 4 MiB budget; `truncated` indicates that a match or context budget stopped the search. Search does not change another client's viewport or input ownership. Ordinary history pages still use relative line indices and plain text; stable, styled native-history pagination is not implemented. The same history and search are available on the stream itself through `pane-history` (below). Pane renaming is client UI. Device capability requests, attachments, MCP, native apps and final public-protocol compatibility guarantees remain future work described in ADR 0001.

### Hyperlink cells

The server's `hyperlinks` capability indicates support for an optional `hyperlink` URI string per cell. Missing fields mean no target; explicit targets and recognized plain HTTP(S) URLs are carried independently of visible text. Targets are bounded to 2,048 bytes, require a URI scheme and reject whitespace and control characters. Deltas include target-only changes. Native clients open links through their own user interaction rather than requesting browser execution on the server. Older cell encodings remain decodable.

## Session API

The session API is step 1 of [ADR 0002](adr/0002-generic-session-server.md): the session-level operations a full TUI needs to run as a stream client with local rendering and local UI state. The classification of every TUI capability, and what remains, is in [session-api.md](session-api.md). The composed protocol 58 is unchanged.

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

Rules that keep older peers working:

- A connection receives an event of a new type only after it opts in (`session_events`, `host_status`) or as the reply to a request it sent. Existing clients that reject unknown event types (KelpClient, KelpHub's bridge) therefore never see one. `Update.sessions` and `Update.host` are omitted unless enabled.
- New fields on existing messages (`Welcome.server_version`, `Error.request`, `Update.sessions`, `Update.host`) are optional; decoders that ignore unknown fields are unaffected.
- A server without a capability disconnects a connection that sends an unknown request type. Check the Welcome capabilities before sending any request listed above.
- The SSH bridge relays every stream request after Hello, so the session API works through `kelpserver bridge` unchanged.

In the Rust API, `Event::Welcome`, `Event::Error` and `Update` gained fields and `Request`/`Event` gained variants, so exhaustive matches and struct literals in consumers need updating when they move their pin.

### Correlation and errors

Requests that have a reply carry a client-chosen `request` string of 1–128 ASCII letters, digits, `-` or `_`. The reply echoes it, and so does `{"type":"error","message":…,"request":…}` when such a request fails. A malformed identifier fails with an error without `request`. Errors of v1 requests still have no `request`. Fire-and-forget requests (`scroll`, `reveal`, `acknowledge_all`, the opt-ins) have no identifier and no reply, like `resize` or `acknowledge`.

### Pane lifecycle (`pane-lifecycle`)

```json
{"Stream":{"type":"session_events","enabled":true}}
```

After the opt-in, each update carries `sessions`, one `PaneSession` per live pane in ID order: `pane`, `created_at` (Unix seconds), `launch_cwd`, `shell`, `argv` (the `spawn` arguments, otherwise empty), `startup` (the command line typed into the shell at startup, if any) and `terminal_title` (the application's latest OSC title, unmodified). `Update.panes` keeps carrying the observations: current cwd, process, agent kind and session, status and evidence, activity times and shell command.

Lifecycle events follow, for every pane whatever created or closed it, including the composed TUI:

- `{"type":"pane_created","session":{…}}`;
- `{"type":"pane_exited","pane":3,"at":…,"code":0,"signal":null}`, when the process exits, always followed by `pane_closed`;
- `{"type":"pane_closed","pane":3,"at":…,"reason":"exited"|"closed","record":"<key>"|null}`, where `record` names the closed-session record created for the pane, if any.

Events are reliable messages. A replaceable update that was built before an event and is still pending is discarded with that connection's screen baselines, so the next update (a full one) already reflects the event. Updates that were already on the wire may precede it.

```json
{"Stream":{"type":"spawn","request":"r1","cwd":"/home/me/src","argv":["htop"]}}
{"Stream":{"type":"close","request":"r2","pane":3}}
```

`spawn` takes the same parameters as the `pane-spawn` administration request; `cwd` defaults to the server's launch directory and an empty `argv` opens an interactive shell. Arguments are quoted as data and run before `exec $SHELL -i`. The server removes agent and multiplexer session markers from the environment; clients cannot pass variables. A retry with the same `request`, directory and arguments returns the same pane (bounded 512-entry cache per epoch). The reply is `spawned {request, pane}`, after `pane_created`.

`close` terminates the pane's process group and removes the pane; the reply is `done {request}`, after `pane_closed`. It is refused while another connection holds the pane's lease. Asking the user for confirmation is client UI. For compatibility with the composed UI, closing the last pane first creates a replacement shell, and the last pane's own exit stops the server.

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

`history` returns `history {request, pane, lines}` with the same plain-text rows and relative line indices as the local `History` request (at most 200). `search` returns `search {request, results, truncated}`; each result is `{pane, hits, truncated}` with the same immutable-context matches as `SearchPane`. An empty `panes` list searches every pane in ID (creation) order. One reply holds at most `limit` (1–64) matches and 4 MiB of context; `truncated` reports that a limit stopped the search. Neither request needs a lease or changes any viewport.

`scroll {pane, lines}` and `reveal {pane, line}` require the pane's lease and move its emulator viewport, which every subscriber of that pane sees in its snapshots. `reveal` takes a primary-history line (a `history` index, or a match's `first_line + start.y`) and fails while the application uses the alternate screen. They exist until styled history lets a client render scrollback itself.

### Notifications, host status and server lifecycle

`acknowledge_all` marks every retained notification read (`notification-ack-all`).

`{"type":"host_status","enabled":true}` adds `host` to that connection's updates (`host-status`): per-CPU usage, memory and swap bytes, load, uptime, task counts, network and disk I/O rates, mounted disks and the process table, sampled on demand about once a second while at least one connection wants it. `ready` is false until the first sample. Views, sorting and formatting are client UI.

`shutdown {request}` replies `done` and stops the server and its panes (`server-lifecycle`). A connection that enabled `session_events` receives `server_stopping {reason: "shutdown"|"last_pane_exited"}` before its socket closes; the server gives queued messages up to 200 ms. There is no upgrade event: a live server is never upgraded in place. A client detects a different server by its epoch and `server_version`, and its features by the capabilities.

### Parking and hidden panes

The server has no parking concept. A client that hides a pane unsubscribes from it and either keeps its lease, so its size stays fixed while hidden, or releases it, so the pane returns to the server's base geometry.
