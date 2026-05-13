# OrbKit

OrbKit is the Proof of Humanity (PoH) issuer package for [World ID](https://world.org/world-id). It owns the on-device lifecycle of an Orb-issued credential: how Personal Custody Packages (PCPs) are stored, queried, and updated as the user enrolls, re-authenticates, or recovers their identity.

Part of the [World ID SDK](https://docs.world.org/world-id).

> **Status:** early development. Not yet published. Swift/Kotlin bindings are planned but not implemented yet.

## What OrbKit does

- Holds the user's PCPs in an encrypted, on-device store.
- Tracks each signup's lifecycle: download, enrollment submission, success, failure, abandonment, identity deletion.
- Knows which PCP is the active one and supports re-authentication and pioneer-reset flows that supersede it.
- Acks downloads, supports per-tier shards of a single signup, and cleans up bytes when a PCP is no longer referenced.

The encrypted storage is provided by [`walletkit-db`](https://github.com/worldcoin/walletkit/tree/main/walletkit-db); OrbKit composes its PCP schema and access patterns on top.

## Public API

```rust
use orb_kit::storage::{Vault, OrbPcpStore, PackageStatus, CreationSource};

// Open the encrypted PCP vault for this device.
let vault = Vault::open(&vault_path, now, lock, &keystore, &blob_store)?;
let store = vault.store();

// Ingest a downloaded PCP (one tier of a signup).
store.put_package(&ingest)?;

// Advance the enrollment state machine.
store.update_status(signup_id, PackageStatus::EnrollmentRequestInitiated, None, now)?;

// After a successful re-auth, promote the new signup atomically.
store.promote_to_current(new_signup_id, Some(CreationSource::ReAuthentication), now)?;

// Read the active PCP (all tier shards).
let tiers = store.current_pcp_tiers()?;
```

The full API surface is in [`src/storage`](src/storage). Notable types: `Vault`, `OrbPcpStore`, `PcpRecord`, `PackageStatus`, `CreationSource`, `StorageError`.

## Development

Install Rust via [`rustup`](https://www.rust-lang.org/tools/install), then:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

## Security

For security issues, please see [SECURITY](./SECURITY.md). Do not file public issues for vulnerabilities.

## Code of Conduct

See [CODE_OF_CONDUCT](./CODE_OF_CONDUCT.md).

## Contributing

See [CONTRIBUTING](./CONTRIBUTING.md).
