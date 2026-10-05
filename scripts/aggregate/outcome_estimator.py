"""Backward-compatible entrypoint for the canonical T1 outcome estimator.

Use ``python -m scripts.aggregate.outcome.outcome_estimator`` for new callers.
This module preserves the original import and ``-m`` paths while routing both
to the same v3 implementation.
"""

import sys

from scripts.aggregate.outcome import outcome_estimator as _implementation


if __name__ == "__main__":
    sys.exit(_implementation.main())

# Keep legacy imports and monkeypatch-based tests attached to the canonical
# module object, not a second set of estimator globals.
sys.modules[__name__] = _implementation
