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

## UI maintenance

`src/ui.rs` owns state, action dispatch, task replies and native attachment. Private modules under `src/ui/` keep changes local:

| Module | Responsibility |
|---|---|
| `navigation.rs` | Counted key motions and the `Target` interface; no requests or rendering |
| `panels.rs` | List, device and file adapters, including refresh/selection effects |
| `filters.rs` | Scoped filter state, device identity mapping and fuzzy matching |
| `menus.rs` | Session choices shared by labels/shortcuts/dispatch; container display labels |
| `rendering.rs` | Layout and rendering; no request dispatch |
| `tests.rs` | Existing UI and cross-panel regression fixtures |

Reuse a navigation target when adding a panel; keep panel-specific side effects in its adapter. Change session choices in `menus.rs` rather than maintaining parallel label/key/provider arrays. Rendering never changes the session provider: devcontainer identity is a display scope, while shell/agent behavior stays in the session runtime. Preserve typed-input ownership and the existing host/container scope guards. Verify UI changes with the capture matrices and native PTY fixtures in `tests/`.

Emit footer notifications through `App::set_notice`: it renews a five-second monotonic deadline, and the event loop clears expired text and redraws while idle. Right alignment and timer renewal follow noice.nvim's [mini view configuration](https://github.com/folke/noice.nvim/blob/7bfd942445fb63089b59f97ca487d605e715f155/lua/noice/config/views.lua) and [mini backend](https://github.com/folke/noice.nvim/blob/7bfd942445fb63089b59f97ca487d605e715f155/lua/noice/view/backend/mini.lua). CX keeps its single footer notice rather than adding a notification stack.

## Architecture and current limits

Network observation and SSH jump routes work. Internet sharing, full trust/revocation and full uninstall are not release-ready. No public hostnames or mesh provider are required. See [robot mesh sharing](robot-mesh-sharing.md) for access policy, sharing previews and their acceptance requirements, and [robot execution and updates](robot-tools.md) for platform-specific tooling.

For session naming research, see [Codex session identity](../research/codex-session-names.md).

[Back to README](../README.md)
