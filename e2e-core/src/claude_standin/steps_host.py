"""E3b: Python host glue for the Rust step stand-ins (seams/crates/steps, binary `steps_cli`).

Every call goes through the FRZ0 step schema (contracts/engine-steps): the input document is validated BEFORE the
process is spawned and the output document is validated BEFORE it is returned. The Rust side stays
`semantics: claude-standin`; this module is host=python glue and never reinterprets a result.

Env contract (documented by STP1, set per call here, never inherited blindly):
  STEPS_CLI_EXE        path of steps_cli (e.g. D:/cargo-targets/claude-seams/debug/steps_cli.exe); read by from_env()
  STEPS_RUNNER_EXE     sensor runner binary (improvement-engine); STEPS_SNAPSHOT_ROOT/<snapshot id> is the package
  STEPS_LAB_DIR        recompute: <dir>/<lab id>.json = {"rows":[{signal_id, evidence_ref, numerator, count}]}
  STEPS_RECOMPUTE_DIR  validation (CLI name `intent`): <dir>/<recompute id>.json = a recompute step output
  STEPS_ARRANQUE, STEPS_MIN_SUPPORT   sensor tuning

Exit codes of the CLI: 0 ok, 1 step error (stderr), 2 usage. The semantics label is the last stderr line
`semantics: <label>`.
"""
from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

from . import ed0_detect as ed0
from . import ed0_lab as lab
from .contracts import validate_in, validate_out

CLI_NAME = {"sensors": "sensor", "recompute": "recompute", "validation": "intent", "compile": "compile", "gate": "gate"}
DEFAULT_TIMEOUT_S = 900.0


class StepsHostError(Exception):
    """Base class: any failure of a Rust step call."""


class StepsHostConfigError(StepsHostError):
    """The steps CLI (or a required exe) is not configured or does not exist."""


class StepInputInvalid(StepsHostError):
    """The input document violates the step input schema (nothing was spawned)."""


class StepOutputInvalid(StepsHostError):
    """The Rust step produced a document that violates the step output schema."""


class StepFailed(StepsHostError):
    """The Rust step exited non-zero (1 = step error, 2 = usage)."""

    def __init__(self, step: str, code: int, stderr: str):
        super().__init__(f"{step} exited {code}: {stderr.strip()[-300:]}")
        self.step, self.code, self.stderr = step, code, stderr


def validate_out_or_raise(step: str, doc: dict) -> dict:
    errs = validate_out(step, doc)
    if errs:
        raise StepOutputInvalid(f"{step} output invalid: {errs}")
    return doc


class StepsHost:
    def __init__(self, cli_exe: str | None, runner_exe: str | None = None, timeout_s: float = DEFAULT_TIMEOUT_S):
        self.cli_exe = cli_exe
        self.runner_exe = runner_exe or os.environ.get("STEPS_RUNNER_EXE") or os.environ.get("ED0_RUNNER_EXE")
        self.timeout_s = timeout_s

    @classmethod
    def from_env(cls, **kw) -> "StepsHost | None":
        exe = os.environ.get("STEPS_CLI_EXE")
        return cls(exe, **kw) if exe else None

    # ---- the one process boundary -----------------------------------------------------------------------------
    def _spawn(self, step: str, stdin_doc: dict, env_extra: dict) -> tuple[dict, str]:
        if not (self.cli_exe and os.path.exists(self.cli_exe)):
            raise StepsHostConfigError("steps CLI not found: set STEPS_CLI_EXE to the built steps_cli binary")
        env = {k: v for k, v in os.environ.items() if not k.startswith("STEPS_")}
        env.update({k: str(v) for k, v in env_extra.items()})
        done = subprocess.run([self.cli_exe, CLI_NAME[step]], input=json.dumps(stdin_doc).encode("utf-8"),
                              capture_output=True, env=env, timeout=self.timeout_s)
        err = done.stderr.decode("utf-8", "replace")
        if done.returncode != 0:
            raise StepFailed(step, done.returncode, err)
        label = next((ln.split(":", 1)[1].strip() for ln in err.splitlines() if ln.startswith("semantics:")), "")
        try:
            return json.loads(done.stdout.decode("utf-8")), label
        except ValueError as e:
            raise StepOutputInvalid(f"{step} stdout is not JSON: {type(e).__name__}") from e

    def call(self, step: str, doc: dict, env_extra: dict | None = None, stdin_doc: dict | None = None) -> tuple[dict, str]:
        """Validate `doc` against <step>.in, run the CLI, validate the result against <step>.out."""
        errs = validate_in(step, doc)
        if errs:
            raise StepInputInvalid(f"{step} input invalid: {errs}")
        out, label = self._spawn(step, stdin_doc if stdin_doc is not None else doc, env_extra or {})
        return validate_out_or_raise(step, out), label

    # ---- the five steps -----------------------------------------------------------------------------------------
    def compile(self, doc: dict) -> tuple[dict, str]:
        return self.call("compile", doc)

    def gate(self, gate_in: dict, reports: dict, world_authors: dict) -> tuple[dict, str]:
        env = {"gate_in": gate_in, "reports": reports, "world_authors": world_authors}
        return self.call("gate", gate_in, stdin_doc=env)

    def sensors(self, doc: dict, snapshot_root, arranque: int, min_support: int) -> tuple[dict, str]:
        if not (self.runner_exe and os.path.exists(self.runner_exe)):
            raise StepsHostConfigError("sensor runner exe not found (STEPS_RUNNER_EXE / ED0_RUNNER_EXE)")
        return self.call("sensors", doc, {"STEPS_RUNNER_EXE": self.runner_exe, "STEPS_SNAPSHOT_ROOT": snapshot_root,
                                          "STEPS_ARRANQUE": arranque, "STEPS_MIN_SUPPORT": min_support})

    def recompute(self, doc: dict, lab_dir) -> tuple[dict, str]:
        return self.call("recompute", doc, {"STEPS_LAB_DIR": lab_dir})

    def validation(self, doc: dict, recompute_dir) -> tuple[dict, str]:
        return self.call("validation", doc, {"STEPS_RECOMPUTE_DIR": recompute_dir})

    # ---- glue that builds the files the Rust steps read from the Python lab / package -------------------------
    def detect(self, package: str, arranque: int, min_support: int, run_id: str = "run-thread01-0001",
               cutoff_day: str = "2025-07-01") -> tuple[dict, str]:
        """Sensor over an E0-shaped package directory. Returns (detection record shaped like ed0_detect.project, label)."""
        pkg = Path(package)
        doc = {"contract_version": "engine-steps/0", "step": "sensors", "run_id": run_id, "data_class": "synthetic",
               "source_snapshot_ref": f"source_snapshot:{pkg.name}@1", "discovery_config_ref": "discovery_config:thread01@1",
               "window": {"start": "2025-04-01", "end": cutoff_day}, "metric_spec_refs": ["metric_spec:thread01@1"]}
        out, label = self.sensors(doc, pkg.parent, arranque, min_support)
        sig = out["signals"][0]
        if sig["metric_id"] != ed0.FAMILY:
            raise ValueError("sensor admitted an unexpected signal family")
        return ({"admitted_family": sig["metric_id"], "winner": sig["signal_id"], "winner_support": sig["numerator"],
                 "denominator": sig["denominator"], "discards": out["discards"],
                 "holdout_status": "checked" if sig["holdout_checked"] else None, "data_origin": "generated_sample",
                 "providers": ["local"], "producer": "rust_steps_sensor", "run_id": run_id}, label)

    def recompute_claims(self, db: str, claims: list, salt: bytes, workdir, run_id: str = "run-thread01-0001") -> dict:
        """Verifier recompute of scout claims ({hypothesis_id, evidence_ref, rate, count}) through the Rust recompute step.

        The rate recompute is the Rust step's; the integrity checks the Rust step does not own (row digest, k, claimed
        count, ref identity) stay the ED0L ones, so `ok` is the conjunction. Returns {hypothesis_id: {ok, reasons,
        recomputed, label}}. An unresolved ref is a verdict (ok False), a missing exe / spawn failure raises."""
        workdir = Path(workdir)
        lab_dir = workdir / "lab"
        lab_dir.mkdir(parents=True, exist_ok=True)
        res = {}
        for c in claims:
            py = lab.verify_claim(db, c, salt)
            row = lab._fetch(db, c.get("evidence_ref"))
            if row is None:
                res[c["hypothesis_id"]] = {**py, "label": None}
                continue
            sid = c["evidence_ref"].replace("_", "-")  # signal id = the ref (an opaque id shape both sides accept)
            lab_id = f"lab-{sid}"
            (lab_dir / f"{lab_id}.json").write_text(json.dumps({"rows": [
                {"signal_id": sid, "evidence_ref": sid, "numerator": row[4], "count": row[5]}]}), "utf-8")
            doc = {"contract_version": "engine-steps/0", "step": "recompute", "run_id": run_id, "data_class": "synthetic",
                   "lab_ref": f"lab:{lab_id}@1", "signal_ids": [sid],
                   "scout_claims": [{"signal_id": sid, "claimed_rate": c["rate"]}]}
            try:
                out, label = self.recompute(doc, lab_dir)
            except (StepInputInvalid, StepFailed) as e:  # a claim the step schema or the step refuses is not corroborated
                res[c["hypothesis_id"]] = {"ok": False, "reasons": [f"recompute step refused the claim: {type(e).__name__}"],
                                           "recomputed": py["recomputed"], "label": None}
                continue
            r = out["recomputes"][0]
            reasons = list(py["reasons"])
            if r["recomputed_rate"] != py["recomputed"]:
                reasons.append("rust recompute disagrees with the ED0L recompute")
            if not r["match"] and "claimed rate differs from recompute" not in reasons:
                reasons.append("claimed rate differs from recompute")
            res[c["hypothesis_id"]] = {"ok": bool(r["match"]) and not reasons, "reasons": reasons,
                                       "recomputed": r["recomputed_rate"], "label": label, "step_output": out}
        return res

    def validate_recompute(self, recompute_outputs: list, workdir, scout_actor: str, verifier_actor: str,
                           run_id: str = "run-thread01-0001") -> list:
        """Validation (CLI `intent`) of each recompute output: [(signal_id, verdict, label)]; corroborated needs a match."""
        workdir = Path(workdir)
        rdir = workdir / "recompute"
        rdir.mkdir(parents=True, exist_ok=True)
        out_rows = []
        for i, rc in enumerate(recompute_outputs, 1):
            rid = f"recompute-{i}"
            (rdir / f"{rid}.json").write_text(json.dumps(rc), "utf-8")
            sid = rc["recomputes"][0]["signal_id"]
            doc = {"contract_version": "engine-steps/0", "step": "validation", "run_id": run_id, "data_class": "synthetic",
                   "signal_id": sid, "recompute_ref": f"recompute:{rid}@1", "checks": ["denominator"],
                   "scout_actor": scout_actor, "verifier_actor": verifier_actor}
            out, label = self.validation(doc, rdir)
            out_rows.append((sid, out["verdict"], label))
        return out_rows
