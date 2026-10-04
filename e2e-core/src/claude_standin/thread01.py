"""E2E-THREAD-01 runner (Q1r). RED stub: every step is red until implemented."""
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class CoreHooks:
    dry_run: object = None
    run_arms: object = None
    publish: object = None
    alias_read: object = None


@dataclass
class ThreadConfig:
    workdir: Path
    exe: str
    queue_dir: Path
    mode: str = "replay"
    hooks: CoreHooks = field(default_factory=CoreHooks)
    gate_evaluators: dict | None = None


def run_thread(cfg):
    steps = [{"n": n, "id": f"step{n}", "status": "red", "error": "not implemented", "data_class": None,
              "host": "python", "detail": {}} for n in range(1, 11)]
    return {"steps": steps, "replay": {"misses": 1, "calls": 0}, "mode": cfg.mode, "host": "python", "report": {},
            "m3": {}}
