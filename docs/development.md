# Development

Work from an isolated branch/worktree as described in [AGENTS.md](../AGENTS.md).

## Build and test

```sh
cargo build --release --locked
cargo test --locked
```

GitHub workers test builds and installer fixtures across distributions. Rocky Linux supplies the RHEL-compatible CI fixture; RHEL itself requires access to a licensed environment. See [validation](../VALIDATION.md) for actual test results and remaining limits, and [status](../STATUS.md) for outstanding work.

## Releases

Record changes in [CHANGELOG.md](../CHANGELOG.md). Before tagging a stable version, move its reviewed changes from Unreleased into an exact `vX.Y.Z` section; publication requires these notes.

## Architecture and current limits

Network observation and SSH jump routes work. Internet sharing, full trust/revocation and full uninstall are not release-ready. No public hostnames or mesh provider are required. See [robot mesh sharing](robot-mesh-sharing.md) for access policy, sharing previews and their acceptance requirements, and [robot execution and updates](robot-tools.md) for platform-specific tooling.

For session naming research, see [Codex session identity](../research/codex-session-names.md).

[Back to README](../README.md)
