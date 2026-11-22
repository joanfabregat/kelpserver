//! The session server: owned PTYs and terminal emulation, the pane stream,
//! pane observations, closed-session records, host sampling and the server
//! lifecycle. Presentation and client UI state live in the clients.
pub mod activity;
pub mod background;
pub mod bridge;
pub mod cli;
pub mod connection;
pub mod diagnostics;
pub mod engine;
pub mod history;
pub mod hooks;
pub mod lifecycle;
pub mod program_status;
pub mod protocol;
pub mod recovery;
pub mod reference;
pub mod server;
pub mod shell;
pub mod status;
pub mod stream;
pub mod templates;
pub mod tracking;
pub mod upgrade;
pub mod vitals;
pub mod web_relay;
