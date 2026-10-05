#!/usr/bin/env python3
"""OUT1 scripted estimator double (tests): prints the canned JSON verdict, logs how it was called. Not an estimator."""
import argparse
import json
import sys

p = argparse.ArgumentParser()
p.add_argument("--canned", required=True)
p.add_argument("--log")
p.add_argument("--fail", type=int, default=0, help="exit with this code instead of answering")
p.add_argument("--cells")
p.add_argument("--treated")
p.add_argument("--release-date")
p.add_argument("--window-months", type=int)
a = p.parse_args()
if a.log:
    with open(a.log, "a", encoding="utf-8") as f:
        f.write(json.dumps({"cells": a.cells, "treated": a.treated, "release_date": a.release_date, "window_months": a.window_months}) + "\n")
if a.fail:
    print("scripted failure", file=sys.stderr)
    sys.exit(a.fail)
sys.stdout.write(open(a.canned, encoding="utf-8").read())
