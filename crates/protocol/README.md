# Kelp terminal protocol

Portable terminal wire types, the stream version constant and cell validation shared by the [server](../server) in the [KelpServer repository](../../README.md) and its clients, including the Rust/WASM client in [KelpMux](https://github.com/joanfabregat/kelpmux). This crate has no server, client presentation or WASM dependency. Terminal stream version 1 is unchanged by the extraction; version 0.2.0 removed `LOCAL_PROTOCOL_VERSION` with the composed protocol 58 (ADR 0002 step 3).

See [stream documentation](../server/docs/native-client-protocol.md), [contribution policy](../../CONTRIBUTING.md), [security policy](../../SECURITY.md) and the repository [Apache-2.0 license](../../LICENSE).
