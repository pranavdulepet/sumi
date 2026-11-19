"""Collect dependency notices for the Mac distribution from the locked build."""
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--offline", "--filter-platform", "aarch64-apple-darwin",
    "--format-version", "1", "--manifest-path", str(root / "src-tauri/Cargo.toml"),
]))
index = []
texts = defaultdict(list)

def include(package, folder, extra=()):
    names = ("license", "licence", "copying", "notice", "copyright", "unlicense")
    paths = [p for p in folder.iterdir() if p.name.lower().startswith(names) and p.is_file()]
    paths.extend(folder / p for p in extra)
    for p in sorted(set(paths)):
        texts[p.read_text(errors="replace").strip()].append(f"{package} / {p.relative_to(folder)}")

for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    name, version = package["name"], package["version"]
    if name == "sumi":
        continue
    label = f"{name} {version}"
    index.append(f"{label}  {package['license'] or package['license_file']}  https://crates.io/crates/{name}/{version}")
    extra = {"libsqlite3-sys": ["sqlcipher/LICENSE"], "openssl-src": ["openssl/LICENSE.txt"]}.get(name, [])
    if package.get("license_file"):
        extra = [*extra, package["license_file"]]
    include(label, Path(package["manifest_path"]).parent, extra)
for name in ("react", "react-dom", "scheduler", "@tauri-apps/api"):
    folder = root / "node_modules" / name
    package = json.loads((folder / "package.json").read_text())
    label = f"{name} {package['version']}"
    index.append(f"{label}  {package.get('license', '')}  https://www.npmjs.com/package/{name}/v/{package['version']}")
    include(label, folder)

parts = ["Sumi dependency notices", "\n".join(index)]
for content, labels in texts.items():
    parts.extend(["\n".join(labels), content])
Path(sys.argv[1]).write_text("\n\n".join(parts) + "\n")
