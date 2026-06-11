# OrbKit

OrbKit is the Proof of Humanity (PoH) issuer package for [World ID](https://world.org/world-id). It owns the on-device lifecycle of an Orb-issued credential: how Personal Custody Packages (PCPs) are stored, queried, and updated as the user enrolls, re-authenticates, or recovers their identity.

Part of the [World ID SDK](https://docs.world.org/world-id).

> **Status:** early development. Not yet published. Swift/Kotlin bindings are planned but not implemented yet.

## What OrbKit does

- Holds the user's PCPs in an encrypted, on-device store.
- Tracks each signup's lifecycle: download, enrollment submission, success, failure, abandonment, identity deletion.
- Knows which PCP is the latest enrolled one and supports re-authentication and pioneer-reset flows that supersede it.
- Acks downloads and supports per-tier shards of a single signup.

The encrypted storage is provided by [`walletkit-db`](https://github.com/worldcoin/walletkit/tree/main/walletkit-db); OrbKit composes its PCP schema and access patterns on top.

## How storage is protected

OrbKit's PCP vault is a sqlite3mc-encrypted `SQLite` file. Page-level encryption uses a 32-byte working key (`K_intermediate`); that key is itself sealed under the device's hardware keystore key (`K_device`) using ChaCha20-Poly1305 AEAD with a per-consumer associated-data string, and persisted as a CBOR envelope.

Sealing and unsealing happen inside the device's TEE (Secure Enclave on iOS, TEE / StrongBox on Android). `K_device` never crosses into app memory. Lose the device, lose `K_device`, lose the vault permanently — recovery comes through a separate backup path, not through the envelope.

For the seal / unseal flow, full key hierarchy, and threat model, see [`walletkit-db`'s README](https://github.com/worldcoin/walletkit/tree/main/walletkit-db).

## Public API

```rust
use orb_kit::storage::{OrbPcpStore, PcpIngest, PackageStatus, CreationSource};

// Open the encrypted PCP vault for this device.
let store = OrbPcpStore::open(&vault_path, now, &lock, &keystore, &blob_store)?;

// Ingest a downloaded PCP (one tier of a signup).
store.put_package(&ingest)?;

// Advance the enrollment state machine across every tier of a signup.
store.update_status(signup_id, PackageStatus::EnrollmentRequestInitiated, None, now)?;

// Record a successful download ack for one tier.
store.mark_ack(signup_id, tier, now)?;

// Read the latest signup with at least one tier in `Enrolled` status.
let tiers = store.latest_enrolled()?;
```

The full API surface is in [`src/storage`](src/storage). Public types: `OrbPcpStore`, `PcpIngest`, `PcpRecord`, `PackageStatus`, `CreationSource`, `StorageError`.

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
