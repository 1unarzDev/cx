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
    python(unpack()?, args)
}

pub fn run_claude(args: &[String]) -> Result<()> {
    python(unpack()?.with_file_name("claude.py"), args)
}

fn python(script: PathBuf, args: &[String]) -> Result<()> {
    let mut command = std::process::Command::new("python3");
    command.arg(&script).args(args);
    inject_token(&mut command);
    let status = command
        .status()
        .context("cx mesh and cx claude need python3")?;
    std::process::exit(status.code().unwrap_or(1));
}

/// Hand a stored `cloudflare.api_token` (docs/secrets.md) to the util as $CLOUDFLARE_API_TOKEN, the variable it
/// already reads first, so mesh.py stays unchanged and standalone. An explicit $CLOUDFLARE_API_TOKEN wins; with no
/// stored token (or an unreachable store) the util keeps its own lookup: credentials file, then prompt.
fn inject_token(command: &mut std::process::Command) {
    if std::env::var_os("CLOUDFLARE_API_TOKEN").is_some() {
        return;
    }
    let name = crate::secrets::Kind::CloudflareApiToken.as_str();
    match crate::secrets::Secrets::open().and_then(|s| s.inject(&[name], command).map(|_| ())) {
        Ok(()) => {}
        Err(e) => {
            if crate::secrets::Secrets::open()
                .and_then(|s| s.meta(name))
                .is_ok_and(|m| m.is_some())
            {
                eprintln!(
                    "cx: stored {name} unavailable ({e:#}); mesh.py falls back to its own lookup"
                );
            }
        }
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
}
