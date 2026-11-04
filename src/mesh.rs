//! `cx mesh`: runs the standalone mesh util (mesh/mesh.py, docs/mesh.md) from a copy embedded in this binary,
//! so it works wherever cx is installed; the util itself needs only python3 and no cx.
use anyhow::{Context, Result};
use std::path::PathBuf;

const FILES: &[(&str, &str)] = &[
    ("mesh.py", include_str!("../mesh/mesh.py")),
    ("worker/worker.js", include_str!("../mesh/worker/worker.js")),
    (
        "worker/schema.sql",
        include_str!("../mesh/worker/schema.sql"),
    ),
    ("worker/join.sh", include_str!("../mesh/worker/join.sh")),
    ("worker/leave.sh", include_str!("../mesh/worker/leave.sh")),
    ("worker/join.ps1", include_str!("../mesh/worker/join.ps1")),
    ("worker/leave.ps1", include_str!("../mesh/worker/leave.ps1")),
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
    let script = unpack()?;
    let status = std::process::Command::new("python3")
        .arg(&script)
        .args(args)
        .status()
        .context("cx mesh needs python3")?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    #[test]
    fn embeds_every_asset_the_util_uploads() {
        let py = super::FILES[0].1;
        for (name, _) in &super::FILES[1..] {
            let base = name.trim_start_matches("worker/");
            assert!(py.contains(base), "mesh.py does not reference {base}");
        }
    }
}
