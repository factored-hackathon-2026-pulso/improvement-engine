"""Exit codes and configuration errors. Messages name the piece, never values."""

from __future__ import annotations

EXIT_OK = 0
EXIT_CONFIG = 2


class RuntimeConfigError(Exception):
    """Fatal configuration problem. `code` is a stable `pulso:*` identifier; `piece` names what is wrong."""

    code = "pulso:runtime_config_invalid"

    def __init__(self, piece: str, detail: str = "") -> None:
        super().__init__(f"{self.code}: {piece}" + (f" ({detail})" if detail else ""))
        self.piece = piece


class AdapterMissing(RuntimeConfigError):
    code = "pulso:adapter_missing"


class DemoDoubleInRealMode(RuntimeConfigError):
    code = "pulso:demo_double_in_real_mode"


class PinDrift(RuntimeConfigError):
    code = "pulso:pin_symbol_drift"
