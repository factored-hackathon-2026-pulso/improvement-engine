"""Regenerate schemas/ and event-catalog.json: python -m platform_contract"""

from . import ROOT, generate_artifacts

for rel, content in generate_artifacts().items():
    (ROOT / rel).write_text(content, encoding="utf-8")
    print("wrote", rel)
