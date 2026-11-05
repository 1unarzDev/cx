//! `cx mesh`: runs the standalone mesh util (mesh/mesh.py, docs/mesh.md) from a copy embedded in this binary,
//! so it works wherever cx is installed; the util itself needs only python3 and no cx. `cx claude` runs its
//! neighbour mesh/claude.py (docs/claude.md), whose codes the same Worker serves.
use anyhow::{Context, Result};
use std::path::PathBuf;

const FILES: &[(&str, &str)] = &[
    ("mesh.py", include_str!("../mesh/mesh.py")),
    ("claude.py", include_str!("../mesh/claude.py")),
    ("worker/worker.js", include_str!("../mesh/worker/worker.js")),
    (
        "worker/schema.sql",
        include_str!("../mesh/worker/schema.sql"),
    ),
    ("worker/join.sh", include_str!("../mesh/worker/join.sh")),
    ("worker/leave.sh", include_str!("../mesh/worker/leave.sh")),
    ("worker/join.ps1", include_str!("../mesh/worker/join.ps1")),
    ("worker/leave.ps1", include_str!("../mesh/worker/leave.ps1")),
    ("worker/claude.sh", include_str!("../mesh/worker/claude.sh")),
    (
        "worker/claude.ps1",
        include_str!("../mesh/worker/claude.ps1"),
    ),
];

fn unpack() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .context("HOME is not set")?;
    let dir = base
        .join("cx")
        .join(format!("mesh-{}", env!("CARGO_PKG_VERSION")));
    for (name, body) in FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*body) {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, body)?;
        }
    }
    Ok(dir.join("mesh.py"))
}

pub fn run(args: &[String]) -> Result<()> {
    python(unpack()?, args, MESH_API)
}

pub fn run_claude(args: &[String]) -> Result<()> {
    python(unpack()?.with_file_name("claude.py"), args, CLAUDE_API)
}

/// mesh.py subcommands that call the Cloudflare API and so get the stored token. `join` and `leave` are left out
/// on purpose: they pipe a script fetched from a (possibly foreign) URL into `sh`, which inherits the environment.
const MESH_API: &[&str] = &[
    "setup", "invite", "devices", "deploy", "audit", "profile", "revoke", "teardown",
];
/// claude.py subcommands; each builds a Mesh for the Access API. claude.py runs the configured `gateway` command
/// with CLOUDFLARE_API_TOKEN removed from its environment.
const CLAUDE_API: &[&str] = &["lend", "revoke", "list"];

fn python(script: PathBuf, args: &[String], api: &[&str]) -> Result<()> {
    let mut command = std::process::Command::new("python3");
    command.arg(&script).args(args);
    if subcommand(args).is_some_and(|s| api.contains(&s)) {
        inject_token(&mut command);
    }
    let status = command
        .status()
        .context("cx mesh and cx claude need python3")?;
    std::process::exit(status.code().unwrap_or(1));
}

/// The subcommand argparse will pick, read conservatively: only the util's own leading options (`--json`,
/// `--config V`, `--config=V`) are skipped; any other leading option (an abbreviation such as `--conf`, `-h`,
/// `--`) gives None, so an argv this cannot read never gets the token.
fn subcommand(args: &[String]) -> Option<&str> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => {}
            "--config" => {
                it.next()?;
            }
            a if a.starts_with("--config=") => {}
            a if a.starts_with('-') => return None,
            a => return Some(a),
        }
    }
    None
}

/// Hand a stored `cloudflare.api_token` (docs/secrets.md) to the util as $CLOUDFLARE_API_TOKEN, the variable it
/// already reads first, so mesh.py stays standalone. An explicit $CLOUDFLARE_API_TOKEN wins; with no stored token
/// (or an unreachable store) the util keeps its own lookup: credentials file, then prompt.
fn inject_token(command: &mut std::process::Command) {
    if std::env::var_os("CLOUDFLARE_API_TOKEN").is_some() {
        return;
    }
    let name = crate::secrets::Kind::CloudflareApiToken.as_str();
    let Ok(secrets) = crate::secrets::Secrets::open() else {
        return;
    };
    if !secrets.meta(name).is_ok_and(|m| m.is_some()) {
        return;
    }
    if let Err(e) = secrets.inject(&[name], command) {
        eprintln!("cx: stored {name} unavailable ({e:#}); mesh.py falls back to its own lookup");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn embeds_every_asset_the_util_uploads() {
        let py = super::FILES[0].1;
        for (name, _) in super::FILES[2..].iter() {
            let base = name.trim_start_matches("worker/");
            assert!(py.contains(base), "mesh.py does not reference {base}");
        }
    }

    fn sub(args: &[&str]) -> Option<String> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        super::subcommand(&args).map(str::to_string)
    }

    #[test]
    fn token_goes_only_to_api_subcommands() {
        assert_eq!(sub(&["devices", "--json"]).as_deref(), Some("devices"));
        assert_eq!(
            sub(&["--json", "--config", "c.toml", "audit"]).as_deref(),
            Some("audit")
        );
        assert_eq!(sub(&["--config=c.toml", "setup"]).as_deref(), Some("setup"));
        // --config's value is never taken for the subcommand: argparse runs `join` here.
        assert_eq!(
            sub(&["--config", "devices", "join", "x"]).as_deref(),
            Some("join")
        );
        // Options argparse might expand (abbreviations) or anything unreadable: no subcommand, no token.
        assert_eq!(sub(&["--conf", "devices", "join", "x"]), None);
        assert_eq!(sub(&["-h"]), None);
        assert_eq!(sub(&["--", "devices"]), None);
        assert_eq!(sub(&["--config"]), None);
        assert_eq!(sub(&[]), None);
        for s in ["join", "leave"] {
            assert!(!super::MESH_API.contains(&s));
            assert!(!super::CLAUDE_API.contains(&s));
        }
        // Every allowlisted name is a real subcommand of the embedded utils.
        for s in super::MESH_API {
            assert!(
                super::FILES[0]
                    .1
                    .contains(&format!("sp.add_parser(\"{s}\")")),
                "{s}"
            );
        }
        for s in super::CLAUDE_API {
            assert!(
                super::FILES[1]
                    .1
                    .contains(&format!("sp.add_parser(\"{s}\"")),
                "{s}"
            );
        }
    }
}
