"""Run either using the compiled high-performance Rust binary or fallback to Python."""
from pathlib import Path
import os
import runpy
import subprocess
import sys

bundled = Path(__file__).resolve().parent / "runtime"
root = bundled if bundled.is_dir() else Path(__file__).resolve().parents[3]

candidates = [
    Path(__file__).resolve().parent / "maimai",
    Path(__file__).resolve().parent / "bin" / "maimai",
    root / "target" / "release" / "maimai",
    root / "target" / "debug" / "maimai",
]
for candidate in candidates:
    if candidate.is_file() and os.access(candidate, os.X_OK):
        sys.exit(subprocess.run([str(candidate), *sys.argv[1:]]).returncode)

sys.path.insert(0, str(root))
runpy.run_module("maimai", run_name="__main__")
