#!/usr/bin/env python3
"""Stamp a built Apple bundle from the single release manifest."""
import json, plistlib, sys
from pathlib import Path
release = json.loads((Path(__file__).resolve().parent.parent / "release.json").read_text())
path = Path(sys.argv[1])
data = plistlib.loads(path.read_bytes())
data["CFBundleShortVersionString"] = release["version"]
data["CFBundleVersion"] = str(release["build"])
path.write_bytes(plistlib.dumps(data))
