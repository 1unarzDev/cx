#!/usr/bin/env python3
"""The mesh util offline (mesh/mesh.py, docs/mesh.md): profile excludes, the config template, the device scripts'
syntax and placeholders, and a dry-run join that changes nothing. No Cloudflare account is touched."""
import ipaddress
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "mesh"))
import mesh  # noqa: E402

W = ROOT / "mesh" / "worker"


class MeshUtil(unittest.TestCase):
    def test_mesh_is_carved_out_of_the_excludes(self):
        ex = [ipaddress.ip_network(e["address"]) for e in mesh.excludes(mesh.DEFAULT_PROFILE)]
        mesh_net = ipaddress.ip_network("100.96.0.0/12")
        self.assertFalse(any(n.version == 4 and n.overlaps(mesh_net) for n in ex))
        self.assertIn(ipaddress.ip_network("100.64.0.0/11"), ex)
        self.assertIn(ipaddress.ip_network("192.168.0.0/16"), ex)

    def test_tunneled_lans_ride_the_mesh(self):
        p = {**mesh.DEFAULT_PROFILE, "tunneled_lans": ["192.168.1.0/24"]}
        ex = [ipaddress.ip_network(e["address"]) for e in mesh.excludes(p)]
        self.assertFalse(any(n.overlaps(ipaddress.ip_network("192.168.1.0/24")) for n in ex if n.version == 4))
        self.assertIn(ipaddress.ip_network("192.168.0.0/24"), ex)

    def test_config_round_trip(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "config.toml"
            mesh.write_config(p, "demo", "acct", "")
            c = mesh.load_config(str(p))
            self.assertEqual((c["name"], c["dns_suffix"], c["profile"]["tunnel_protocol"]), ("demo", ".mesh", "masque"))
            mesh.write_config(p, "demo", "acct", "example.org")
            self.assertEqual(mesh.load_config(str(p))["dns_suffix"], ".mesh.example.org")

    def test_scripts_parse_and_carry_no_instance(self):
        for f in ("join.sh", "leave.sh"):
            subprocess.run(["sh", "-n", str(W / f)], check=True)
        for f in ("join.sh", "leave.sh", "join.ps1", "leave.ps1", "worker.js"):
            t = (W / f).read_text()
            self.assertNotIn("lunarz", t.lower(), f)
            self.assertNotRegex(t, r"\b100\.96\.\d+\.\d+\b", f)

    def test_dry_run_join_changes_nothing(self):
        body = (W / "join.sh").read_text().replace("__MESH_URL__", "https://mesh.example.org").replace("__MESH_CODE__", "abcd2345")
        env = {**os.environ, "MESH_DRY_RUN": "1", "MESH_NAME": "probe"}
        with tempfile.TemporaryDirectory() as d:
            fake = Path(d) / "warp-cli"
            fake.write_text("#!/bin/sh\nexit 1\n")
            fake.chmod(0o755)
            sudo = Path(d) / "sudo"
            sudo.write_text('#!/bin/sh\nexec "$@"\n')
            sudo.chmod(0o755)
            env["PATH"] = f"{d}:{env['PATH']}"
            r = subprocess.run(["sh", "-s"], input=body, text=True, capture_output=True, env=env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("would write /var/lib/cloudflare-warp/mdm.xml", r.stderr)
        self.assertIn("ssh ", r.stdout)


if __name__ == "__main__":
    unittest.main()
