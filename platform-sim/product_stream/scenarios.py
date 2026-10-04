"""Planted-signal scenarios. All effects are functions of the event_log sequence, never of wall time."""
from __future__ import annotations

from dataclasses import dataclass, field

BASE_REASSIGN = 0.05
BASE_REOPEN = 0.06
CELLS = (("es", "app_chat"), ("es", "web_chat"), ("pt", "app_chat"), ("pt", "web_chat"))


@dataclass
class Scenario:
    name: str = "null"
    onset_sequence: int = 0
    target_cell: tuple[str, str] = ("pt", "web_chat")
    params: dict = field(default_factory=dict)

    def reassign_prob(self, cell, seq: int) -> float:
        return BASE_REASSIGN

    def reopen_prob(self, cell, seq: int) -> float:
        return BASE_REOPEN

    def volume_mult(self, seq: int) -> float:
        return 1.0
