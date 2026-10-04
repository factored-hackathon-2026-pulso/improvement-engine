"""Path-ownership map for OWNERS.md (G0f). Stdlib only.

OWNERS.md carries two fenced blocks, ``owners`` (lane: globs separated by
' ; ') and ``newpaths`` (planned path -> lane). The most specific glob wins
(specificity = glob length without '*'); two different lanes at the same
specificity is a tie and fails.
"""
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FENCE = "`" * 3


def block(text, name):
    m = re.search(FENCE + name + r"\n(.*?)" + FENCE, text, re.S)
    if not m:
        raise ValueError("missing block " + name)
    return m.group(1)


def conv(glob):
    out, i, depth = "", 0, 0
    while i < len(glob):
        c = glob[i]
        if glob.startswith("**", i):
            out += ".*"
            i += 2
            continue
        if c == "*":
            out += "[^/]*"
        elif c == "{":
            out += "(?:"
            depth += 1
        elif c == "}":
            out += ")"
            depth -= 1
        elif c == "," and depth:
            out += "|"
        elif c == ".":
            out += r"\."
        else:
            out += c
        i += 1
    return re.compile("^" + out + "$")


def load_rules(text):
    rules = []
    for line in block(text, "owners").splitlines():
        if not line.strip():
            continue
        lane, globs = line.split(": ", 1)
        for g in globs.split(" ; "):
            g = g.strip()
            rules.append((lane.strip(), g, conv(g), len(g.replace("*", ""))))
    return rules


def candidates(rules, path):
    return sorted(
        ((spec, lane) for lane, _g, rx, spec in rules if rx.match(path)), reverse=True
    )


def owner(rules, path):
    """Lane owning path, None if unowned, 'TIE' if two lanes tie."""
    m = candidates(rules, path)
    if not m:
        return None
    if len(m) > 1 and m[0][0] == m[1][0] and m[0][1] != m[1][1]:
        return "TIE"
    return m[0][1]


def newpaths(text):
    out = []
    for line in block(text, "newpaths").splitlines():
        if line.strip():
            p, lane = [x.strip() for x in line.split("->")]
            out.append((p, lane))
    return out


def tracked_files(root=ROOT):
    raw = subprocess.check_output(["git", "-C", str(root), "ls-files"])
    return [f for f in raw.decode("utf-8").split("\n") if f]
