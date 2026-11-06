#!/usr/bin/env python3
"""Collect release notices from the locked Cargo and npm packages."""

import json
from pathlib import Path
import subprocess
import sys


root = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=root
))
sections = ["File Peek - third-party license notices\n\n" + (root / "LICENSE").read_text()]
covered = 0
for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
    if package["source"] is None:
        continue
    directory = Path(package["manifest_path"]).parent
    files = sorted(path for path in directory.iterdir()
                   if path.is_file() and path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE")))
    if not files and package.get("repository") == "https://github.com/madsmtm/objc2":
        revision = json.loads((directory / ".cargo_vcs_info.json").read_text())["git"]["sha1"]
        files = [root / "licenses" / "upstream" / f"objc2-{revision}.md",
                 root / "licenses" / "upstream" / "objc2-MIT.txt"]
    if not files or any(not path.is_file() for path in files):
        raise SystemExit(f"Missing license notice for {package['name']} {package['version']}")
    heading = f"{package['name']} {package['version']} ({package.get('license', 'see license text')})"
    sections.append(heading + "\n\n" + "\n\n".join(path.read_text(errors="replace") for path in files))
    covered += 1

dompurify = root / "node_modules" / "dompurify"
version = json.loads((dompurify / "package.json").read_text())["version"]
sections.append(f"DOMPurify {version}\n\n" + (dompurify / "LICENSE").read_text())
output = Path(sys.argv[1])
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text("\n\n" + ("\n\n" + "=" * 72 + "\n\n").join(sections), encoding="utf-8")
assert covered == len(metadata["packages"]) - 1, "Every external Cargo package must have a notice"
assert "DOMPurify" in output.read_text(), "DOMPurify must be included"
print(f"Bundled license notices for {covered} Cargo packages and DOMPurify {version}")
