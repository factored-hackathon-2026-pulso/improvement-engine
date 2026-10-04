import sys
from pathlib import Path

DB = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(DB))
sys.path.insert(0, str(DB / "loader"))
