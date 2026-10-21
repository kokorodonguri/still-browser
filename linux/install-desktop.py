#!/usr/bin/env python3
"""Register the existing portable installation for the current Linux user."""
import os
from pathlib import Path
import shlex
import shutil

root = Path(__file__).resolve().parent
bin_dir = Path.home() / ".local" / "bin"
data_dir = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share"))
bin_dir.mkdir(parents=True, exist_ok=True)
launcher = bin_dir / "still-browser"
launcher.write_text("#!/bin/sh\nexec " + shlex.quote(str(root / "still")) + ' "$@"\n')
launcher.chmod(0o755)
icons = data_dir / "icons" / "hicolor" / "scalable" / "apps"
icons.mkdir(parents=True, exist_ok=True)
shutil.copyfile(root / "profile" / "chrome" / "still.svg", icons / "still-browser.svg")
applications = data_dir / "applications"
applications.mkdir(parents=True, exist_ok=True)
exec_path = str(launcher).replace("\\", "\\\\").replace('"', '\\"').replace("`", "\\`").replace("$", "\\$").replace("%", "%%")
desktop = (root / "still-browser.desktop").read_text().replace("Exec=still-browser ", f'Exec="{exec_path}" ')
(applications / "still-browser.desktop").write_text(desktop)
print(f"Installed desktop launcher: {applications / 'still-browser.desktop'}")
print("Keep this folder in place; the launcher points to it.")
