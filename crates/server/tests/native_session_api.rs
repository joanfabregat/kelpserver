//! Session API on the stream protocol (KelpServer ADR 0002, step 1).
use kelpserver::native::{
    protocol::*,
    server::{ServerConfig, serve},
    stream::{self, CloseReason, Event, PaneSnapshot, Request, StopReason, Update},
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

struct Daemon {
    root: PathBuf,
    thread: Option<thread::JoinHandle<kelpserver::Result<()>>>,
}
impl Daemon {
    fn start() -> Self {
        Self::start_with(|_| {})
    }
    /// `prepare` may seed the private state directory before the server starts.
    fn start_with(prepare: impl FnOnce(&std::path::Path)) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "kelpserver-session-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let state = root.join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        prepare(&state);
        let config = ServerConfig {
            initial_weights: Default::default(),
            template: None,
            socket: root.join("control.sock"),
            state_dir: state,
            shell: "/bin/sh".into(),
            cwd: root.clone(),
            cols: 100,
            rows: 30,
            ui: Default::default(),
            initial_grid: (2, 1),
            initial_tabs: None,
        };
        let handle = thread::spawn(move || serve(config));
        let daemon = Self {
            root,
            thread: Some(handle),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while UnixStream::connect(daemon.root.join("control.sock")).is_err() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        daemon
    }
    fn socket(&self) -> UnixStream {
        let s = UnixStream::connect(self.root.join("control.sock")).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        s
    }
    fn peer(&self) -> Peer {
        let mut p = Peer {
            socket: self.socket(),
            id: 0,
            epoch: String::new(),
            capabilities: Vec::new(),
            server_version: None,
            update: None,
            screens: BTreeMap::new(),
        };
        p.send(Request::Hello {
            version: stream::VERSION,
        });
        loop {
            if let Event::Welcome {
                client,
                epoch,
                capabilities,
                server_version,
                ..
            } = p.next()
            {
                p.id = client;
                p.epoch = epoch;
                p.capabilities = capabilities;
                p.server_version = server_version;
                break;
            }
        }
        p
    }
    fn inspect(&self, pane: u64) -> Screen {
        let mut s = self.socket();
        write_message(&mut s, &ClientMessage::InspectPane { pane }).unwrap();
        match read_message(&mut s).unwrap().unwrap() {
            ServerMessage::PaneState { screen, .. } => screen,
            other => panic!("{other:?}"),
        }
    }
    fn wait_stopped(&mut self) {
        let thread = self.thread.take().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !thread.is_finished() {
            assert!(Instant::now() < deadline, "server did not stop");
            thread::sleep(Duration::from_millis(20));
        }
        assert!(thread.join().unwrap().is_ok());
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        if let Ok(mut s) = UnixStream::connect(self.root.join("control.sock")) {
            let _ = write_message(&mut s, &ClientMessage::Command(Action::Shutdown));
        }
        if let Some(t) = self.thread.take() {
            assert!(t.join().unwrap().is_ok());
        }
        fs::remove_dir_all(&self.root).unwrap();
    }
}

struct Peer {
    socket: UnixStream,
    id: u64,
    epoch: String,
    capabilities: Vec<String>,
    server_version: Option<String>,
    update: Option<Update>,
    screens: BTreeMap<u64, PaneSnapshot>,
}
impl Peer {
    fn send(&mut self, r: Request) {
        write_message(&mut self.socket, &ClientMessage::Stream(r)).unwrap();
    }
    fn next(&mut self) -> Event {
        let event = match read_message(&mut self.socket).unwrap().unwrap() {
            ServerMessage::Stream(e) => e,
            other => panic!("non-stream message {other:?}"),
        };
        if let Event::Update(u) = &event {
            assert_eq!(u.epoch, self.epoch);
            for snapshot in &u.snapshots {
                self.screens.insert(snapshot.pane, snapshot.clone());
            }
            for patch in &u.patches {
                let next = stream::apply(&self.screens[&patch.pane], patch).unwrap();
                self.screens.insert(patch.pane, next);
            }
            self.update = Some(u.clone());
        }
        event
    }
    /// The first event matching `pick`, failing on any error event.
    fn until<T>(&mut self, mut pick: impl FnMut(&Event) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "timed out waiting for an event");
            let event = self.next();
            if let Event::Error { message, .. } = &event {
                panic!("{message}");
            }
            if let Some(value) = pick(&event) {
                return value;
            }
        }
    }
    /// The first update satisfying `predicate`.
    fn update_until(&mut self, predicate: impl Fn(&Update) -> bool) -> Update {
        if let Some(update) = self.update.clone().filter(|u| predicate(u)) {
            return update;
        }
        self.until(|event| match event {
            Event::Update(u) if predicate(u) => Some(u.clone()),
            _ => None,
        })
    }
    fn error(&mut self, r: Request) -> (String, Option<String>) {
        self.send(r);
        loop {
            if let Event::Error { message, request } = self.next() {
                return (message, request);
            }
        }
    }
    fn lease(&mut self, pane: u64) {
        self.send(Request::Claim {
            pane,
            takeover: false,
        });
        self.until(|event| match event {
            Event::Lease {
                pane: p,
                owner: Some(owner),
            } if *p == pane => Some(*owner),
            _ => None,
        });
    }
    fn spawn(&mut self, request: &str, argv: &[&str]) -> u64 {
        self.send(Request::Spawn {
            request: request.into(),
            cwd: None,
            argv: argv.iter().map(|a| (*a).into()).collect(),
        });
        self.until(|event| match event {
            Event::Spawned { request: r, pane } if r == request => Some(*pane),
            _ => None,
        })
    }
    fn screen_until(&mut self, pane: u64, predicate: impl Fn(&Screen) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self
            .screens
            .get(&pane)
            .is_some_and(|s| predicate(&s.screen))
        {
            assert!(Instant::now() < deadline, "screen condition not reached");
            self.next();
        }
    }
}
fn text(s: &Screen) -> String {
    s.cells.iter().map(|c| c.text.as_str()).collect()
}

#[test]
fn welcome_advertises_session_capabilities_and_server_version() {
    let daemon = Daemon::start();
    let peer = daemon.peer();
    for capability in [
        "pane-lifecycle",
        "closed-sessions",
        "pane-history",
        "pane-scroll",
        "notification-ack-all",
        "host-status",
        "server-lifecycle",
        "pane-spawn",
    ] {
        assert!(
            peer.capabilities.iter().any(|c| c == capability),
            "{capability}"
        );
    }
    assert_eq!(
        peer.server_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
}

/// A v1 client that opts in to nothing must never see an event type it does
/// not know, even while another client creates and closes panes.
#[test]
fn clients_that_do_not_opt_in_receive_only_v1_events() {
    let daemon = Daemon::start();
    let mut old = daemon.peer();
    old.send(Request::Subscribe { panes: vec![] });
    let mut new = daemon.peer();
    new.send(Request::SessionEvents { enabled: true });
    let pane = new.spawn("compat-1", &[]);
    new.send(Request::Close {
        request: "compat-2".into(),
        pane,
    });
    new.until(|event| {
        matches!(event, Event::Done { request } if request == "compat-2").then_some(())
    });
    // The raw JSON is what a v1 client decodes.
    let deadline = Instant::now() + Duration::from_secs(4);
    old.socket
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    while Instant::now() < deadline {
        let mut header = [0u8; 4];
        if std::io::Read::read_exact(&mut old.socket, &mut header).is_err() {
            continue;
        }
        let mut payload = vec![0; u32::from_be_bytes(header) as usize];
        std::io::Read::read_exact(&mut old.socket, &mut payload).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        let kind = value["Stream"]["type"].as_str().unwrap().to_owned();
        assert!(
            ["welcome", "update", "lease", "clipboard", "error"].contains(&kind.as_str()),
            "{kind}"
        );
        if kind == "update" {
            assert!(value["Stream"].get("sessions").is_none());
            assert!(value["Stream"].get("host").is_none());
        }
        if kind == "welcome" {
            assert_eq!(value["Stream"]["version"], 1);
        }
    }
}

#[test]
fn spawn_and_close_emit_ordered_lifecycle_events_with_session_data() {
    let daemon = Daemon::start();
    let mut peer = daemon.peer();
    peer.send(Request::SessionEvents { enabled: true });
    let initial = peer.update_until(|u| u.sessions.len() == 2);
    let ids: Vec<_> = initial.sessions.iter().map(|s| s.pane).collect();
    assert_eq!(ids, vec![1, 2], "IDs give creation order");
    assert!(
        initial
            .sessions
            .iter()
            .all(|s| s.created_at > 0 && s.argv.is_empty())
    );

    peer.send(Request::Spawn {
        request: "spawn-1".into(),
        cwd: Some(daemon.root.clone()),
        argv: vec![
            "sh".into(),
            "-c".into(),
            "echo SPAWNED; exec sleep 300".into(),
        ],
    });
    let mut created = None;
    let mut spawned = None;
    while created.is_none() || spawned.is_none() {
        match peer.until(|e| match e {
            Event::PaneCreated { .. } | Event::Spawned { .. } => Some(e.clone()),
            _ => None,
        }) {
            Event::PaneCreated { session } => {
                assert!(spawned.is_none(), "creation event precedes the reply");
                created = Some(session);
            }
            Event::Spawned { pane, .. } => spawned = Some(pane),
            _ => unreachable!(),
        }
    }
    let session = created.unwrap();
    let pane = spawned.unwrap();
    assert_eq!(session.pane, pane);
    assert!(pane > 2, "IDs are never reused");
    assert_eq!(session.argv, ["sh", "-c", "echo SPAWNED; exec sleep 300"]);
    assert_eq!(
        session.launch_cwd,
        daemon.root.canonicalize().unwrap().to_string_lossy()
    );
    assert!(session.startup.as_deref().unwrap().contains("SPAWNED"));
    // The first update after the event already contains the pane.
    let update = peer.until(|e| match e {
        Event::Update(u) => Some(u.clone()),
        _ => None,
    });
    assert!(update.panes.iter().any(|p| p.id == pane));
    assert!(update.sessions.iter().any(|s| s.pane == pane));

    // An idempotent retry returns the same pane without creating another.
    assert_eq!(
        peer.spawn("spawn-1", &["sh", "-c", "echo SPAWNED; exec sleep 300"]),
        pane
    );
    let (message, request) = peer.error(Request::Spawn {
        request: "spawn-1".into(),
        cwd: None,
        argv: vec![],
    });
    assert!(message.contains("different parameters"), "{message}");
    assert_eq!(request.as_deref(), Some("spawn-1"));

    // Another client's lease protects the pane from being closed.
    let mut other = daemon.peer();
    other.lease(pane);
    let (message, request) = peer.error(Request::Close {
        request: "close-1".into(),
        pane,
    });
    assert!(message.contains("leased"), "{message}");
    assert_eq!(request.as_deref(), Some("close-1"));
    drop(other);
    // The server notices the disconnect asynchronously; retry until the lease is gone.
    let deadline = Instant::now() + Duration::from_secs(5);
    let closed = 'retry: loop {
        assert!(Instant::now() < deadline, "lease was not released");
        peer.send(Request::Close {
            request: "close-2".into(),
            pane,
        });
        loop {
            match peer.next() {
                Event::Error { request, .. } if request.as_deref() == Some("close-2") => {
                    thread::sleep(Duration::from_millis(50));
                    continue 'retry;
                }
                event @ Event::PaneClosed { .. } => break 'retry event,
                _ => {}
            }
        }
    };
    let Event::PaneClosed {
        pane: closed,
        reason,
        record,
        ..
    } = closed
    else {
        unreachable!()
    };
    assert_eq!(closed, pane);
    assert_eq!(reason, CloseReason::Closed);
    assert!(record.is_none(), "no foreground program was observed");
    peer.until(|e| matches!(e, Event::Done { request } if request == "close-2").then_some(()));
    let update = peer.until(|e| match e {
        Event::Update(u) => Some(u.clone()),
        _ => None,
    });
    assert!(!update.panes.iter().any(|p| p.id == pane));
    assert!(!update.sessions.iter().any(|s| s.pane == pane));
    let (message, _) = peer.error(Request::Close {
        request: "close-3".into(),
        pane,
    });
    assert!(message.contains("unknown pane"), "{message}");
}

#[test]
fn exited_panes_report_status_then_close() {
    let daemon = Daemon::start();
    let mut peer = daemon.peer();
    peer.send(Request::SessionEvents { enabled: true });
    // The startup command runs before `exec $SHELL -i`; `exit` ends the shell.
    let pane = peer.spawn("exit-1", &["exit", "7"]);
    let (code, signal) = peer.until(|e| match e {
        Event::PaneExited {
            pane: p,
            code,
            signal,
            ..
        } if *p == pane => Some((*code, *signal)),
        Event::PaneClosed { pane: p, .. } if *p == pane => panic!("closed before exit"),
        _ => None,
    });
    assert_eq!((code, signal), (Some(7), None));
    let reason = peer.until(|e| match e {
        Event::PaneClosed {
            pane: p, reason, ..
        } if *p == pane => Some(*reason),
        _ => None,
    });
    assert_eq!(reason, CloseReason::Exited);
}

#[test]
fn history_search_scroll_and_reveal_on_the_stream() {
    let daemon = Daemon::start();
    let mut peer = daemon.peer();
    peer.send(Request::Subscribe { panes: vec![1] });
    peer.lease(1);
    peer.send(Request::Input {
        pane: 1,
        // Quotes keep the typed command line from matching the output.
        bytes: b"i=0; while [ $i -lt 80 ]; do echo row-$i; i=$((i+1)); done; echo NEEDLE-O''NE\n"
            .to_vec(),
    });
    peer.screen_until(1, |s| text(s).contains("NEEDLE-ONE"));
    peer.send(Request::History {
        request: "h1".into(),
        pane: 1,
        start: -1000,
        limit: 200,
    });
    let lines = peer.until(|e| match e {
        Event::History {
            request,
            pane: 1,
            lines,
        } if request == "h1" => Some(lines.clone()),
        _ => None,
    });
    assert!(lines.iter().any(|(_, line)| line == "row-0"), "{lines:?}");
    assert!(lines.windows(2).all(|w| w[0].0 < w[1].0));
    let (message, request) = peer.error(Request::History {
        request: "h2".into(),
        pane: 1,
        start: 0,
        limit: 201,
    });
    assert!(message.contains("200"), "{message}");
    assert_eq!(request.as_deref(), Some("h2"));

    // Search across every pane, and across an explicit subset.
    let mut other = daemon.peer();
    other.send(Request::Subscribe { panes: vec![2] });
    other.lease(2);
    other.send(Request::Input {
        pane: 2,
        bytes: b"echo needle-T''WO\n".to_vec(),
    });
    other.screen_until(2, |s| text(s).contains("needle-TWO"));
    peer.send(Request::Search {
        request: "s1".into(),
        panes: vec![],
        query: "needle".into(),
        limit: 64,
    });
    let (results, truncated) = peer.until(|e| match e {
        Event::Search {
            request,
            results,
            truncated,
        } if request == "s1" => Some((results.clone(), *truncated)),
        _ => None,
    });
    assert!(!truncated);
    let panes: Vec<_> = results.iter().map(|r| r.pane).collect();
    assert_eq!(panes, vec![1, 2], "creation order");
    assert!(results.iter().all(|r| !r.hits.is_empty()));
    peer.send(Request::Search {
        request: "s2".into(),
        panes: vec![2],
        query: "NEEDLE".into(),
        limit: 1,
    });
    let results = peer.until(|e| match e {
        Event::Search {
            request, results, ..
        } if request == "s2" => Some(results.clone()),
        _ => None,
    });
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].pane, 2);
    assert_eq!(results[0].hits.len(), 1);
    let (message, request) = peer.error(Request::Search {
        request: "s3".into(),
        panes: vec![99],
        query: "x".into(),
        limit: 1,
    });
    assert!(message.contains("invalid pane search"), "{message}");
    assert_eq!(request.as_deref(), Some("s3"));
    let (message, request) = peer.error(Request::Search {
        request: "bad id!".into(),
        panes: vec![],
        query: "x".into(),
        limit: 1,
    });
    assert!(message.contains("request identifier"), "{message}");
    assert_eq!(request, None);

    // Scrolling needs the lease and moves the pane's shared viewport.
    let (message, _) = other.error(Request::Scroll { pane: 1, lines: 10 });
    assert!(message.contains("lease"), "{message}");
    peer.send(Request::Scroll { pane: 1, lines: 30 });
    peer.screen_until(1, |s| {
        !text(s).contains("NEEDLE-ONE") && text(s).contains("row-")
    });
    peer.send(Request::Reveal { pane: 1, line: 0 });
    peer.screen_until(1, |s| text(s).contains("NEEDLE-ONE"));
}

#[test]
fn acknowledge_all_marks_every_notification_read() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let daemon = Daemon::start_with(|state| {
        let events: Vec<_> = (1..=2)
            .map(|id| {
                serde_json::json!({
                    "id": id, "pane": 1, "title": format!("seeded {id}"),
                    "status": "turn_finished", "at": now, "read": false, "workspace": "test"
                })
            })
            .collect();
        let path = state.join("activity.json");
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "initialized": true, "next_id": 3, "events": events, "seen": {}
            }))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    });
    let mut peer = daemon.peer();
    peer.send(Request::Subscribe { panes: vec![] });
    peer.update_until(|u| u.notifications.len() == 2 && u.notifications.iter().all(|n| !n.read));
    peer.send(Request::AcknowledgeAll);
    peer.update_until(|u| u.notifications.len() == 2 && u.notifications.iter().all(|n| n.read));
}

#[test]
fn host_status_is_opt_in() {
    let daemon = Daemon::start();
    let mut peer = daemon.peer();
    peer.send(Request::Subscribe { panes: vec![] });
    let first = peer.update_until(|_| true);
    assert!(first.host.is_none());
    peer.send(Request::HostStatus { enabled: true });
    let update = peer.update_until(|u| u.host.as_ref().is_some_and(|h| h.ready));
    let host = update.host.unwrap();
    assert!(host.memory_total > 0);
    assert!(!host.cpus.is_empty());
    peer.send(Request::HostStatus { enabled: false });
    peer.update_until(|u| u.host.is_none());
}

/// Parking is client-only. A client that hides a pane keeps its lease (and so
/// its size) while unsubscribing; the server needs no "parked" concept.
#[test]
fn a_leased_but_unsubscribed_pane_keeps_its_size() {
    let daemon = Daemon::start();
    let mut peer = daemon.peer();
    peer.send(Request::Subscribe { panes: vec![1] });
    peer.lease(1);
    peer.send(Request::Resize {
        pane: 1,
        cols: 57,
        rows: 19,
    });
    peer.screen_until(1, |s| s.cols == 57 && s.rows == 19);
    peer.send(Request::Subscribe { panes: vec![] });
    peer.update_until(|u| u.subscribed.is_empty());
    // A composed-protocol resize of the base geometry must not touch it.
    let mut tui = daemon.socket();
    write_message(
        &mut tui,
        &ClientMessage::Attach {
            version: VERSION,
            cols: 120,
            rows: 40,
            takeover: false,
        },
    )
    .unwrap();
    thread::sleep(Duration::from_millis(300));
    let screen = daemon.inspect(1);
    assert_eq!((screen.cols, screen.rows), (57, 19));
    peer.send(Request::Release { pane: 1 });
    let deadline = Instant::now() + Duration::from_secs(5);
    while daemon.inspect(1).cols == 57 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn closed_sessions_list_and_restore_by_record() {
    let daemon = Daemon::start();
    let mut peer = daemon.peer();
    peer.send(Request::SessionEvents { enabled: true });
    let pane = peer.spawn("prog-1", &[]);
    peer.lease(pane);
    peer.send(Request::Input {
        pane,
        bytes: b"sleep 300\n".to_vec(),
    });
    // Discovery observes the foreground program within a few seconds.
    peer.update_until(|u| u.panes.iter().any(|p| p.id == pane && p.command.is_some()));
    // One more discovery cycle so the history observation is saved.
    thread::sleep(Duration::from_millis(2500));
    peer.send(Request::Close {
        request: "close-prog".into(),
        pane,
    });
    let record = peer.until(|e| match e {
        Event::PaneClosed {
            pane: p, record, ..
        } if *p == pane => Some(record.clone()),
        _ => None,
    });
    let record = record.expect("a closed program leaves a record");
    assert_eq!(record, format!("{}:{pane}", peer.epoch));

    let (message, request) = peer.error(Request::Restore {
        request: "early".into(),
        record: record.clone(),
        rerun: true,
    });
    assert!(message.contains("list closed sessions"), "{message}");
    assert_eq!(request.as_deref(), Some("early"));

    peer.send(Request::ClosedSessions {
        request: "list-1".into(),
    });
    let records = peer.until(|e| match e {
        Event::ClosedSessions { request, records } if request == "list-1" => Some(records.clone()),
        _ => None,
    });
    let listed = records
        .iter()
        .find(|r| r.record.key == record)
        .expect("record is listed");
    assert_eq!(listed.record.argv, ["sleep", "300"]);
    assert_eq!(listed.record.reason, "Closed");
    assert_eq!(listed.rerun.as_deref(), Some("'sleep' '300'"));
    assert!(listed.resume.is_none());

    peer.send(Request::Restore {
        request: "restore-1".into(),
        record: record.clone(),
        rerun: true,
    });
    let (restored, existing) = peer.until(|e| match e {
        Event::Restored {
            request,
            record: r,
            pane,
            existing,
        } if request == "restore-1" => {
            assert_eq!(r, &record);
            Some((*pane, *existing))
        }
        _ => None,
    });
    assert!(!existing);
    assert!(restored > pane);
    let update = peer.update_until(|u| u.sessions.iter().any(|s| s.pane == restored));
    let session = update.sessions.iter().find(|s| s.pane == restored).unwrap();
    assert_eq!(session.startup.as_deref(), Some("'sleep' '300'"));

    // A record is consumed by restoring it.
    let (message, _) = peer.error(Request::Restore {
        request: "restore-2".into(),
        record: record.clone(),
        rerun: true,
    });
    assert!(
        message.contains("unknown closed-session record"),
        "{message}"
    );
    peer.send(Request::ClosedSessions {
        request: "list-2".into(),
    });
    let records = peer.until(|e| match e {
        Event::ClosedSessions { request, records } if request == "list-2" => Some(records.clone()),
        _ => None,
    });
    assert!(!records.iter().any(|r| r.record.key == record));
}

#[test]
fn shutdown_request_announces_the_stop() {
    let mut daemon = Daemon::start();
    let mut peer = daemon.peer();
    peer.send(Request::SessionEvents { enabled: true });
    peer.send(Request::Shutdown {
        request: "stop-1".into(),
    });
    peer.until(|e| matches!(e, Event::Done { request } if request == "stop-1").then_some(()));
    let reason = peer.until(|e| match e {
        Event::ServerStopping { reason } => Some(*reason),
        _ => None,
    });
    assert_eq!(reason, StopReason::Shutdown);
    daemon.wait_stopped();
}

#[test]
fn last_pane_exit_announces_the_stop() {
    let mut daemon = Daemon::start();
    let mut peer = daemon.peer();
    peer.send(Request::SessionEvents { enabled: true });
    peer.send(Request::Subscribe { panes: vec![1, 2] });
    for pane in [1, 2] {
        peer.lease(pane);
        peer.send(Request::Input {
            pane,
            bytes: b"exit\n".to_vec(),
        });
    }
    let reason = peer.until(|e| match e {
        Event::ServerStopping { reason } => Some(*reason),
        _ => None,
    });
    assert_eq!(reason, StopReason::LastPaneExited);
    daemon.wait_stopped();
}

#[test]
fn new_requests_require_hello() {
    let daemon = Daemon::start();
    let mut socket = daemon.socket();
    write_message(
        &mut socket,
        &ClientMessage::Stream(Request::ClosedSessions {
            request: "x".into(),
        }),
    )
    .unwrap();
    match read_message::<ServerMessage>(&mut socket).unwrap().unwrap() {
        ServerMessage::Stream(Event::Error { message, request }) => {
            assert!(message.contains("hello"), "{message}");
            assert_eq!(request.as_deref(), Some("x"));
        }
        other => panic!("{other:?}"),
    }
    // The pane stream itself still works afterwards for a proper client.
    let mut peer = daemon.peer();
    peer.send(Request::Subscribe { panes: vec![1] });
    peer.screen_until(1, |_| true);
    assert_eq!(daemon.inspect(1).cols, peer.screens[&1].screen.cols);
}
