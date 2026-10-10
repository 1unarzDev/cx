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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", content = "args", rename_all = "snake_case")]
pub enum Operation {
    Info,
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
    SetLaunchShell {
        shell: String,
    },
    Sessions,
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
