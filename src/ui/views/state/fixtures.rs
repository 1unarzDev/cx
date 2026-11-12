//! The lab fixtures: synthetic devices, sessions, files and secrets.
use super::*;

pub const FIXTURES: [&str; 3] = ["small", "typical", "large"];

// ---------------------------------------------------------------- fixture builders

/// Deterministic pseudo-random wave in 0..=1, for history arrays.
fn wave(seed: u32, n: usize, base: f32, amp: f32) -> Vec<f32> {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(12345);
    (0..n)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let noise = (x % 1000) as f32 / 1000.0 - 0.5;
            let s = ((i as f32 + seed as f32) * 0.45).sin() * 0.5;
            (base + amp * (s + noise * 0.8)).clamp(0.0, 1.0)
        })
        .collect()
}

fn rtt(seed: u32, base: f32, jitter: f32, lost: &[usize]) -> Vec<Option<f32>> {
    wave(seed, 32, 0.5, 0.5)
        .into_iter()
        .enumerate()
        .map(|(i, v)| (!lost.contains(&i)).then_some(base + (v - 0.5) * 2.0 * jitter))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn metrics(
    seed: u32,
    cpu: f32,
    mem: f32,
    gb: f32,
    temp: Option<f32>,
    cores: u16,
    uptime: &'static str,
) -> Metrics {
    let mut cpu_hist = wave(seed, 32, cpu, 0.25);
    if let Some(last) = cpu_hist.last_mut() {
        *last = cpu;
    }
    let l = cpu * f32::from(cores);
    Metrics {
        cpu,
        mem,
        mem_total_gb: gb,
        temp_c: temp,
        load: [l, l * 0.9, l * 0.75],
        cores,
        cpu_hist,
        battery: None,
        gpu: None,
        disk: 0.5,
        uptime: uptime.into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn dev(
    id: &str,
    key: char,
    os: &str,
    status: DeviceStatus,
    evidence: &str,
    observed: u32,
    m: Option<Metrics>,
    meshes: &[&str],
    core: bool,
) -> LabDevice {
    LabDevice {
        id: id.into(),
        name: id.into(),
        hotkey: key,
        hue: 0,
        os: os.into(),
        account: "lunar".into(),
        host: format!("{id}.lab.invalid"),
        status,
        evidence: evidence.into(),
        observed_secs: observed,
        metrics: m,
        meshes: meshes.iter().map(|s| s.to_string()).collect(),
        joined_via: None,
        core,
    }
}

#[allow(clippy::too_many_arguments)]
fn edge(
    from: &str,
    to: &str,
    kind: EdgeKind,
    forward: Access,
    reverse: Access,
    up: bool,
    rtt_ms: Option<f32>,
    hist: Vec<Option<f32>>,
) -> LabEdge {
    let lost = hist.iter().filter(|v| v.is_none()).count() as f32;
    LabEdge {
        id: format!("{from}-{to}"),
        from: from.into(),
        to: to.into(),
        kind,
        forward,
        reverse,
        up,
        rtt_ms,
        loss: lost / hist.len().max(1) as f32,
        rtt_hist: hist,
    }
}

const E: &str = "\x1b[0m";

fn claude_screen(title: &str, working: bool) -> Vec<String> {
    let mut v = vec![
        format!("\x1b[38;2;224;163;122m✻\x1b[0m \x1b[1mWelcome back!\x1b[0m  \x1b[2m{title}\x1b[0m"),
        String::new(),
        "\x1b[2m> \x1b[0mmove the framed reader into transport::frame and keep the 1 MiB cap".into(),
        String::new(),
        "\x1b[38;2;143;207;157m●\x1b[0m \x1b[1mRead\x1b[0m(src/transport.rs)".into(),
        "  \x1b[2m⎿  Read 412 lines\x1b[0m".into(),
        "\x1b[38;2;143;207;157m●\x1b[0m \x1b[1mUpdate\x1b[0m(src/transport/frame.rs)".into(),
        "  \x1b[2m⎿  Updated with \x1b[0m\x1b[32m38 additions\x1b[0m\x1b[2m and \x1b[0m\x1b[31m12 removals\x1b[0m".into(),
        "     \x1b[48;2;30;60;40m+ pub fn read_frame<R: Read>(r: &mut R) -> Result<Vec<u8>> {\x1b[0m".into(),
        "     \x1b[48;2;30;60;40m+     let len = header(r)?.min(MAX_FRAME);                  \x1b[0m".into(),
        "     \x1b[48;2;70;30;30m-     let len = parse_len(&line)?;                          \x1b[0m".into(),
        "\x1b[38;2;143;207;157m●\x1b[0m \x1b[1mBash\x1b[0m(cargo test --locked transport)".into(),
        "  \x1b[2m⎿  test result: \x1b[0m\x1b[32mok\x1b[0m\x1b[2m. 41 passed; 0 failed\x1b[0m".into(),
        String::new(),
    ];
    if working {
        v.push("\x1b[38;2;224;163;122m✻ Refactoring…\x1b[0m \x1b[2m(42s · ↓ 1.9k tokens · esc to interrupt)\x1b[0m".into());
    } else {
        v.push("\x1b[2m  idle · last reply 2h ago\x1b[0m".into());
    }
    v.push("\x1b[2m╭──────────────────────────────────────────────────────────────╮\x1b[0m".into());
    v.push("\x1b[2m│\x1b[0m > \x1b[7m \x1b[0m                                                          \x1b[2m│\x1b[0m".into());
    v.push("\x1b[2m╰──────────────────────────────────────────────────────────────╯\x1b[0m".into());
    v.push("  \x1b[2m? for shortcuts\x1b[0m                               \x1b[38;2;224;163;122mopus\x1b[0m".into());
    v
}

fn codex_screen() -> Vec<String> {
    vec![
        "\x1b[1m>_ Codex\x1b[0m \x1b[2m(research preview)  ~/cx\x1b[0m".into(),
        String::new(),
        "\x1b[36muser\x1b[0m".into(),
        "fix the flaky transfer_retry test; it fails 1 in 20 under load".into(),
        String::new(),
        "\x1b[35mthinking\x1b[0m".into(),
        "\x1b[2mThe retry loop races the fake server's accept; add a ready pipe.\x1b[0m".into(),
        String::new(),
        "\x1b[35mexec\x1b[0m \x1b[1mcargo test --locked transfer_retry -- --test-threads 8\x1b[0m".into(),
        "\x1b[2m  running 3 tests ... \x1b[0m\x1b[31mFAILED\x1b[0m\x1b[2m (1/3)\x1b[0m".into(),
        String::new(),
        "\x1b[33m▌ Allow Codex to run\x1b[0m \x1b[1mpython tests/transfer_retry_pty.py --repeat 50\x1b[0m\x1b[33m?\x1b[0m".into(),
        "\x1b[33m▌\x1b[0m  \x1b[7m y \x1b[0m Yes   \x1b[1m a \x1b[0m Always   \x1b[1m n \x1b[0m No, tell Codex what to do".into(),
        String::new(),
        "\x1b[2m⏎ send   ⌃J newline   ⌃C quit                     62% context left\x1b[0m".into(),
    ]
}

fn ros_screen() -> Vec<String> {
    let mut v = vec![
        "\x1b[1mlunar@blastoise-odroid\x1b[0m:\x1b[1;34m~/ros_ws\x1b[0m$ roslaunch base bringup.launch".into(),
        "... logging to /home/lunar/.ros/log/7f3e/roslaunch-blastoise-odroid-2211.log".into(),
        "\x1b[1mstarted roslaunch server\x1b[0m http://blastoise-odroid:41233/".into(),
        String::new(),
        "SUMMARY".into(),
        "========".into(),
        " * /rosdistro: \x1b[32mnoetic\x1b[0m".into(),
        " * /base/wheel_radius: 0.0475".into(),
        String::new(),
        "NODES".into(),
        "  /base".into(),
        "    \x1b[1mmotor_driver\x1b[0m (base/motor_driver_node)".into(),
        "    \x1b[1mimu\x1b[0m (bno055/bno055_node)".into(),
        "    \x1b[1mlidar\x1b[0m (rplidar_ros/rplidarNode)".into(),
        String::new(),
    ];
    let logs = [
        (
            "INFO",
            "\x1b[0m",
            "motor_driver: serial /dev/ttyACM0 open @115200",
        ),
        (
            "INFO",
            "\x1b[0m",
            "imu: calibration sys=3 gyro=3 acc=2 mag=1",
        ),
        (
            "WARN",
            "\x1b[33m",
            "lidar: scan rate 6.8Hz below target 7.0Hz",
        ),
        ("INFO", "\x1b[0m", "motor_driver: odom publishing at 50Hz"),
        ("ERROR", "\x1b[31m", "imu: i2c read timeout (retry 1/3)"),
        ("INFO", "\x1b[0m", "imu: recovered"),
    ];
    for (i, (lvl, c, msg)) in logs.iter().enumerate() {
        v.push(format!(
            "{c}[{lvl}] [1730{:06}.{:03}]: {msg}{E}",
            481200 + i * 7,
            i * 113 % 1000
        ));
    }
    v
}

fn build_screen() -> Vec<String> {
    vec![
        "\x1b[2m[Running 'cargo build --locked']\x1b[0m".into(),
        "\x1b[1;32m   Compiling\x1b[0m cx v0.1.95 (/home/lunar/cx)".into(),
        "\x1b[1;33mwarning\x1b[0m\x1b[1m: unused variable: `peer`\x1b[0m".into(),
        "   \x1b[1;34m-->\x1b[0m src/mesh.rs:212:13".into(),
        "    \x1b[1;34m|\x1b[0m".into(),
        "\x1b[1;34m212\x1b[0m \x1b[1;34m|\x1b[0m         let peer = lookup(&id)?;".into(),
        "    \x1b[1;34m|\x1b[0m             \x1b[1;33m^^^^\x1b[0m \x1b[1;33mhelp: prefix it with an underscore: `_peer`\x1b[0m".into(),
        String::new(),
        "\x1b[1;32m    Finished\x1b[0m `dev` profile [unoptimized + debuginfo] target(s) in 7.42s".into(),
        "\x1b[2m[Finished running. Exit status: 0]\x1b[0m".into(),
        "\x1b[2m[Watching for changes in src/, Cargo.toml]\x1b[0m".into(),
    ]
}

fn shell_screen(device: &str, cmd: &str) -> Vec<String> {
    vec![
        format!("\x1b[1mlunar@{device}\x1b[0m:\x1b[1;34m~\x1b[0m$ {cmd}"),
        "ok".into(),
        format!("\x1b[1mlunar@{device}\x1b[0m:\x1b[1;34m~\x1b[0m$ "),
    ]
}

#[allow(clippy::too_many_arguments)]
fn session(
    id: &str,
    title: &str,
    device: &str,
    provider: Provider,
    state: SessionState,
    cwd: &str,
    command: &str,
    last: u32,
    age: u32,
    seed: u32,
    screen: Vec<String>,
) -> LabSession {
    let busy = match state {
        SessionState::Working | SessionState::Running => 0.6,
        SessionState::AwaitingInput => 0.15,
        _ => 0.02,
    };
    let mut activity = wave(seed, 32, busy, if busy > 0.1 { 0.4 } else { 0.03 });
    if matches!(state, SessionState::Idle | SessionState::AwaitingInput) {
        for v in activity.iter_mut().skip(20) {
            *v = 0.0;
        }
    }
    LabSession {
        id: id.into(),
        title: title.into(),
        device: device.into(),
        provider,
        state,
        cwd: cwd.into(),
        command: command.into(),
        last_activity_secs: last,
        age_secs: age,
        activity,
        clients: 0,
        unread: false,
        screen,
        screen_cols: 80,
    }
}

const README: &str = "# cx\n\nOne viewer for every shell and agent on every machine you own.\n\n## Quick start\n\n```sh\ncurl -sL https://cx.invalid/install | sh\ncx            # open the network hub\n```\n\n- **Sessions** live in tmux on each device.\n- **Files** move over the same SSH channel.\n- **Network** shows the mesh and who may reach whom.\n\n| Key | Action |\n|-----|--------|\n| `Enter` | attach |\n| `Ctrl+\\\\` | detach |\n\n> Devices join with a short code: `cx mesh invite`.\n";

const RUST_SRC: &str = "//! Framed request/response transport over SSH stdin.\nuse anyhow::{bail, Context, Result};\nuse std::io::{BufRead, Read, Write};\n\n/// Largest frame either side accepts.\npub const MAX_FRAME: usize = 1 << 20;\n\npub fn write_frame<W: Write>(w: &mut W, body: &[u8]) -> Result<()> {\n    if body.len() > MAX_FRAME {\n        bail!(\"frame of {} bytes exceeds the 1 MiB cap\", body.len());\n    }\n    write!(w, \"CX1 {}\\n\", body.len())?;\n    w.write_all(body).context(\"write frame body\")?;\n    w.flush()?;\n    Ok(())\n}\n\npub fn read_frame<R: BufRead>(r: &mut R) -> Result<Vec<u8>> {\n    let mut line = String::new();\n    r.read_line(&mut line)?;\n    let len: usize = line\n        .strip_prefix(\"CX1 \")\n        .context(\"missing CX1 header\")?\n        .trim()\n        .parse()?;\n    let mut body = vec![0; len.min(MAX_FRAME)];\n    r.read_exact(&mut body)?;\n    Ok(body)\n}\n";

fn files_fixture(device: &str) -> LabFiles {
    let f =
        |name: &str, depth: u8, kind: FileKind, size: u64, m: u32, git: Option<char>| FileNode {
            name: name.into(),
            depth,
            kind,
            size,
            modified_secs: m,
            expanded: false,
            git,
        };
    let mut entries = vec![
        f(".github", 0, FileKind::Dir, 0, 86400 * 9, None),
        f("assets", 0, FileKind::Dir, 0, 86400 * 3, None),
        f("docs", 0, FileKind::Dir, 0, 3600, None),
        f("src", 0, FileKind::Dir, 0, 120, Some('M')),
        f("sessions", 1, FileKind::Dir, 0, 7200, None),
        f("ui", 1, FileKind::Dir, 0, 120, Some('M')),
        f("main.rs", 1, FileKind::File, 18_442, 86400, None),
        f("model.rs", 1, FileKind::File, 9_210, 3600 * 5, None),
        f("transport.rs", 1, FileKind::File, 14_873, 120, Some('M')),
        f("frame.rs", 1, FileKind::File, 1_214, 60, Some('A')),
        f("tests", 0, FileKind::Dir, 0, 3600 * 2, None),
        f(".gitignore", 0, FileKind::File, 58, 86400 * 30, None),
        f("Cargo.lock", 0, FileKind::File, 61_877, 86400 * 2, None),
        f("Cargo.toml", 0, FileKind::File, 912, 86400 * 2, None),
        f("README.md", 0, FileKind::File, 4_301, 3600, Some('M')),
        f("install.sh", 0, FileKind::Symlink, 21, 86400 * 40, None),
        f("notes-unsorted.txt", 0, FileKind::File, 233, 600, Some('?')),
    ];
    entries[3].expanded = true;
    LabFiles {
        device: device.into(),
        cwd: "~/cx".into(),
        selected: 8,
        entries,
        previews: vec![
            LabPreview {
                path: "README.md".into(),
                kind: PreviewKind::Markdown,
                text: README.into(),
                lines: README.lines().count(),
                note: None,
            },
            LabPreview {
                path: "src/transport.rs".into(),
                kind: PreviewKind::Source("rust"),
                text: RUST_SRC.into(),
                lines: RUST_SRC.lines().count(),
                note: None,
            },
        ],
        preview_path: "src/transport.rs".into(),
        offset: 0,
        total: 0,
    }
}

/// QR-looking 25x25 matrix: real finder/timing patterns, pseudo-random data.
fn qr_matrix(seed: u32) -> Vec<Vec<bool>> {
    let n = 25;
    let mut m = vec![vec![false; n]; n];
    let mut x = seed | 1;
    for (r, row) in m.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *cell = x % 2 == 0;
            if r == 6 || c == 6 {
                *cell = (r + c) % 2 == 0;
            }
        }
    }
    let finder = |m: &mut Vec<Vec<bool>>, r0: usize, c0: usize| {
        for r in 0..9 {
            for c in 0..9 {
                let (rr, cc) = (r0 as i32 + r - 1, c0 as i32 + c - 1);
                if rr < 0 || cc < 0 || rr >= n as i32 || cc >= n as i32 {
                    continue;
                }
                let (dr, dc) = (r - 4, c - 4);
                let d = dr.abs().max(dc.abs());
                m[rr as usize][cc as usize] = d != 4 && d != 2;
            }
        }
    };
    finder(&mut m, 0, 0);
    finder(&mut m, 0, n - 7);
    finder(&mut m, n - 7, 0);
    // alignment pattern
    for (r, row) in m.iter_mut().enumerate().take(21).skip(16) {
        for (c, cell) in row.iter_mut().enumerate().take(21).skip(16) {
            let d = (r as i32 - 18).abs().max((c as i32 - 18).abs());
            *cell = d != 1;
        }
    }
    m
}

fn invite() -> LabInvite {
    LabInvite {
        code: "KX7-4QM".into(),
        oneliner: "curl -sL https://mesh.lab.invalid/j/KX7-4QM | sh".into(),
        mesh: "cloudflare-warp".into(),
        expires_secs: 23 * 60 + 41,
        also: Vec::new(),
        qr: qr_matrix(0x4b58_3734),
    }
}

fn secrets(all: bool) -> Vec<LabSecret> {
    let s = |key: &str,
             label: &str,
             masked: Option<&str>,
             src: &str,
             up: Option<u32>,
             used: &[&str]| {
        LabSecret {
            key: key.into(),
            label: label.into(),
            masked: masked.map(Into::into),
            source: src.into(),
            updated_secs: up,
            used_by: used.iter().map(|s| s.to_string()).collect(),
        }
    };
    let mut v = vec![s(
        "cloudflare.api_token",
        "Cloudflare API token",
        Some("cf_••••••••••••3f9a"),
        "keyring",
        Some(86400 * 12),
        &["mesh cloudflare-warp", "join codes"],
    )];
    if all {
        v.push(s(
            "claude.gateway_key",
            "Claude gateway key",
            Some("sk-gw-••••••••••7c21"),
            "keyring",
            Some(86400 * 3),
            &["cx lend (shared claude access)"],
        ));
        v.push(s(
            "codex.api_key",
            "Codex API key",
            Some("sk-••••••••••••b0d4"),
            "0600 file",
            Some(3600 * 7),
            &["codex sessions on 3 devices"],
        ));
    }
    v.push(s(
        "tailscale.auth_key",
        "Tailscale auth key",
        None,
        "-",
        None,
        &[],
    ));
    if !all {
        v.push(s(
            "claude.gateway_key",
            "Claude gateway key",
            None,
            "-",
            None,
            &[],
        ));
    }
    v
}

pub fn typical() -> LabState {
    use Access::*;
    use DeviceStatus::*;
    let cf = "cloudflare-warp";
    let so = "ssh-overlay";
    let mut robot = metrics(5, 0.71, 0.58, 4.0, Some(68.5), 8, "4d 2h");
    robot.battery = Some(0.64);
    let mut jet = metrics(6, 0.0, 0.0, 8.0, None, 6, "-");
    jet.cpu_hist = vec![0.0; 32];
    let mut guest = metrics(7, 0.22, 0.47, 16.0, Some(51.0), 8, "1h 12m");
    guest.battery = Some(0.83);
    let mut inno = metrics(2, 0.48, 0.62, 64.0, Some(61.0), 16, "11d 4h");
    inno.gpu = Some(0.35);
    let mut devices = vec![
        dev(
            "tranquility",
            't',
            "arch linux",
            Local,
            "this device",
            0,
            Some(metrics(1, 0.18, 0.41, 32.0, Some(47.0), 12, "2d 7h")),
            &[cf],
            true,
        ),
        dev(
            "innovation",
            'i',
            "arch linux",
            Online,
            "14ms · 3s ago",
            3,
            Some(inno),
            &[cf],
            true,
        ),
        dev(
            "creation",
            'c',
            "ubuntu 24.04",
            Online,
            "0.6ms · 2s ago",
            2,
            Some(metrics(3, 0.07, 0.23, 64.0, Some(39.0), 24, "38d")),
            &[cf, so],
            true,
        ),
        dev(
            "blastoise-odroid",
            'b',
            "ubuntu 20.04 arm64",
            Online,
            "via creation · 3.1ms · 4s",
            4,
            Some(robot),
            &[so],
            false,
        ),
        dev(
            "squirtle-jetson",
            's',
            "jetpack 5.1",
            Down,
            "no route · last seen 3h",
            3 * 3600,
            None,
            &[so],
            false,
        ),
        dev(
            "nas",
            'n',
            "truenas scale",
            Online,
            "1.2ms · 5s ago",
            5,
            Some(metrics(4, 0.04, 0.71, 16.0, Some(42.0), 4, "92d")),
            &[],
            true,
        ),
        dev(
            "laptop-guest",
            'p',
            "macos 15",
            Stale,
            "41ms · 2m ago",
            120,
            Some(guest),
            &[cf],
            false,
        ),
    ];
    devices[3].metrics.as_mut().unwrap().disk = 0.81;
    devices[5].metrics.as_mut().unwrap().disk = 0.67;
    devices[6].account = "guest".into();
    devices[6].joined_via = Some("cloudflare invite 7QW-2PL".into());
    let edges = vec![
        edge(
            "tranquility",
            "innovation",
            EdgeKind::Mesh(cf.into()),
            Key,
            Key,
            true,
            Some(14.2),
            rtt(11, 14.0, 4.0, &[]),
        ),
        edge(
            "tranquility",
            "creation",
            EdgeKind::Lan,
            Key,
            Key,
            true,
            Some(0.6),
            rtt(12, 0.6, 0.2, &[]),
        ),
        edge(
            "tranquility",
            "nas",
            EdgeKind::Lan,
            Password,
            Denied,
            true,
            Some(1.2),
            rtt(13, 1.2, 0.3, &[]),
        ),
        edge(
            "tranquility",
            "laptop-guest",
            EdgeKind::Mesh(cf.into()),
            Key,
            Denied,
            true,
            Some(41.0),
            rtt(14, 40.0, 18.0, &[20, 27]),
        ),
        edge(
            "tranquility",
            "blastoise-odroid",
            EdgeKind::Jump("creation".into()),
            Key,
            Denied,
            true,
            Some(3.1),
            rtt(15, 3.1, 1.0, &[]),
        ),
        edge(
            "innovation",
            "creation",
            EdgeKind::Mesh(cf.into()),
            Key,
            Key,
            true,
            Some(15.0),
            rtt(16, 15.0, 3.0, &[]),
        ),
        edge(
            "innovation",
            "laptop-guest",
            EdgeKind::Mesh(cf.into()),
            Denied,
            Denied,
            true,
            Some(38.0),
            rtt(17, 38.0, 10.0, &[]),
        ),
        edge(
            "creation",
            "blastoise-odroid",
            EdgeKind::Mesh(so.into()),
            Key,
            Key,
            true,
            Some(2.4),
            rtt(18, 2.4, 0.8, &[9]),
        ),
        edge(
            "creation",
            "squirtle-jetson",
            EdgeKind::Mesh(so.into()),
            Key,
            Key,
            false,
            None,
            {
                let mut h = rtt(19, 2.8, 0.6, &[]);
                for v in h.iter_mut().skip(22) {
                    *v = None;
                }
                h
            },
        ),
        edge(
            "creation",
            "nas",
            EdgeKind::Lan,
            Key,
            Password,
            true,
            Some(0.4),
            rtt(20, 0.4, 0.1, &[]),
        ),
    ];
    let meshes = vec![
        LabMesh {
            id: cf.into(),
            name: "cloudflare-warp".into(),
            backend: MeshBackend::CloudflareWarp,
            members: ["tranquility", "innovation", "creation", "laptop-guest"]
                .map(String::from)
                .to_vec(),
            healthy: true,
            detail: "account lunar-lab · 4 members · tunnel ok".into(),
        },
        LabMesh {
            id: so.into(),
            name: "ssh-overlay".into(),
            backend: MeshBackend::SshOverlay,
            members: ["creation", "blastoise-odroid", "squirtle-jetson"]
                .map(String::from)
                .to_vec(),
            healthy: false,
            detail: "hub creation · 3 members · 1 unreachable".into(),
        },
    ];
    let neighbors = vec![
        LabNeighbor {
            seen_from: "tranquility".into(),
            name: "pi-hole".into(),
            addr: "10.0.0.x".into(),
            vendor: "Raspberry Pi".into(),
            ssh_open: true,
            seen_secs: 40,
        },
        LabNeighbor {
            seen_from: "tranquility".into(),
            name: "printer-hp".into(),
            addr: "10.0.0.x".into(),
            vendor: "HP".into(),
            ssh_open: false,
            seen_secs: 300,
        },
        LabNeighbor {
            seen_from: "tranquility".into(),
            name: "tv-livingroom".into(),
            addr: "10.0.0.x".into(),
            vendor: "LG".into(),
            ssh_open: false,
            seen_secs: 900,
        },
        LabNeighbor {
            seen_from: "creation".into(),
            name: "lidar-ouster".into(),
            addr: "192.168.8.x".into(),
            vendor: "Ouster".into(),
            ssh_open: false,
            seen_secs: 60,
        },
    ];
    let mut sessions = vec![
        session(
            "refactor",
            "refactor transport",
            "innovation",
            Provider::Claude,
            SessionState::Working,
            "~/cx",
            "claude",
            2,
            3600 + 1200,
            31,
            claude_screen("refactor transport", true),
        ),
        session(
            "flaky",
            "fix flaky test",
            "tranquility",
            Provider::Codex,
            SessionState::AwaitingInput,
            "~/cx",
            "codex",
            95,
            1800,
            32,
            codex_screen(),
        ),
        session(
            "roscore",
            "ros core",
            "blastoise-odroid",
            Provider::Shell,
            SessionState::Running,
            "~/ros_ws",
            "roslaunch base bringup.launch",
            1,
            86400,
            33,
            ros_screen(),
        ),
        session(
            "watch",
            "build watcher",
            "creation",
            Provider::Shell,
            SessionState::Running,
            "~/cx",
            "cargo watch -x build",
            40,
            7200,
            34,
            build_screen(),
        ),
        session(
            "docs",
            "docs pass",
            "tranquility",
            Provider::Claude,
            SessionState::Idle,
            "~/cx/docs",
            "claude",
            2 * 3600 + 300,
            5 * 3600,
            35,
            claude_screen("docs pass", false),
        ),
    ];
    sessions[0].clients = 1;
    sessions[1].unread = true;
    for (i, d) in devices.iter_mut().enumerate() {
        d.hue = (i % 8) as u8;
    }
    LabState {
        fixture: "typical",
        viewer: "tranquility".into(),
        devices,
        meshes,
        edges,
        neighbors,
        sessions,
        files: files_fixture("innovation"),
        secrets: secrets(true),
        invite: Some(invite()),
        secret_entry: None,
        notifications: vec![LabNotice {
            kind: NoticeKind::Warn,
            device: Some("squirtle-jetson".into()),
            text: "squirtle-jetson unreachable via ssh-overlay for 3h".into(),
            age_secs: 3 * 3600,
            action: Some("retry".into()),
        }],
        transfers: vec![LabTransfer {
            from: "creation".into(),
            to: "tranquility".into(),
            path: "~/bags/run-0412.bag".into(),
            done: 1_610_612_736,
            total: 2_791_728_742,
            rate_hist: wave(41, 32, 0.6, 0.3)
                .into_iter()
                .map(|v| v * 48e6)
                .collect(),
        }],
        palette: None,
    }
}

pub fn small() -> LabState {
    let t = typical();
    let keep = ["tranquility", "innovation", "nas"];
    let mut s = LabState {
        fixture: "small",
        viewer: t.viewer.clone(),
        devices: t
            .devices
            .iter()
            .filter(|d| keep.contains(&d.id.as_str()))
            .cloned()
            .collect(),
        meshes: vec![LabMesh {
            members: vec!["tranquility".into(), "innovation".into()],
            detail: "account lunar-lab · 2 members · tunnel ok".into(),
            ..t.meshes[0].clone()
        }],
        edges: t
            .edges
            .iter()
            .filter(|e| keep.contains(&e.from.as_str()) && keep.contains(&e.to.as_str()))
            .cloned()
            .collect(),
        neighbors: t.neighbors[..1].to_vec(),
        sessions: t
            .sessions
            .iter()
            .filter(|x| x.id == "refactor" || x.id == "flaky")
            .cloned()
            .collect(),
        files: t.files.clone(),
        secrets: secrets(false),
        invite: None,
        secret_entry: None,
        notifications: vec![],
        transfers: vec![],
        palette: None,
    };
    for d in &mut s.devices {
        d.meshes.retain(|m| m == "cloudflare-warp");
    }
    s
}

pub fn large() -> LabState {
    use DeviceStatus::*;
    let mut s = typical();
    s.fixture = "large";
    let extra = [
        ("warehouse-picker-robot-07", Online),
        ("warehouse-picker-robot-08", Online),
        ("warehouse-picker-robot-11", Down),
        ("render-farm-a", Online),
        ("render-farm-b", Online),
        ("render-farm-c", Stale),
        ("pixel-9-termux", Online),
        ("ipad-blink", Stale),
        ("homeassistant", Online),
        ("ci-runner-arm64", Online),
        ("ci-runner-x86", Online),
        ("drone-ground-station", Unknown),
        ("lab-bench-scope", Down),
        ("charmander-rpi5", Online),
        ("bulbasaur-orin-nano", Online),
        ("studio-mac-mini", Online),
        ("old-thinkpad-x220", Down),
        ("vps-fra1", Online),
    ];
    // `e` is never a device (it is the hub's link verb); `h j k l` are vim motion, so
    // they are assigned last, only past 21 devices.
    let keys = "adfgmoqruvwxyzhjkl";
    for (i, (name, status)) in extra.iter().enumerate() {
        let seed = 100 + i as u32;
        let up = matches!(status, Online | Stale);
        let mesh = if i % 3 == 0 {
            "ssh-overlay"
        } else {
            "cloudflare-warp"
        };
        let ms = 2.0 + (i as f32 * 7.3) % 90.0;
        let mut d = dev(
            name,
            keys.chars().nth(i).unwrap_or('?'),
            if i % 2 == 0 {
                "debian 12"
            } else {
                "ubuntu 24.04"
            },
            *status,
            &match status {
                Online => format!("{ms:.0}ms · {}s ago", 2 + i % 9),
                Stale => format!("{ms:.0}ms · {}m ago", 2 + i % 7),
                Down => "refused · 1d".into(),
                _ => "not probed yet".into(),
            },
            if up { 3 } else { 86400 },
            up.then(|| {
                metrics(
                    seed,
                    (i as f32 * 0.137) % 1.0,
                    0.3 + (i as f32 * 0.07) % 0.6,
                    8.0,
                    Some(40.0 + i as f32),
                    8,
                    "3d",
                )
            }),
            &[mesh],
            false,
        );
        if name.starts_with("pixel") || name.starts_with("ipad") {
            d.account = "u0_a231".into();
        }
        d.hue = (s.devices.len() % 8) as u8;
        s.devices.push(d);
        let via = if i % 4 == 1 {
            EdgeKind::Jump("creation".into())
        } else {
            EdgeKind::Mesh(mesh.into())
        };
        let (from, fwd, rev) = if i % 5 == 4 {
            ("innovation", Access::Key, Access::Password)
        } else {
            (
                "tranquility",
                Access::Key,
                if i % 2 == 0 {
                    Access::Key
                } else {
                    Access::Denied
                },
            )
        };
        s.edges.push(edge(
            from,
            name,
            via,
            fwd,
            rev,
            up,
            up.then_some(ms),
            rtt(seed, ms, ms * 0.2, &[]),
        ));
    }
    for m in &mut s.meshes {
        m.members = s
            .devices
            .iter()
            .filter(|d| d.meshes.contains(&m.id))
            .map(|d| d.id.clone())
            .collect();
        m.detail = format!("{} members", m.members.len());
    }
    let titles = [
        "triage nightly failures",
        "bump ratatui",
        "lidar calibration",
        "migrate store to sqlite",
        "write mesh docs",
        "profile preview decode",
        "tail -f syslog",
        "htop",
        "rosbag record",
        "review PR 412 network hub",
        "fix wheel bursts",
        "sweep orphans",
        "ollama serve",
        "jupyter lab",
        "port forward 8080",
        "make -j32",
        "nvim",
        "python train.py --epochs 40",
        "deploy worker",
        "kubectl get pods -w",
        "claude: explain lockfile",
        "codex: add tests",
        "bisect regression",
        "update firmware",
        "watch nvidia-smi",
        "backup rsync",
        "psql",
        "golden screenshots",
        "cargo flamegraph",
        "investigate memory leak",
        "draft release notes",
        "ssh-audit",
        "tmux scratch",
        "ffmpeg encode",
        "ansible run",
    ];
    let ids: Vec<String> = s.devices.iter().map(|d| d.id.clone()).collect();
    for (i, t) in titles.iter().enumerate() {
        let provider = match i % 4 {
            0 => Provider::Claude,
            1 => Provider::Codex,
            _ => Provider::Shell,
        };
        let state = [
            SessionState::Working,
            SessionState::Running,
            SessionState::Idle,
            SessionState::AwaitingInput,
            SessionState::Exited,
        ][i % 5];
        let device = &ids[(i * 7) % ids.len()];
        let screen = match provider {
            Provider::Claude => claude_screen(t, state == SessionState::Working),
            Provider::Codex => codex_screen(),
            Provider::Shell => shell_screen(device, t),
        };
        s.sessions.push(session(
            &format!("s{i:02}"),
            t,
            device,
            provider,
            state,
            "~/work/a-rather-long-project-directory-name/sub",
            t,
            (i as u32 * 97) % 7200,
            86400 + i as u32 * 600,
            200 + i as u32,
            screen,
        ));
    }
    s.notifications.push(LabNotice {
        kind: NoticeKind::Err,
        device: Some("warehouse-picker-robot-11".into()),
        text: "warehouse-picker-robot-11: host key changed; refusing to connect".into(),
        age_secs: 30,
        action: Some("inspect".into()),
    });
    s
}
