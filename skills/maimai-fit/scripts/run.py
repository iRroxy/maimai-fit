"""Run either from the repository or its self-contained installed Skill."""
from pathlib import Path
import runpy
import sys

bundled = Path(__file__).resolve().parent / "runtime"
root = bundled if bundled.is_dir() else Path(__file__).resolve().parents[3]
sys.path.insert(0, str(root))
runpy.run_module("maimai", run_name="__main__")
