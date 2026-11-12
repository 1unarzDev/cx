use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub target: Option<String>,
    pub account: String,
    pub host: String,
    pub status: String,
    pub observed_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerScope {
    #[serde(default)]
    pub engine: String,
    pub id: String,
    pub name: String,
    pub user: String,
    pub started_at: String,
    pub folder: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalIdentity {
    pub server_pid: u32,
    pub server_start: String,
    pub session_id: String,
    pub created: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub directory: String,
    pub provider: String,
    pub host: String,
    pub account: String,
    pub pid: u32,
    pub started: String,
    pub boot_id: String,
    pub external: bool,
    pub socket: Option<String>,
    #[serde(default)]
    pub launcher: Option<String>,
    #[serde(default)]
    pub process: Option<ProcessIdentity>,
    #[serde(default)]
    pub container: Option<ContainerScope>,
    /// Stable terminal generation; pane/provider changes do not replace the terminal.
    #[serde(default)]
    pub terminal: Option<TerminalIdentity>,
    /// Observed activity, never inferred from a cached liveness label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_tasks: Vec<AgentTask>,
    /// Seconds since the terminal's last output (tmux `#{window_activity}`),
    /// measured on the execution host when observed, so clock skew between
    /// hosts cannot distort it. Older helpers omit it; viewers render `·`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_seconds: Option<u64>,
    /// Terminal evidence for a session started outside CX. Adoption re-verifies it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ExternalOrigin>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalOrigin {
    /// Owner of the terminal master: terminal, sshd, tmux or screen.
    pub kind: String,
    pub tty: String,
    pub leader: ProcessIdentity,
    #[serde(default)]
    pub holder: Option<ProcessIdentity>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunCommand {
    pub directory: String,
    pub command: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateSession {
    pub key: String,
    pub directory: String,
    pub provider: String,
    pub name: String,
    #[serde(default)]
    pub resume: bool,
    /// Resume by choosing from the provider's own session picker rather than the
    /// most recent session. Ignored unless `resume` is set.
    #[serde(default)]
    pub resume_pick: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", content = "args", rename_all = "snake_case")]
pub enum Operation {
    Info,
    PeerIdentity,
    PeerConfigure {
        peer: Device,
        host_key: String,
        client_key: String,
        /// False pins the peer's host key for outbound use without authorizing
        /// its client key (and removes an existing cx-managed line). Older
        /// helpers ignore it; callers check `peer-authorize-v1` before sending false.
        #[serde(default = "default_true", skip_serializing_if = "is_true")]
        authorize: bool,
    },
    /// Install (`authorize: true`) or remove the cx-managed authorized_keys line
    /// for one pinned peer, selected by its client key or pinned identity. The
    /// receiver refuses to authorize a peer it removed or keeps directed. Never
    /// touches lines CX did not write. Capability `peer-authorize-v1`.
    /// `authorize` is required: a missing field must not decode as a revoke.
    PeerAuthorize {
        #[serde(default)]
        peer_id: Option<String>,
        #[serde(default)]
        client_key: Option<String>,
        authorize: bool,
    },
    PeerEnroll {
        peer: Device,
        #[serde(default)]
        replace_stale: bool,
    },
    Containers,
    ContainerInspect {
        id: String,
    },
    DevcontainerUp {
        workspace: String,
    },
    DevcontainerRebuild {
        scope: ContainerScope,
    },
    ContainerAccess {
        id: String,
        engine: String,
        enabled: bool,
    },
    ContainerLifecycle {
        id: String,
        engine: String,
        started_at: String,
        action: String,
    },
    ContainerFiles {
        scope: ContainerScope,
        operation: Box<Operation>,
    },
    ContainerFileAction {
        scope: ContainerScope,
        operation: Box<Operation>,
    },
    ContainerCreate {
        scope: ContainerScope,
        request: CreateSession,
        yolo: bool,
    },
    /// Scoped installed-provider evidence; does not report authentication.
    ContainerProviders {
        scope: ContainerScope,
    },
    /// Versioned agent launch: resume selects native history, never latest.
    ContainerAgentCreate {
        scope: ContainerScope,
        request: CreateSession,
        yolo: bool,
    },
    SetLaunchShell {
        shell: String,
    },
    Sessions,
    /// Read-only current-screen snapshot for one verified terminal.
    SessionPreview {
        session: Session,
        #[serde(default)]
        scrollback: bool,
        #[serde(default)]
        history_start: Option<u32>,
    },
    /// Input to the exact pane shown in a verified live session view. Never retried.
    SessionPaneInput {
        session: Session,
        pane_id: String,
        hex: String,
    },
    /// Subscribe this channel to pushed events (capability `watch-v1`); replaces
    /// the channel's previous watch, and neither field set ends it. The reply is
    /// followed by `Response` frames with id `evt:<n>` whose result is `{kind, ...}`:
    /// `sessions` (the full list), `pane` (a `SessionPreview` result for the
    /// watched managed session) or `closed` (poll again). An error means
    /// unsupported here: poll.
    Watch {
        #[serde(default)]
        sessions: bool,
        #[serde(default)]
        pane: Option<Session>,
    },
    /// Terminals started outside CX, shown for adoption in the UI only.
    ExternalSessions,
    /// Move a terminal started outside CX into the managed tmux server.
    AdoptSession {
        id: String,
        origin: ExternalOrigin,
        boot_id: String,
    },
    Create(CreateSession),
    CreateYolo(CreateSession),
    StopAgentSession {
        id: String,
        pid: u32,
        started: String,
        boot_id: String,
        provider: String,
    },
    StopSession {
        id: String,
        pid: u32,
        started: String,
        boot_id: String,
    },
    List {
        path: String,
    },
    Preview {
        path: String,
    },
    PreviewPage {
        path: String,
        page: u32,
    },
    Mkdir {
        path: String,
    },
    Rename {
        path: String,
        name: String,
        expected_identity: Option<String>,
    },
    Remove {
        path: String,
        expected_identity: Option<String>,
    },
    RemoveEmptyDirectory {
        path: String,
        identity: String,
    },
    Move {
        path: String,
        destination: String,
        expected_identity: Option<String>,
    },
    Copy {
        source: String,
        destination: String,
        conflict: String,
        key: String,
    },
    Jobs,
    Cancel {
        key: String,
    },
    FileInfo {
        path: String,
    },
    ReadChunk {
        path: String,
        offset: u64,
        limit: u32,
        identity: String,
    },
    ReceivePrepare {
        path: String,
        key: String,
        source_identity: String,
        total: u64,
        conflict: String,
        mode: u32,
    },
    ReceiveChunk {
        key: String,
        offset: u64,
        data: String,
    },
    ReceiveFinalize {
        key: String,
        sha256: String,
    },
    ReceiveSymlink {
        path: String,
        target: String,
        conflict: String,
        key: String,
    },
    ListPage {
        path: String,
        offset: u64,
        limit: u32,
    },
    SetPermissions {
        path: String,
        mode: u32,
        expected_identity: Option<String>,
    },
    TransferReachability {
        destination: Device,
    },
    Transfer(TransferSpec),
    ScopedTransfer(TransferSpec),
    TransferJobs,
    TransferCancel {
        key: String,
    },
    TransferRetry {
        key: String,
    },
    /// Run a command in a managed pane shown live in a CX panel.
    CommandPanel {
        request: RunCommand,
        cols: u16,
        rows: u16,
    },
    /// Styled screen, liveness and exit status of a command panel pane.
    CapturePane {
        id: String,
        cols: u16,
        rows: u16,
    },
    /// Hex-encoded bytes to deliver to a command panel pane.
    PaneInput {
        id: String,
        hex: String,
    },
    /// Close a command panel pane, stopping the command if it is still running.
    PaneClose {
        id: String,
    },
    Network,
    NetworkCandidates,
    ProbeCandidate {
        address: String,
        interface: Option<String>,
    },
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub id: String,
    pub op: Operation,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferSpec {
    pub source: Device,
    #[serde(default)]
    pub source_container: Option<ContainerScope>,
    pub source_path: String,
    pub destination: Device,
    #[serde(default)]
    pub destination_container: Option<ContainerScope>,
    pub destination_path: String,
    pub conflict: String,
    pub key: String,
    #[serde(default)]
    pub cut: bool,
    #[serde(default)]
    pub source_identity: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_ticks: String,
    pub native_id: Option<String>,
}

impl TransferSpec {
    pub fn operation(self) -> Operation {
        if self.source_container.is_some() || self.destination_container.is_some() {
            Operation::ScopedTransfer(self)
        } else {
            Operation::Transfer(self)
        }
    }
}

fn default_true() -> bool {
    true
}
fn is_true(value: &bool) -> bool {
    *value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> Device {
        Device {
            id: "m:a".into(),
            name: "a".into(),
            target: Some("a@192.0.2.1".into()),
            account: "a".into(),
            host: "a".into(),
            status: "unknown".into(),
            observed_at: 0,
        }
    }

    /// EP-12: the edge-policy protocol changes are additive. PeerConfigure with the default `authorize`
    /// is byte-identical to what older helpers send and expect; their requests (no field) decode as
    /// authorize; PeerAuthorize needs its field, so a truncated request never decodes as a revoke.
    #[test]
    fn edge_policy_wire_changes_are_additive() {
        let configure = |authorize| Operation::PeerConfigure {
            peer: peer(),
            host_key: "h".into(),
            client_key: "c".into(),
            authorize,
        };
        let wire = serde_json::to_value(configure(true)).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"op": "peer_configure", "args": {
                "peer": serde_json::to_value(peer()).unwrap(), "host_key": "h", "client_key": "c"}})
        );
        let old: Operation = serde_json::from_value(wire).unwrap();
        assert!(matches!(
            old,
            Operation::PeerConfigure {
                authorize: true,
                ..
            }
        ));
        let one_way = serde_json::to_value(configure(false)).unwrap();
        assert_eq!(one_way["args"]["authorize"], false);
        let decoded: Operation =
            serde_json::from_value(serde_json::json!({"op": "peer_authorize", "args": {
                "client_key": "c", "authorize": false}}))
            .unwrap();
        assert!(matches!(
            decoded,
            Operation::PeerAuthorize {
                peer_id: None,
                authorize: false,
                ..
            }
        ));
        assert!(serde_json::from_value::<Operation>(
            serde_json::json!({"op": "peer_authorize", "args": {"client_key": "c"}})
        )
        .is_err());
    }
}
