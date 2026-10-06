# cx status

Implementation is in progress, not release ready. Local project has no GitHub remote; public publication is not authorized. Baseline f30bfa2; three native workers use isolated build branches under worktrees.

Working inventory: innovation Arch/tmux 3.7c/Codex 0.160.1/Claude 2.1.290; tranquility Arch with system Codex 0.160.1 but Fish selects user Codex 0.157.1, npm installed, user-owned signature-verified tmux added; server Ubuntu 24.04/tmux 3.4, no AI runtimes. Existing rideshare_planning untouched. Five inter-device SSH directions work; innovation self-alias trust fails and is not modified. Linger is disabled on both Arch accounts. These observations are local deployment facts, not product defaults.

Root owns shared model, transport, persistence, manifests, integration and deployment. Workers own sessions; UI; files/read-only network. Live network changes and trust changes remain serialized and gated by actual trust/privilege requirements. The legacy robotics shared_internet.sh replaces the host default route and appends unowned firewall rules; it is inspected, not executed or changed.
