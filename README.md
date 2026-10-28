# OrbKit

OrbKit is the Proof of Humanity (PoH) issuer package for [World ID](https://world.org/world-id). It owns the on-device lifecycle of an Orb-issued credential: how Personal Custody Packages (PCPs) are stored, queried, and updated as the user enrolls, re-authenticates, or recovers their identity.

Part of the [World ID SDK](https://docs.world.org/world-id).

> **Status:** early development. Not yet published. Swift/Kotlin bindings are planned but not implemented yet.

## What OrbKit does

- Holds the user's PCPs in an encrypted, on-device vault, one row per signup.
- Tracks each signup's enrollment lifecycle as a status state machine.

Storage and encryption come from [`walletkit-db`](https://github.com/worldcoin/walletkit/tree/main/crates/walletkit-db); see its README for the key hierarchy and threat model. The API lives in [`src/storage`](src/storage).

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
