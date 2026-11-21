//! Experimental UI-independent pane streams. SSH transports these framed messages unchanged.
//!
//! Send `Hello { version: 1 }` first, then `Subscribe` (replacement set, at most ten
//! unique current panes). Welcome's random server epoch scopes all pane IDs. A new
//! epoch invalidates every saved baseline/lease, even if numerical IDs repeat.
//! Every Update includes current bounded metadata, notifications and structured vitals.
//! A snapshot starts revision 1; patches advance exactly one revision and require
//! the exact delivered baseline. Geometry changes, explicit Resync and backpressure
//! produce replacement snapshots. Clients must never apply a patch to another epoch.
//!
//! Claim/Resize/Input/Paste/Mouse address one pane. Claim is exclusive, explicit
//! takeover may replace an owner, and release/disconnect restores the current base
//! workspace geometry. Leases have connection lifetime (no time-based expiry).
//! The ordinary TUI retains its independent workspace controller. Actual controller
//! input/focus reclaims a pane; passive resize and focus-out never steal a lease.
//!
//! At most 100,000 cells are subscribed per connection. A later resize exceeding
//! that aggregate budget disconnects that subscriber rather than accumulating state.
//! Server retains one in-flight packet and one replaceable aggregate update; excess
//! reliable responses disconnect a slow peer. Notifications are the latest 128
//! retained records, not an exactly-once event log; acknowledgement is explicit.
//!
//! The session API (capabilities `pane-lifecycle`, `closed-sessions`,
//! `pane-history`, `pane-scroll`, `notification-ack-all`, `host-status` and
//! `server-lifecycle`) is additive. Its events are sent only to a connection
//! that asked for them, either by opting in (`session_events`, `host_status`)
//! or by sending a correlated request, so v1 clients that reject unknown
//! events never receive one. Clients must check the Welcome capabilities
//! before sending a new request: an older server disconnects on an unknown
//! request type. Device/MCP capabilities remain deferred.
use super::protocol::{
    Cell, Cursor, InputModes, MouseEvent, Notification, PaneId, PaneInfo, PaneSearchMatch, Screen,
};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const VERSION: u16 = 1;
pub const MAX_SUBSCRIPTIONS: usize = 10;
/// Pane geometry is independent of the composed TUI's minimum window size.
pub fn validate_size(cols: u16, rows: u16) -> Result<()> {
    if !(2..=400).contains(&cols) || !(1..=160).contains(&rows) {
        return Err("pane size must be 2–400 columns and 1–160 rows".into());
    }
    Ok(())
}
/// Bound both retained screen baselines and aggregate update serialization.
pub const MAX_SUBSCRIBED_CELLS: usize = 100_000;
pub const CAPABILITIES: &[&str] = &[
    "pane-snapshots",
    "pane-patches",
    "pane-leases",
    "metadata",
    "notifications",
    "vitals",
    "hyperlinks",
    "program-status",
    "controller-theme",
    "clipboard-write",
    "pane-spawn",
    "pane-lifecycle",
    "closed-sessions",
    "pane-history",
    "pane-scroll",
    "notification-ack-all",
    "host-status",
    "server-lifecycle",
];
/// Aggregate context budget of one multi-pane `search` reply.
pub const MAX_SEARCH_CONTEXT: usize = 4 * 1024 * 1024;
/// Longest `history` page.
pub const MAX_HISTORY_LINES: usize = 200;

/// Client-chosen correlation identifiers: 1–128 ASCII letters, digits, `-` or `_`.
pub fn valid_request(request: &str) -> bool {
    !request.is_empty()
        && request.len() <= 128
        && request
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Hello {
        version: u16,
    },
    Subscribe {
        panes: Vec<PaneId>,
    },
    /// Opt in to application clipboard writes for panes owned by this connection.
    Clipboard {
        enabled: bool,
    },
    Claim {
        pane: PaneId,
        takeover: bool,
    },
    Release {
        pane: PaneId,
    },
    Resize {
        pane: PaneId,
        cols: u16,
        rows: u16,
    },
    Theme {
        pane: PaneId,
        foreground: [u8; 3],
        background: [u8; 3],
    },
    Input {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    Paste {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    Mouse {
        pane: PaneId,
        event: MouseEvent,
    },
    Resync {
        pane: PaneId,
    },
    Acknowledge {
        notification: u64,
    },
    /// `notification-ack-all`: mark every retained notification read.
    AcknowledgeAll,
    /// `pane-lifecycle`: receive lifecycle events and per-pane session data.
    SessionEvents {
        enabled: bool,
    },
    /// `pane-lifecycle`: start a pane. Without `cwd`, the server's launch
    /// directory is used; an empty `argv` opens an interactive shell. A retry
    /// with the same `request`, directory and arguments returns the same pane.
    Spawn {
        request: String,
        #[serde(default)]
        cwd: Option<PathBuf>,
        #[serde(default)]
        argv: Vec<String>,
    },
    /// `pane-lifecycle`: terminate a pane's process group and remove the pane.
    /// Refused while another connection holds the pane's lease.
    Close {
        request: String,
        pane: PaneId,
    },
    /// `closed-sessions`: list restorable records of closed panes.
    ClosedSessions {
        request: String,
    },
    /// `closed-sessions`: reopen a listed record. `rerun` repeats a plain
    /// program; agent records always resume their conversation if known.
    Restore {
        request: String,
        record: String,
        #[serde(default)]
        rerun: bool,
    },
    /// `pane-history`: plain-text rows; negative lines are scrollback.
    History {
        request: String,
        pane: PaneId,
        start: i32,
        limit: usize,
    },
    /// `pane-history`: literal case-insensitive search. An empty pane list
    /// searches every pane in creation order.
    Search {
        request: String,
        #[serde(default)]
        panes: Vec<PaneId>,
        query: String,
        limit: usize,
    },
    /// `pane-scroll`: move the leased pane's viewport through its scrollback.
    Scroll {
        pane: PaneId,
        lines: i32,
    },
    /// `pane-scroll`: show a primary-history line (as returned by `history`
    /// or a search match's `first_line + start.y`) in the leased pane.
    Reveal {
        pane: PaneId,
        line: i32,
    },
    /// `host-status`: include the host process/disk snapshot in updates.
    HostStatus {
        enabled: bool,
    },
    /// `server-lifecycle`: stop the server and every pane process.
    Shutdown {
        request: String,
    },
}
impl Request {
    /// The correlation identifier echoed by this request's reply and errors.
    pub fn correlation(&self) -> Option<&str> {
        match self {
            Self::Spawn { request, .. }
            | Self::Close { request, .. }
            | Self::ClosedSessions { request }
            | Self::Restore { request, .. }
            | Self::History { request, .. }
            | Self::Search { request, .. }
            | Self::Shutdown { request } => Some(request),
            _ => None,
        }
    }
}

/// Session facts about a live pane, independent of any client's presentation.
/// Pane IDs increase monotonically and are never reused within one epoch, so
/// they also give creation order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSession {
    pub pane: PaneId,
    /// Unix seconds.
    pub created_at: u64,
    /// The directory the shell was started in; `PaneInfo::cwd` is the current one.
    pub launch_cwd: String,
    pub shell: String,
    /// Arguments given to `spawn`; empty for an interactive shell or another origin.
    pub argv: Vec<String>,
    /// The command line typed into the shell at startup, if any.
    pub startup: Option<String>,
    /// The latest OSC title set by the application, unmodified by naming rules.
    pub terminal_title: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// The pane's process exited.
    Exited,
    /// A client closed the pane.
    Closed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// An explicit shutdown request.
    Shutdown,
    /// The last pane exited and no other pane keeps the server alive.
    LastPaneExited,
}
/// A restorable record of a closed pane. `key` identifies it for `restore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedSession {
    #[serde(flatten)]
    pub record: super::history::Entry,
    /// The agent resume command `restore` runs, if the record is an agent.
    pub resume: Option<String>,
    /// The program `restore` with `rerun` runs, if the record is a program.
    pub rerun: Option<String>,
}
impl From<super::history::Entry> for ClosedSession {
    fn from(record: super::history::Entry) -> Self {
        Self {
            resume: record.command(),
            rerun: record.previous_command(),
            record,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneSearchResult {
    pub pane: PaneId,
    pub hits: Vec<PaneSearchMatch>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSnapshot {
    pub pane: PaneId,
    pub revision: u64,
    pub screen: Screen,
}

/// Check a full pane screen before allocating a renderer's backing surface.
pub fn validate(snapshot: &PaneSnapshot) -> Result<()> {
    let screen = &snapshot.screen;
    if snapshot.pane == 0
        || snapshot.revision == 0
        || !(1..=400).contains(&screen.cols)
        || !(1..=160).contains(&screen.rows)
        || screen.cells.len() != usize::from(screen.cols) * usize::from(screen.rows)
        || screen.cursor.is_some_and(|cursor| {
            cursor.position.x >= screen.cols || cursor.position.y >= screen.rows
        })
        || screen.cells.iter().enumerate().any(|(index, cell)| {
            !super::delta::cell_valid(cell)
                || cell.width == 2
                    && index % usize::from(screen.cols) + 1 == usize::from(screen.cols)
        })
    {
        return Err("invalid pane snapshot".into());
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanePatch {
    pub pane: PaneId,
    pub base: u64,
    pub revision: u64,
    pub cursor: Option<Cursor>,
    pub modes: InputModes,
    pub changes: Vec<(usize, Cell)>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub pane: PaneId,
    pub owner: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    pub epoch: String,
    pub subscribed: Vec<PaneId>,
    pub snapshots: Vec<PaneSnapshot>,
    pub patches: Vec<PanePatch>,
    pub panes: Vec<PaneInfo>,
    pub leases: Vec<Lease>,
    pub notifications: Vec<Notification>,
    pub vitals: super::vitals::Vitals,
    /// Only for connections that enabled `session_events`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<PaneSession>,
    /// Only for connections that enabled `host_status`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<Box<super::status::Snapshot>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Welcome {
        version: u16,
        client: u64,
        epoch: String,
        capabilities: Vec<String>,
        /// The server's package version (`server-lifecycle`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server_version: Option<String>,
    },
    Update(Update),
    Lease {
        pane: PaneId,
        owner: Option<u64>,
    },
    Clipboard {
        pane: PaneId,
        text: String,
    },
    Error {
        message: String,
        /// The failed request's correlation identifier, when it had one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<String>,
    },
    /// Opt-in lifecycle events. A pending update built before the event is
    /// discarded, so the next update for that connection follows the event.
    PaneCreated {
        session: PaneSession,
    },
    PaneExited {
        pane: PaneId,
        at: u64,
        code: Option<i32>,
        signal: Option<i32>,
    },
    PaneClosed {
        pane: PaneId,
        at: u64,
        reason: CloseReason,
        /// The closed-session record created for this pane, if any.
        record: Option<String>,
    },
    ServerStopping {
        reason: StopReason,
    },
    /// Replies to correlated requests; sent only to the requesting connection.
    Spawned {
        request: String,
        pane: PaneId,
    },
    Done {
        request: String,
    },
    ClosedSessions {
        request: String,
        records: Vec<ClosedSession>,
    },
    Restored {
        request: String,
        record: String,
        pane: PaneId,
        /// The conversation was already open in this pane; nothing was started.
        existing: bool,
    },
    History {
        request: String,
        pane: PaneId,
        lines: Vec<(i32, String)>,
    },
    Search {
        request: String,
        results: Vec<PaneSearchResult>,
        truncated: bool,
    },
}

/// Apply only to the exact baseline. A full snapshot starts a fresh revision chain.
pub fn apply(snapshot: &PaneSnapshot, patch: &PanePatch) -> Result<PaneSnapshot> {
    validate(snapshot)?;
    if patch.pane != snapshot.pane
        || patch.base != snapshot.revision
        || patch.base.checked_add(1) != Some(patch.revision)
        || patch.changes.len() > snapshot.screen.cells.len()
    {
        return Err("invalid pane patch baseline".into());
    }
    if patch.cursor.is_some_and(|cursor| {
        cursor.position.x >= snapshot.screen.cols || cursor.position.y >= snapshot.screen.rows
    }) {
        return Err("invalid pane patch cursor".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for (index, cell) in &patch.changes {
        if *index >= snapshot.screen.cells.len()
            || !seen.insert(*index)
            || !super::delta::cell_valid(cell)
        {
            return Err("invalid pane patch cell".into());
        }
    }
    let mut next = snapshot.clone();
    for (index, cell) in &patch.changes {
        next.screen.cells[*index] = cell.clone();
    }
    next.revision = patch.revision;
    next.screen.cursor = patch.cursor;
    next.screen.modes = patch.modes;
    validate(&next)?;
    Ok(next)
}

pub fn diff(old: &PaneSnapshot, screen: &Screen) -> Option<PanePatch> {
    if old.screen.cols != screen.cols
        || old.screen.rows != screen.rows
        || old.screen.cells.len() != screen.cells.len()
    {
        return None;
    }
    let changes: Vec<_> = old
        .screen
        .cells
        .iter()
        .zip(&screen.cells)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(index, (_, cell))| (index, cell.clone()))
        .collect();
    // A sparse patch is useful; dense changes are cheaper and simpler as snapshots.
    if changes.len() > screen.cells.len() / 4 {
        return None;
    }
    Some(PanePatch {
        pane: old.pane,
        base: old.revision,
        revision: old.revision.checked_add(1)?,
        cursor: screen.cursor,
        modes: screen.modes,
        changes,
    })
}
