"""SMAP stub (RED)."""


def select_target(finding, catalogue):
    return catalogue["entries"][0]["target_ref"] if catalogue else None
