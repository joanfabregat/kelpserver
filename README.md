# KelpServer

KelpServer is the persistent, user-owned terminal server of the Kelp project. It owns PTYs, process lifecycle, authoritative terminal emulation (Alacritty's engine), scrollback, search, selection, parking, input ownership and per-pane sizing, recovery metadata and agent activity discovery. Detaching or losing a client leaves shells and agents running. It also contains a generic, fixed-destination SSH byte relay for browser clients. Recovery metadata cannot preserve live processes across a server crash or reboot.

This repository was extracted from [KelpMux](https://github.com/joanfabregat/kelpmux) at commit `eb31da5` so that every client depends on one versioned server contract. The history before the extraction stays in KelpMux.

## Packages

| Package | Location | Responsibility |
| --- | --- | --- |
| `kelpserver` | [crates/server](crates/server) | Library and `kelpserver` executable: daemon, PTYs, terminal state, local and stream protocols, SSH bridge, generic web relay |
| `kelpclient-protocol` | [crates/protocol](crates/protocol) | Portable wire types, protocol version constants and cell validation, with no server, client-state or WASM dependency |

The package names are historical and kept so that existing consumers need no source changes.

## Clients and the contract

KelpServer has no dependency on any client. Its clients are:

- **KelpMux TUI** (the `kelpmux` executable): attaches over the composed-screen local protocol and launches or reuses the standalone `kelpserver` daemon. Per [ADR 0002](crates/server/docs/adr/0002-generic-session-server.md) it is moving to the pane stream with local rendering and local UI state; the stream's session API (step 1) is in place.
- **KelpMux web client** (`kelpmux web-relay` / `kelpmux web-bridge`) and the **KelpClient** Rust/WASM core: use the independent pane stream and the generic relay in this repository; browser assets are supplied by the consumer.
- **KelpHub**: uses the library and the pane stream behind its own authenticated management adapter. Project, Git, upload and management features stay in KelpHub; KelpServer is not a project manager, credential broker or container orchestrator.

The contract consists of:

| Surface | Version | Stability |
| --- | --- | --- |
| Local composed-screen protocol (TUI ↔ server, same-user Unix socket) | `LOCAL_PROTOCOL_VERSION = 58` | A matching TUI/server pair is required: mouse routing shares UI geometry and dialog types. Clients check the peer's protocol before attaching. |
| Independent pane stream (`Stream` hello/welcome, snapshots, patches, leases) and its session API (pane lifecycle, closed-session records, history and search, host status, server lifecycle) | `STREAM_PROTOCOL_VERSION = 1` | Experimental and not yet frozen. Optional features are advertised as Welcome capabilities; clients must not assume undeclared ones, and must not send a request whose capability is missing. Events of new types reach only connections that opted in or sent the request, so existing v1 clients are unaffected. Unsupported versions fail clearly. See [the stream contract](crates/server/docs/native-client-protocol.md) and [the session API inventory](crates/server/docs/session-api.md). |
| Framing | 4-byte big-endian length + UTF-8 JSON, 16 MiB maximum | Shared by the Unix socket and `kelpserver bridge` over SSH. |
| Rust API of `kelpserver` and `kelpclient-protocol` | crate versions | Pre-1.0 and unstable; consumers pin an exact Git revision. |
| Executable and state | `kelpserver`, XDG state paths, `KELPMUX_*` environment names | Unchanged by the extraction. |

The version constants live in [`crates/protocol/src/lib.rs`](crates/protocol/src/lib.rs). The design background is in [ADR 0001](crates/server/docs/adr/0001-native-client-protocol-and-device-capabilities.md), and the server's internals in [docs/architecture.md](docs/architecture.md), [docs/discovery.md](docs/discovery.md) and [docs/agent-polling.md](docs/agent-polling.md).

## Using it from a client

Pin an exact, reviewed commit; never a branch:

```toml
[dependencies]
kelpserver = { git = "https://github.com/joanfabregat/kelpserver", rev = "<full commit SHA>" }
kelpclient-protocol = { git = "https://github.com/joanfabregat/kelpserver", rev = "<same SHA>" }
```

Use the same revision for every package from this repository, and ship a `kelpserver` executable built from that revision beside the client (or point `KELPSERVER_BINARY` at it). A WASM client built from another repository must use protocol types from the same revision as the server it talks to.

## Build and test

```sh
cargo build --workspace --release --locked
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The toolchain is pinned in [rust-toolchain.toml](rust-toolchain.toml); the minimum supported Rust version is 1.85. Run builds in your environment's approved container workflow. Tests use disposable shells, private sockets and private state; they never connect to a real workspace.

```sh
kelpserver start --workspace trial
kelpserver status --workspace trial
kelpserver stream-inspect --workspace trial
kelpserver web-relay --config /path/to/web.json  # relay only: embedding clients supply UI assets via web_relay::run_with_assets
kelpserver stop --workspace trial
```

Stopping a workspace ends its processes.

## Security and contributions

Use [SECURITY.md](SECURITY.md) for private vulnerability reporting. Follow [CONTRIBUTING.md](CONTRIBUTING.md) and the [repository security policy](docs/repository-security.md). Licensed under [Apache-2.0](LICENSE); see [NOTICE](NOTICE) for attribution.
