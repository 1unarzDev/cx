#!/usr/bin/env python3
"""Exercise the actual installer with a real binary and isolated signing root."""
import io
import json
import os
import pathlib
import re
import subprocess
import sys
import tarfile
import tempfile

binary = pathlib.Path(sys.argv[1]).resolve()
repo = pathlib.Path(__file__).resolve().parent.parent
assert os.getuid() != 0, "installer fixture must run as an ordinary user"
with tempfile.TemporaryDirectory(prefix="cx-distro-", dir=os.environ["HOME"]) as temporary:
    root = pathlib.Path(temporary)
    home = root / "home"
    home.mkdir(mode=0o700)
    key = root / "key.pem"
    pub = root / "pub.pem"
    subprocess.run(["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", str(key)], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    subprocess.run(["openssl", "pkey", "-in", str(key), "-pubout", "-out", str(pub)], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    installer = (repo / "install.sh").read_text()
    installer, replacements = re.subn(r"-----BEGIN PUBLIC KEY-----\n.*?-----END PUBLIC KEY-----", pub.read_text().strip(), installer, flags=re.S)
    assert replacements == 1, "expected one embedded installer trust root"
    script = root / "install.sh"
    script.write_text(installer)
    version = subprocess.check_output([str(binary), "--version"], universal_newlines=True).strip().split()[-1]
    archive = root / "binary.tar.gz"
    body = binary.read_bytes()
    with tarfile.open(archive, "w:gz") as packed:
        entry = tarfile.TarInfo("cx")
        entry.size = len(body)
        entry.mode = 0o755
        packed.addfile(entry, io.BytesIO(body))
    signature = root / "binary.sig"
    subprocess.run(["openssl", "dgst", "-sha256", "-sign", str(key), "-out", str(signature), str(archive)], check=True)
    tools = root / "tools"
    tools.mkdir()
    curl = tools / "curl"
    curl.write_text('''#!/usr/bin/python3
import os,pathlib,sys
args=sys.argv[1:]
url=next(arg for arg in args if arg.startswith('https://'))
assert url.startswith('https://github.com/1unarzDev/cx/releases/download/'),url
out=pathlib.Path(args[args.index('-o')+1])
source=os.environ['CX_FIXTURE_SIGNATURE'] if url.endswith('.sig') else os.environ['CX_FIXTURE_ARCHIVE']
out.write_bytes(pathlib.Path(source).read_bytes())
''')
    curl.chmod(0o755)
    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / ".local/state"),
               PATH=str(tools) + ":/usr/local/bin:/usr/bin:/bin", CX_VERSION="v" + version,
               CX_NO_LAUNCH="1", CX_INSTALL_DEPS="never", CX_FIXTURE_ARCHIVE=str(archive),
               CX_FIXTURE_SIGNATURE=str(signature))
    for name in ("CX_PREFIX", "CX_REPO"):
        env.pop(name, None)
    for _ in range(2):
        result = subprocess.run(["sh", str(script)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True, timeout=30)
        assert result.returncode == 0, (result.stdout, result.stderr)
    installed = home / ".local/bin/cx"
    assert installed.read_bytes() == body
    subprocess.run([str(installed), "--version"], env=env, check=True)
    folder = home / "files"
    folder.mkdir()
    (folder / "hello.txt").write_text("fixture\n")
    listing = subprocess.check_output([str(installed), "files", str(folder)], env=env, universal_newlines=True)
    assert "hello.txt" in listing, listing
    print(json.dumps({"result": "PASS", "version": version, "checks": ["signed real binary installation", "idempotent rerun", "version", "filesystem listing"], "trust": "synthetic test key; release authenticity tested separately"}))
