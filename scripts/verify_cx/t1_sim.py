"""Monte-Carlo check of Codex's T1 estimator on SYNTHETIC cell tables (no real data).
usage: python t1_sim.py <cx_root>
null: no effect -> fraction of tables with any improved/worsened cell (family false-positive rate);
effect: -3/-6pp on one channel after the release -> detection rate; plus `inconclusive` counts."""
import random, sys, json
from collections import Counter
sys.path.insert(0, sys.argv[1])
from scripts.aggregate.outcome.outcome_estimator import estimate_outcomes
CH = ["phone", "email", "web", "branch"]

def table(rng, n, base, eff, release_idx=6, months=12):
    rows = []
    for m in range(months):
        per = f"2024-{m+1:02d}"
        for half in ("discovery", "holdout"):
            for c in CH:
                p = base + (eff if (c == "phone" and m >= release_idx) else 0.0)
                k = sum(rng.random() < p for _ in range(n))
                rows.append({"metric": "M2", "dims": {"channel": c}, "half": half, "period": per, "numerator": k, "denominator": n})
    return rows

def run(n, eff, reps, seed):
    rng = random.Random(seed)
    fam = Counter(); phone = Counter(); incon = 0
    for _ in range(reps):
        res = estimate_outcomes(table(rng, n, 0.30, eff), "2024-07")
        st = {tuple(r["dims"].values())[0]: r["status"] for r in res["cells"]}
        fam[any(s in ("improved", "worsened") for s in st.values())] += 1
        phone[st["phone"]] += 1
        incon += sum(s == "inconclusive" for s in st.values())
    return {"n_per_cell_month": n, "effect_pp": eff * 100, "reps": reps, "any_signal_in_table": fam[True] / reps,
            "phone_status": dict(phone), "inconclusive_cells_mean": incon / reps}

out = [run(n, e, 200, 1) for n in (200, 600, 2000) for e in (0.0, -0.03, -0.06)]
print(json.dumps(out, indent=1))
