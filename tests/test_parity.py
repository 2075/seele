"""The Python scene must keep reproducing the golden frames (the Rust port's spec)."""
import json
from pathlib import Path

from seele.replay import replay

HERE = Path(__file__).parent / "parity"


def test_replay_matches_golden():
    got = replay(json.loads((HERE / "fixture.json").read_text()), str(HERE.parent.parent / "config.toml"))
    assert got == json.loads((HERE / "golden.json").read_text())
