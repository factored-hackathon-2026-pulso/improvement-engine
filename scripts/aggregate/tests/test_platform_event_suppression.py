"""EVT1 property tests for suppression: a differencing attacker over random cell tables.

The attacker knows every published cell and the partition identities of the table: signatures of one metric partition the same
population (so two signatures sharing a marginal have equal group totals) and ALL = W1 + W2. It tries to determine (a) any
single withheld cell exactly, or (b) any sum of withheld cells that would itself fail the k rule. A determined quantity is a
vector in the row space of the identity matrix restricted to the withheld cells. Pure stdlib (Fractions), seeded loops."""
import itertools
import random
import sys
import unittest
from fractions import Fraction
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import platform_event_cells as pec  # noqa: E402
import bank_cells as bc  # noqa: E402

A = ("a1", "a2")
B = ("b1", "b2")
C = ("c1", "c2")
SIGS = (("case_type", "channel"), ("language", "channel"))  # share `channel`
VALS = {"case_type": A, "channel": B, "language": ("es", "pt")}


def random_raw(rng, k):
    """Random per-half, per-window counts, aggregated into both signatures so each signature partitions the same units."""
    raw = {}
    for half in ("discovery", "holdout"):
        for window in ("W1", "W2"):
            for ct in A:
                for ch in B:
                    for lg in VALS["language"]:
                        n_units = rng.choice([0, 0, 1, 2, 3, 5, 8, 12, 20])
                        p = rng.choice([0.0, 0.1, 0.5, 0.9, 1.0])
                        nums = sum(rng.random() < p for _ in range(n_units))
                        dd = {"case_type": ct, "channel": ch, "language": lg}
                        for sig in SIGS:
                            key = tuple(sorted((d, dd[d]) for d in sig))
                            for period in ("ALL", window):
                                c = raw.setdefault(("M", key, half, period), [0, 0])
                                c[0] += nums
                                c[1] += n_units
    return {k_: tuple(v) for k_, v in raw.items()}


def identities(cells):
    """Each identity: dict key -> coefficient, equal to zero."""
    ids = []
    by = {}
    for key in cells:
        metric, dims, half, period = key
        by.setdefault((metric, half, period), {}).setdefault(tuple(d for d, _ in dims), []).append(key)
    for (metric, half, period), sigs in by.items():
        names = sorted(sigs)
        for s, s2 in itertools.combinations(names, 2):
            common = sorted(set(s) & set(s2))
            for r in range(len(common) + 1):
                for q in itertools.combinations(common, r):
                    groups = {}
                    for sig, sign in ((s, 1), (s2, -1)):
                        for key in sigs[sig]:
                            dd = dict(key[1])
                            groups.setdefault(tuple(dd[x] for x in q), {})[key] = sign
                    for g in groups.values():
                        ids.append(g)
    per = {}
    for key in cells:
        metric, dims, half, period = key
        per.setdefault((metric, dims, half), {})[period] = key
    for p in per.values():
        if len(p) == 3:
            ids.append({p["ALL"]: 1, p["W1"]: -1, p["W2"]: -1})
    return ids


def rref(rows, ncols):
    """Reduced row echelon form over Fractions: returns {pivot_col: {nonpivot_col: coef}} and the pivot set."""
    rows = [list(r) for r in rows]
    piv_rows = []
    for j in range(ncols):
        pr = next((i for i, r in enumerate(rows) if r[j] != 0 and i not in {x for x, _ in piv_rows}), None)
        if pr is None:
            continue
        f = rows[pr][j]
        rows[pr] = [x / f for x in rows[pr]]
        for i, r in enumerate(rows):
            if i != pr and r[j] != 0:
                g = r[j]
                rows[i] = [x - g * y for x, y in zip(r, rows[pr])]
        piv_rows.append((pr, j))
    pivots = {j for _, j in piv_rows}
    table = {j: {c: rows[i][c] for c in range(ncols) if c not in pivots and rows[i][c] != 0} for i, j in piv_rows}
    return table, pivots


def determined(table, pivots, subset, members):
    """Is the 0/1 indicator of `subset` in the row space? (RREF: v = sum of its pivot-column coefficients times the rows.)"""
    acc = {}
    for j in subset:
        if j in pivots:
            for c, coef in table[j].items():
                acc[c] = acc.get(c, 0) + coef
    for c in members - pivots:
        if acc.get(c, 0) != (1 if c in subset else 0):
            return False
    return True


def attack(raw, pub, k, max_subset=3):
    """Return a list of leaks (determined withheld cells / unsafe determined sums)."""
    cells = {key: list(v) for key, v in raw.items()}
    unknown = sorted(key for key in cells if key not in pub)
    idx = {key: i for i, key in enumerate(unknown)}
    rows = []
    for ident in identities(cells):
        row = [Fraction(0)] * len(unknown)
        for key, coef in ident.items():
            if key in idx:
                row[idx[key]] += coef
        if any(row):
            rows.append(row)
    if not rows:
        return []
    table, pivots = rref(rows, len(unknown))
    support = sorted({c for r in rows for c, x in enumerate(r) if x != 0})
    members = set(range(len(unknown)))
    leaks = []
    for size in range(1, min(max_subset, len(support)) + 1):
        for subset in itertools.combinations(support, size):
            sset = set(subset)
            if determined(table, pivots, sset, members):
                n = sum(cells[unknown[i]][0] for i in subset)
                d = sum(cells[unknown[i]][1] for i in subset)
                if not bc.k_ok(n, d, k):
                    leaks.append([unknown[i] for i in subset])
    return leaks


class AttackerProperty(unittest.TestCase):
    K = 3

    def test_published_cells_always_pass_k(self):
        for seed in range(150):
            raw = random_raw(random.Random(seed), self.K)
            pub, cells = pec.publish(raw, k=self.K)
            for key in pub:
                self.assertTrue(bc.k_ok(cells[key][0], cells[key][1], self.K), (seed, key))

    def test_no_withheld_cell_and_no_unsafe_sum_is_recoverable_by_differencing(self):
        checked_with_suppression = 0
        for seed in range(250):
            raw = random_raw(random.Random(1000 + seed), self.K)
            pub, cells = pec.publish(raw, k=self.K)
            if len(pub) < len(cells):
                checked_with_suppression += 1
            leaks = attack(raw, pub, self.K)
            self.assertEqual(leaks, [], f"seed {1000 + seed}: recoverable {leaks[:3]}")
        self.assertGreater(checked_with_suppression, 150)  # the property is exercised, not vacuous

    def test_sums_of_four_withheld_cells_are_not_recoverable_either(self):
        for seed in range(30):
            raw = random_raw(random.Random(5000 + seed), self.K)
            pub, _ = pec.publish(raw, k=self.K)
            self.assertEqual(attack(raw, pub, self.K, max_subset=4), [], f"seed {5000 + seed}")

    def test_the_attacker_does_find_the_leak_when_the_cross_signature_rule_is_off(self):
        """Mutation check (RED for the right reason): publishing by k alone leaks, and the oracle sees it."""
        leaked = 0
        for seed in range(250):
            raw = random_raw(random.Random(1000 + seed), self.K)
            pub = {key for key, (n, d) in raw.items() if bc.k_ok(n, d, self.K)}
            if attack(raw, pub, self.K):
                leaked += 1
        self.assertGreater(leaked, 25)

    def test_the_attacker_does_find_the_leak_when_the_window_hierarchy_is_off(self):
        orig = pec.publish
        leaked = 0
        for seed in range(250):
            raw = random_raw(random.Random(1000 + seed), self.K)
            pub_full, cells = orig(raw, k=self.K)
            # keep the cross-signature result but re-add every window cell that passes k alone
            pub = set(pub_full) | {key for key, (n, d) in raw.items() if key[3] in ("W1", "W2") and bc.k_ok(n, d, self.K)}
            if attack(raw, pub, self.K):
                leaked += 1
        self.assertGreater(leaked, 0)


class HandBuilt(unittest.TestCase):
    def test_one_withheld_cell_next_to_a_fully_published_signature_withholds_the_published_group(self):
        k = 10
        raw = {}
        # signature 1 (case_type x channel): cell (a1, b1) is small -> withheld; (a2, b1) published
        # signature 2 (language x channel): both cells published; together they would reveal (a1, b1) by subtraction
        for period in ("ALL", "W1", "W2"):
            f = 1 if period == "ALL" else 2
            raw[("M", (("case_type", "a1"), ("channel", "b1")), "discovery", period)] = (3 // f if f == 1 else 1, 6 // f)
            raw[("M", (("case_type", "a2"), ("channel", "b1")), "discovery", period)] = (60 // f, 100 // f)
            raw[("M", (("channel", "b1"), ("language", "es")), "discovery", period)] = (63 // f, 106 // f)
        pub, _ = pec.publish(raw, k=k)
        self.assertNotIn(("M", (("case_type", "a1"), ("channel", "b1")), "discovery", "ALL"), pub)
        self.assertNotIn(("M", (("channel", "b1"), ("language", "es")), "discovery", "ALL"), pub)

    def test_two_withheld_cells_with_an_unsafe_sum_cost_exactly_one_more_published_cell(self):
        k = 10
        raw = {}
        for ct, n, d in (("a1", 6, 7), ("a2", 7, 8), ("a3", 50, 100)):
            raw[("M", (("case_type", ct), ("channel", "b1")), "discovery", "ALL")] = (n, d)
        raw[("M", (("channel", "b1"), ("language", "es")), "discovery", "ALL")] = (63, 115)
        # a1 + a2 = (13, 15) fails k (complement 2) and is determined by the published language cell and a3: hide one more cell
        pub, _ = pec.publish(raw, k=k)
        self.assertEqual(len(raw) - len(pub), 3)  # a1, a2 (k) and exactly one repair
        self.assertEqual(attack(raw, pub, k), [])
        # when the withheld pair sums to a k-safe aggregate nothing else is hidden
        raw[("M", (("case_type", "a1"), ("channel", "b1")), "discovery", "ALL")] = (6, 15)
        raw[("M", (("case_type", "a2"), ("channel", "b1")), "discovery", "ALL")] = (7, 15)
        raw[("M", (("channel", "b1"), ("language", "es")), "discovery", "ALL")] = (63, 130)
        pub, _ = pec.publish(raw, k=k)
        self.assertEqual(len(raw) - len(pub), 2)
        self.assertEqual(attack(raw, pub, k), [])

    def test_a_k_safe_cell_withheld_by_the_hierarchy_may_be_recovered_it_is_not_a_leak(self):
        raw = {("M", (("case_type", "a1"), ("channel", "b1")), "discovery", p): v
               for p, v in (("ALL", (500, 1000)), ("W1", (3, 12)), ("W2", (497, 988)))}
        pub, _ = pec.publish(raw, k=10)
        self.assertEqual(pub, {("M", (("case_type", "a1"), ("channel", "b1")), "discovery", "ALL")})

    def test_a_withheld_window_withholds_its_sibling_window(self):
        raw = {("M", (("case_type", "a1"), ("channel", "b1")), "discovery", p): v
               for p, v in (("ALL", (50, 100)), ("W1", (45, 80)), ("W2", (5, 20)))}
        pub, _ = pec.publish(raw, k=10)
        self.assertIn(("M", (("case_type", "a1"), ("channel", "b1")), "discovery", "ALL"), pub)
        self.assertNotIn(("M", (("case_type", "a1"), ("channel", "b1")), "discovery", "W1"), pub)
        self.assertNotIn(("M", (("case_type", "a1"), ("channel", "b1")), "discovery", "W2"), pub)

    def test_a_withheld_all_withholds_both_windows(self):
        raw = {("M", (("case_type", "a1"), ("channel", "b1")), "discovery", p): v
               for p, v in (("ALL", (4, 100)), ("W1", (2, 50)), ("W2", (2, 50)))}
        pub, _ = pec.publish(raw, k=10)
        self.assertEqual(pub, set())


if __name__ == "__main__":
    unittest.main()
