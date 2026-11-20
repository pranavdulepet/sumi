"""Assemble the redistributable Sumi source, excluding notebooks and build state."""
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

root = Path(__file__).resolve().parents[1]
files = [
    "README.md", "CONTRIBUTING.md", "LICENSE", ".gitignore", "index.html", "package.json",
    "package-lock.json", "tsconfig.json", "playwright.config.ts", "rust-toolchain.toml",
    "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/build.rs",
    "src-tauri/runtime/LICENSE", "src-tauri/native.m", "src-tauri/tauri.conf.json", "website/index.html",
    "website/shippori.woff2", "website/zen.woff2",
]
folders = [
    "catalog", "src", "public", "scripts", "tests", "prompts", "licenses", "docs",
    "src-tauri/src", "src-tauri/capabilities", "src-tauri/icons",
]
paths = [root / p for p in files]
for name in folders:
    paths.extend(p for p in (root / name).rglob("*") if p.is_file() and not p.name.startswith("."))
target = root / "release/Sumi-source.zip"
target.parent.mkdir(exist_ok=True)
with ZipFile(target, "w", compression=ZIP_DEFLATED) as archive:
    for p in sorted(paths):
        archive.write(p, Path("Sumi") / p.relative_to(root))
with ZipFile(target) as archive:
    if archive.testzip():
        raise SystemExit("Source archive integrity check failed")
print(target)
