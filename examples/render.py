#!/usr/bin/env python3
"""Rebuild the checked-in example presentations from the repository sources."""

from pathlib import Path
import shutil
import subprocess
import tempfile


root = Path(__file__).resolve().parent.parent
subprocess.run(["cargo", "build", "--locked"], cwd=root, check=True)
zpres = root / "target/debug/zpres"

for name in ("quickstart", "overlapping-intervals"):
    source = root / "examples" / name / "talk.zp.md"
    destination = source.parent / "rendered"
    with tempfile.TemporaryDirectory(prefix="zpres-example-") as temporary:
        staging = Path(temporary)
        subprocess.run(
            [zpres, "build", source, "--strict", "--out", staging / "html"],
            check=True,
        )
        # Publish the standalone bundle without transient generation pointers.
        generation, = (staging / "html/zpres-html-generations").glob("g-*")
        bundle = staging / "rendered"
        shutil.copytree(generation, bundle)
        (bundle / ".zpres-generation.json").unlink()
        (bundle / "presentation.html").unlink()
        subprocess.run(
            [zpres, "export", source, "--strict", "--pdf", bundle / "talk.pdf",
             "--png", staging / "pages", "--image-size", "1280x720",
             "--png-contact-sheet", bundle / "preview.png"],
            check=True,
        )
        if destination.exists():
            shutil.rmtree(destination)
        shutil.copytree(bundle, destination)
