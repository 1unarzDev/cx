//! Pure view-model for render concepts: no App, no IO, deterministic fixtures.
//! All hosts, addresses and secrets are synthetic.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceStatus {
    /// The viewer itself.
    Local,
    Online,
    /// Answered before, but the last probe is old or slow.
    Stale,
    Down,
    /// Never probed yet.
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    /// 0.0..=1.0
    pub cpu: f32,
    pub mem: f32,
    pub mem_total_gb: f32,
    pub temp_c: Option<f32>,
    pub load: [f32; 3],
    pub cores: u16,
    /// Last 32 samples of cpu, oldest first, 0.0..=1.0.
    pub cpu_hist: Vec<f32>,
    pub battery: Option<f32>,
    pub gpu: Option<f32>,
    pub disk: f32,
    pub uptime: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabDevice {
    pub id: String,
    pub name: String,
    /// Single-letter jump key on the hub.
    pub hotkey: char,
    /// Identity hue index (`Tok::Id`), stable for the device across views and restarts.
    pub hue: u8,
    pub os: String,
    pub account: String,
    /// Synthetic address (never a real host).
    pub host: String,
    pub status: DeviceStatus,
    /// Evidence for the status, e.g. "14ms · 3s ago", "refused · 3h".
    pub evidence: String,
    pub observed_secs: u32,
    pub metrics: Option<Metrics>,
    /// Mesh ids this device belongs to.
    pub meshes: Vec<String>,
    /// e.g. "cloudflare invite KX7-4QM", for devices that joined through a code.
    pub joined_via: Option<String>,
    /// Core devices are the user's own always-on machines.
    pub core: bool,
}

// The app fills no mesh membership or LAN evidence yet (STATUS.md); until then only the
// lab fixtures build these.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshBackend {
    CloudflareWarp,
    SshOverlay,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabMesh {
    pub id: String,
    pub name: String,
    pub backend: MeshBackend,
    pub members: Vec<String>,
    pub healthy: bool,
    pub detail: String,
}

#[cfg_attr(not(test), allow(dead_code))] // Mesh, Lan: see MeshBackend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// Plain SSH over a routed network.
    Direct,
    /// Over a mesh (mesh id).
    Mesh(String),
    /// ProxyJump through another device (device id).
    Jump(String),
    /// Same LAN segment.
    Lan,
}

/// Access policy for one direction of an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Key installed and managed automatically.
    Key,
    /// Allowed, but asks for a password.
    Password,
    /// Not allowed.
    Denied,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabEdge {
    /// "from-to"
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    /// from -> to
    pub forward: Access,
    /// to -> from
    pub reverse: Access,
    pub up: bool,
    pub rtt_ms: Option<f32>,
    /// Last 32 rtt samples, oldest first; None is a lost probe.
    pub rtt_hist: Vec<Option<f32>>,
    pub loss: f32,
}

impl LabEdge {
    pub fn one_way(&self) -> bool {
        (self.forward == Access::Denied) != (self.reverse == Access::Denied)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabNeighbor {
    /// Device whose LAN this neighbour was seen on.
    pub seen_from: String,
    pub name: String,
    pub addr: String,
    pub vendor: String,
    pub ssh_open: bool,
    pub seen_secs: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
    Shell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Working,
    AwaitingInput,
    Running,
    Idle,
    /// Nothing current says what it does (its device is unreachable, its listing old).
    Unknown,
    Exited,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabSession {
    pub id: String,
    pub title: String,
    pub device: String,
    pub provider: Provider,
    pub state: SessionState,
    pub cwd: String,
    pub command: String,
    pub last_activity_secs: u32,
    pub age_secs: u32,
    /// Output rate, last 32 samples, 0.0..=1.0.
    pub activity: Vec<f32>,
    pub clients: u8,
    pub unread: bool,
    /// Screen snapshot as SGR-colored lines (see `lab::ansi_lines`).
    pub screen: Vec<String>,
    pub screen_cols: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Dir,
    File,
    Symlink,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileNode {
    pub name: String,
    pub depth: u8,
    pub kind: FileKind,
    pub size: u64,
    pub modified_secs: u32,
    pub expanded: bool,
    /// Git status letter: M, A, ?, D.
    pub git: Option<char>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Markdown,
    Source(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabPreview {
    pub path: String,
    pub kind: PreviewKind,
    /// The first screenful; `lines` counts the whole preview.
    pub text: String,
    pub lines: usize,
    /// Why the preview is not the whole file (e.g. "first 4.0 MiB of 5.2 MiB").
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabFiles {
    pub device: String,
    pub cwd: String,
    /// Visible tree rows in display order.
    pub entries: Vec<FileNode>,
    pub selected: usize,
    pub previews: Vec<LabPreview>,
    /// Path (relative to cwd) the FilePreview view shows.
    pub preview_path: String,
    /// `entries` may be a window of a long listing: its first row's index in the
    /// listing, and the listing's length (fixtures: 0 and entries.len()).
    pub offset: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabSecret {
    pub key: String,
    pub label: String,
    /// None when missing.
    pub masked: Option<String>,
    pub source: String,
    pub updated_secs: Option<u32>,
    pub used_by: Vec<String>,
}

/// A secret being set: its name, whether the name is still being typed, and how many
/// characters of the value are typed (the value itself never reaches the view).
#[derive(Clone, Debug, PartialEq)]
pub struct LabSecretEntry {
    pub name: String,
    pub naming: bool,
    pub typed: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabInvite {
    pub code: String,
    pub oneliner: String,
    pub mesh: String,
    pub expires_secs: u32,
    /// The other addresses the invite listens on, for a device that cannot reach the one-liner's.
    pub also: Vec<String>,
    /// The QR code of the one-liner: scannable, so it carries the pairing code like the text does.
    pub qr: Vec<Vec<bool>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Warn,
    Err,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabNotice {
    pub kind: NoticeKind,
    /// The device the notice is about (its keys and tap act on it).
    pub device: Option<String>,
    pub text: String,
    pub age_secs: u32,
    pub action: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabTransfer {
    pub from: String,
    pub to: String,
    pub path: String,
    pub done: u64,
    pub total: u64,
    /// Bytes/s samples, oldest first.
    pub rate_hist: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabState {
    pub fixture: &'static str,
    /// Device id of the machine running the viewer.
    pub viewer: String,
    pub devices: Vec<LabDevice>,
    pub meshes: Vec<LabMesh>,
    pub edges: Vec<LabEdge>,
    pub neighbors: Vec<LabNeighbor>,
    pub sessions: Vec<LabSession>,
    pub files: LabFiles,
    pub secrets: Vec<LabSecret>,
    pub invite: Option<LabInvite>,
    /// The keys screen's open entry, if any.
    pub secret_entry: Option<LabSecretEntry>,
    pub notifications: Vec<LabNotice>,
    pub transfers: Vec<LabTransfer>,
    /// Palette rows from the real app (already filtered and ranked); None in fixtures,
    /// where the palette lists the fixture's devices, sessions and actions itself.
    pub palette: Option<Vec<PaletteRow>>,
}

/// One palette row the app supplies: its group, label, quiet detail, the keys that
/// reach it directly, the action a tap or `⏎` runs and the row's device (for its hue).
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteRow {
    pub group: String,
    pub label: String,
    pub detail: String,
    pub keys: String,
    pub action: String,
    pub device: Option<String>,
}

impl LabState {
    pub fn device(&self, id: &str) -> Option<&LabDevice> {
        self.devices.iter().find(|d| d.id == id)
    }
    pub fn session(&self, id: &str) -> Option<&LabSession> {
        self.sessions.iter().find(|s| s.id == id)
    }
    pub fn edge(&self, id: &str) -> Option<&LabEdge> {
        self.edges.iter().find(|e| e.id == id)
    }
    pub fn sessions_on<'a>(&'a self, device: &'a str) -> impl Iterator<Item = &'a LabSession> {
        self.sessions.iter().filter(move |s| s.device == device)
    }
    pub fn edges_of<'a>(&'a self, device: &'a str) -> impl Iterator<Item = &'a LabEdge> {
        self.edges
            .iter()
            .filter(move |e| e.from == device || e.to == device)
    }
    pub fn preview(&self) -> Option<&LabPreview> {
        self.files
            .previews
            .iter()
            .find(|p| p.path == self.files.preview_path)
    }
    /// A device's identity hue index.
    pub fn hue(&self, id: &str) -> u8 {
        self.device(id).map_or(0, |d| d.hue)
    }

    #[cfg(test)]
    pub fn fixture(name: &str) -> Option<Self> {
        match name {
            "small" => Some(small()),
            "typical" => Some(typical()),
            "large" => Some(large()),
            "mirror" => Some(mirror()),
            _ => {
                let n: usize = name.strip_prefix("nebula-")?.parse().ok()?;
                (n <= 25).then(|| nebula(n))
            }
        }
    }
}

// Deterministic lab fixtures (test-only: the lab and the view tests draw them).
#[cfg(test)]
mod fixtures;
#[cfg(test)]
pub use fixtures::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_have_requested_shape() {
        let t = typical();
        assert_eq!(t.devices.len(), 7);
        assert_eq!(t.meshes[0].members.len(), 4);
        assert_eq!(t.meshes[1].members.len(), 3);
        assert_eq!(t.sessions.len(), 5);
        assert!(t.secrets.iter().any(|s| s.masked.is_none()));
        assert!(t.edges.iter().any(LabEdge::one_way));
        assert_eq!(small().devices.len(), 3);
        let l = large();
        assert_eq!(l.devices.len(), 25);
        assert_eq!(l.sessions.len(), 40);
        for st in [small(), t, l] {
            for e in &st.edges {
                assert!(
                    st.device(&e.from).is_some() && st.device(&e.to).is_some(),
                    "{}",
                    e.id
                );
            }
            for x in &st.sessions {
                assert!(st.device(&x.device).is_some());
            }
            let mut keys: Vec<char> = st.devices.iter().map(|d| d.hotkey).collect();
            keys.sort();
            keys.dedup();
            assert_eq!(
                keys.len(),
                st.devices.len(),
                "hotkeys unique in {}",
                st.fixture
            );
            assert!(st.preview().is_some());
        }
    }
}
