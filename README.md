# EZF-Plugin-SDK

[English](README.md) · [Deutsch](README_DE.md)

**EZF-Plugin-SDK** is the shared Rust contract layer for the EZF backend's dynamically loaded plugins. It collects the database data contracts, the low-level plugin ABI, and the Rust-side plugin SDK in one repository while keeping them as separate crates.

The repository is intended to be consumed by the EZF backend and its plugins, including plugins built independently from the backend checkout.

## Crates

| Crate | Responsibility |
| --- | --- |
| [`DatabaseContracts`](DatabaseContracts/) | Database-neutral request, response, parameter, result, value, and error types. The crate defines the data exchanged for database operations; it does not implement a database driver or connection. |
| [`PluginAbi`](PluginAbi/) | C-compatible ABI V2 primitives for the dynamic-library boundary, including function-table types, byte slices and buffers, host capabilities, status codes, and cooperative cancellation. |
| [`PluginContracts`](PluginContracts/) | Rust-side plugin SDK and logical contracts: the `IPlugin` trait, plugin manifests and endpoint descriptions, catalog validation, lifecycle context, endpoint request/response types, logging, and access to host-provided database execution. |

The crates have separate responsibilities and are not merged into a single crate. `PluginContracts` depends on `DatabaseContracts` and `PluginAbi` and publicly re-exports both, so plugin implementations can use the types from the same resolved contract dependency.

## Architecture

```text
EZF backend / PluginHost
          │
          │ host capabilities and lifecycle calls
          ▼
   Plugin ABI V2 boundary
   (C-compatible function table + byte buffers)
          │
          ▼
   PluginContracts ───────► DatabaseContracts
          │
          └───────────────► PluginAbi
```

The dynamic-library boundary deliberately avoids passing Rust-specific data types such as `String`, `Vec`, trait objects, or Rust enums directly across the ABI. Calls use C-compatible structures and explicitly managed byte buffers; contract payloads are serialized. Buffers must be released by the side that allocated them through the corresponding ABI callback.

### Key design points

- **Separate logical and binary contracts.** `PluginContracts` describes the Rust-side plugin API; `PluginAbi` defines the lower-level boundary used by the host and plugin library.
- **Cooperative cancellation.** ABI V2 passes a per-call cancellation signal to potentially long-running plugin operations. Cancellation is cooperative: plugin code must check the signal at appropriate safe interruption points. The host does not forcefully terminate plugin code.
- **Database capability, not a database implementation.** Database requests and results are defined in `DatabaseContracts`. Actual execution is provided by the host when the database capability is available.
- **Explicit endpoint metadata.** Plugin manifests describe endpoint names, routes, HTTP methods, request/response media types, and optional query/body metadata. `PluginCatalog` validates manifests and detects conflicting routes and identifiers.
- **Lifecycle contracts.** `IPlugin` describes plugin operations and lifecycle hooks, including load, unload, destroy, and session-close callbacks.

## Current contract versions

The repository contains several independent version identifiers. They serve different purposes and must not be treated as interchangeable.

| Identifier | Current value | Meaning |
| --- | ---: | --- |
| Workspace/package version | `1.0.0` | Cargo package version declared by the standalone Plugin-SDK workspace. |
| `PLUGIN_CONTRACT_VERSION` | `8` | Version of the logical plugin manifest/API contract. |
| `PLUGIN_ABI_VERSION_V2` | `2` | Version of the binary plugin ABI. |

The source code is the authority for these values. When changing the ABI layout, function signatures, buffer ownership rules, or cancellation semantics, assess compatibility with both the host and all affected plugins. Changes to serialized plugin contracts should likewise be validated against the host and plugin versions that will exchange them.

## Build and test

A Rust toolchain supporting **Edition 2024** is required.

From the repository root, run:

```bash
# Compile all crates in the workspace
cargo build --workspace

# Run the workspace tests
cargo test --workspace

# Check formatting
cargo fmt --all -- --check
```

To inspect the resolved workspace dependencies:

```bash
cargo tree --workspace
```

## Consuming EZF-Plugin-SDK

### As a Git dependency

For a plugin repository that is built independently of the EZF backend checkout, depend on `PluginContracts` from this repository. Prefer pinning a reviewed, immutable commit (or a release tag once the project adopts tags) rather than following a moving branch:

```toml
[dependencies]
PluginContracts = { git = "https://github.com/ukf-kohnke/EZF-Plugin-SDK.git", rev = "<full-commit-sha>" }
```

`PluginContracts` resolves its sibling crates from the same repository checkout. Because it re-exports `DatabaseContracts` and `PluginAbi`, plugin code can import these types through the same dependency, for example:

```rust
use PluginContracts::DatabaseContracts::{
    DatabaseError,
    DatabaseRequest,
    DatabaseResponse,
};
use PluginContracts::PluginAbi;
use PluginContracts::ApiContract::{PluginManifest, PluginKind};
```

Avoid adding a second, independently resolved copy of `DatabaseContracts` or `PluginAbi` unless there is a deliberate reason to do so. Using types from a single resolved contract source helps prevent duplicate crate identities and version drift.

### As a submodule in EZF-Backend

The backend includes this repository as the `Plugin-SDK/` Git submodule and uses local path dependencies for integrated development. The parent repository should pin the submodule to a commit compatible with the `EZF-VariantFilter` and other plugin revisions being built. Update and test the contracts and plugins together when changing shared types.

## Compatibility guidance

- Pin the contract commit used by each plugin build and record the corresponding submodule commit in the backend repository.
- Test `EZF-Plugin-SDK` on its own and then build/test the integrated backend and affected plugins.
- Treat changes to the ABI and serialized contract types as compatibility-sensitive; do not assume that matching Cargo package versions alone prove compatibility.
- Cancellation is cooperative, not a mechanism for forcibly stopping arbitrary plugin code.
- This repository contains contracts and SDK code, not the plugin host runtime, database drivers, or application-specific plugin implementations.

## License

The Cargo manifests declare dual licensing under either the [MIT License](https://opensource.org/license/mit) or the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0), at your option. Include the corresponding license text files in the repository when publishing the project.
