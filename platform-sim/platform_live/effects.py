"""Effect realism with author separation (P2py).

Author-separation rule: the simulated effect of a release is authored by an effect author from
an independent EffectSpec (baseline, delta, ramp, noise, own seed). It must not be derived from
the planted detection mechanism the judge/detector is tested against, otherwise the system would
be graded on its own answer key. Enforced three ways: (1) `simulate_effect_series` takes only an
EffectSpec, so it cannot read a planted mechanism; (2) `assert_author_separation` rejects equal
authors and any declared derivation; (3) tests prove the series is invariant to the planted
parameters. Every point is labelled data_class=simulated."""

from __future__ import annotations

import random
from dataclasses import dataclass, field


class AuthorSeparationError(Exception):
    pass


@dataclass(frozen=True)
class EffectSpec:
    effect_id: str
    author: str
    metric: str
    baseline: float
    delta_pct: float
    ramp_days: int = 1
    noise_sd: float = 0.0
    seed: int = 0
    derived_from: tuple = ()


@dataclass(frozen=True)
class PlantedMechanism:
    mechanism_id: str
    author: str
    kind: str
    params: dict = field(default_factory=dict)


def assert_author_separation(spec: EffectSpec, planted: PlantedMechanism) -> None:
    if spec.author == planted.author:
        raise AuthorSeparationError("effect author must differ from the detection author")
    if planted.mechanism_id in spec.derived_from:
        raise AuthorSeparationError("effect must not be derived from the planted detection")


def simulate_effect_series(spec: EffectSpec, days: int, release_day: int = 0) -> list[dict]:
    rng = random.Random(f"{spec.seed}:{spec.effect_id}")
    target = spec.baseline * (1 + spec.delta_pct / 100.0)
    out = []
    for d in range(days):
        frac = 0.0 if d < release_day else min(1.0, (d - release_day + 1) / max(1, spec.ramp_days))
        value = spec.baseline + (target - spec.baseline) * frac + rng.gauss(0, 1) * spec.noise_sd
        out.append({"day": d, "metric": spec.metric, "value": round(value, 6),
                    "data_class": "simulated", "effect_id": spec.effect_id})
    return out
