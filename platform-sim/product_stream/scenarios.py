"""Planted-signal scenarios. All effects are functions of the event_log sequence, never of wall time."""
from __future__ import annotations

from dataclasses import dataclass, field

BASE_REASSIGN = 0.05
BASE_REOPEN = 0.06
CELLS = (("es", "app_chat"), ("es", "web_chat"), ("pt", "app_chat"), ("pt", "web_chat"))


ELEVATED_REASSIGN = 0.32
ELEVATED_REOPEN = 0.36
ONSET_FRAC = 0.30  # planted effect starts here (fraction of the horizon)
SPLIT_FRAC = 0.60  # discovery = (0, split], holdout = (split, horizon]; the effect spans BOTH when planted
VOLUME_MAX_MULT = 4.0
SCENARIO_NAMES = ("null", "escalation_rise", "recurrence_rise", "volume_drift")


@dataclass
class Scenario:
    name: str = "null"
    horizon: int = 20000
    onset_sequence: int = 0
    target_cell: tuple = ("pt", "web_chat")

    def active(self, seq: int) -> bool:
        return self.name != "null" and seq >= self.onset_sequence

    def reassign_prob(self, cell, seq: int) -> float:
        if self.name == "escalation_rise" and cell == self.target_cell and self.active(seq):
            return ELEVATED_REASSIGN
        return BASE_REASSIGN

    def reopen_prob(self, cell, seq: int) -> float:
        if self.name == "recurrence_rise" and cell == self.target_cell and self.active(seq):
            return ELEVATED_REOPEN
        return BASE_REOPEN

    def volume_mult(self, seq: int) -> float:
        if self.name == "volume_drift" and self.active(seq):
            frac = (seq - self.onset_sequence) / max(1, self.horizon - self.onset_sequence)
            return 1.0 + (VOLUME_MAX_MULT - 1.0) * min(1.0, frac)
        return 1.0

    def windows(self) -> dict:
        split = int(self.horizon * SPLIT_FRAC)
        return {"discovery": [0, split], "holdout": [split, self.horizon]}

    def planted(self) -> dict | None:
        cell = {"language": self.target_cell[0], "channel": self.target_cell[1]}
        base = {"onset_sequence": self.onset_sequence, "present_in": ["discovery", "holdout"]}
        if self.name == "escalation_rise":
            return {**base, "effect": "reassignment_rate_rise", "cell": cell, "metric":
                    "manual case.assigned (previous_staff_id set) per case opened",
                    "baseline": BASE_REASSIGN, "elevated": ELEVATED_REASSIGN}
        if self.name == "recurrence_rise":
            return {**base, "effect": "reopen_rate_rise", "cell": cell, "metric":
                    "cases with previous_case_id per case opened", "baseline": BASE_REOPEN,
                    "elevated": ELEVATED_REOPEN}
        if self.name == "volume_drift":
            return {**base, "effect": "volume_drift", "cell": None, "metric": "events per simulated hour",
                    "max_multiplier": VOLUME_MAX_MULT}
        return None


def build_scenario(name: str, horizon: int, onset_sequence: int | None = None, target_cell=("pt", "web_chat")):
    if name not in SCENARIO_NAMES:
        raise ValueError(f"unknown scenario {name!r}; choose one of {SCENARIO_NAMES}")
    onset = int(horizon * ONSET_FRAC) if onset_sequence is None else onset_sequence
    return Scenario(name=name, horizon=horizon, onset_sequence=onset, target_cell=tuple(target_cell))
