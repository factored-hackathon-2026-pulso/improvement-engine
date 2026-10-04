"""Regenerate tps_parity.json: run from roleplay-llm/ as `python ../seams/crates/engine/tests/fixtures/gen_tps_parity.py`.
Expected verdicts come from the Python scanner; engine/tests/models_tps_parity.rs asserts the Rust TPS agrees on `ok`."""
import copy, json, sys, pathlib
sys.path.insert(0, ".")
from roleplay_llm.scanner import Registry, scan_payload
from tests.test_scanner import payload, row

REG = ["sig-0001", "fam_001"]
cases = []
def add(name, p):
    r = scan_payload(p, registry=Registry(REG))
    cases.append({"name": name, "payload": p, "ok": r.ok})

add("base", payload())
for i in range(5):
    add(f"row{i}", payload([row(i, 10 + i)], step=i))
for t in ["Hola, necesito ayuda", "mail me at a@b.com", "5551234567890", "a@b@c.de", "a@b.c.", "a@.b.c", "a@@b.c", "x@y", "@b.c", "a@b.c d@e", "sig-0001", "fam_001", "ev_zz", "ev_0123456789abcdef", "g_0123456789ABCDEF", "w_2026_09", "w1234", "job-abcdef12", "binding:abcdef0123", "+56912345678", "12.345.678-9", "123456789", "1.234.567-k", "12345678", "1234567", "a b c", "abc\n"]:
    add("in:" + repr(t), payload(inputs={"x": t}))
    p = payload(); p["observations"][0]["args"] = {"q": t}; add("arg:" + repr(t), p)
    add("goal:" + repr(t), payload(goal=t))
    add("fb:" + repr(t), payload(feedback=t))
goals = ["call 600 123 4567", "600 123 4567", "600.123.4567", "600．123．4567", "600－123－4567", "６００ 123 4567",
 "a@b．com", "a＠b.com", "a﹫b.com", "a@b․com", "a@b。com", "(600) 123-4567", "1 2 3 4 5 6 7 8", "1234 567", "12 34 56 7", "1 2 3 4 5 6 7 8", "1　2　3　4　5　6　7　8",
 "①②③④⑤⑥⑦⑧", "١٢٣٤٥٦٧٨", "juan＠example.com", "x" * 2000, "x" * 2001, "name: Maria Perez", "é", "9" * 6, "1-2-3-4-5-6-7-8", "tel:+1 555 123 4567", "ab․cd", "1․2․3․4․5․6․7․8", "1（2）3－4．5．6．7．8", "1⁽2⁾3․․4567"]
for g in goals:
    add("goal:" + repr(g)[:60], payload(goal=g))
    p = payload(); p["tools"][0]["description"] = g; add("tooldesc:" + repr(g)[:60], p)
    p = payload(); p["output_schema"] = {"description": g}; add("schema:" + repr(g)[:60], p)
    p = payload(); p["output_schema"] = {g: 1}; add("schemakey:" + repr(g)[:60], p)
for v in [0, 1, 1000, 1001, -1001, 10**7, 10**7 + 1, 56912345678, 0.5, 1e9, True, None, [], {}, "", "a" * 128, "a" * 129, "A_b", "UPPER"]:
    add("val:" + repr(v)[:40], payload(inputs={"x": v}))
    add("key:" + repr(v)[:40], payload(inputs={str(v): 1}))
    add("step:" + repr(v)[:40], payload(step=v))
for k in ["Name", "name ", "a@b.com", "náme", "é", "g_segment", "", "x" * 65]:
    add("inkey:" + repr(k), payload(inputs={k: "sig-0001"}))
    p = payload(); p["observations"][0]["result"]["rows"][0][k] = "a1b2c3d4e5f60718"; add("rowkey:" + repr(k), p)
    add("topkey:" + repr(k), {**payload(), k: 1})
rows = [{"metric_id": "recurrence_rate", "window_id": "w1", "count": "<k"}, {"metric_id": "recurrence_rate", "window_id": "w1", "count": "<k", "rate": 0.5},
 {"metric_id": "m", "window_id": "w", "count": 30}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "rate": 0.333}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "rate": 1.0},
 {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "rate": 1}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "rate": True}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "rate": 0.1},
 {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "rate": 0.07}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "rate": 0.29}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10.0},
 {"metric_id": "synthetic_metric", "window_id": "w12", "count": 9}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": True}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "evidence_ref": "ev_ABCDEF01"},
 {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "evidence_ref": "ev_abcdef01"}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "g_x": "0123456789abcdef"}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "g_x": "0123456789abcdeg"},
 {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "G_x": "0123456789abcdef"}, {"metric_id": "synthetic_metric", "window_id": "w12", "count": 10, "g_x": 5}, {"window_id": "w12", "count": 10}]
for i, r in enumerate(rows):
    add(f"row-variant{i}", payload([r]))
    add(f"row-variant-k20-{i}", payload([r]))
for name, mut in {
 "obs-no-tool": lambda p: p["observations"][0].pop("tool"), "obs-bad-tool": lambda p: p["observations"][0].__setitem__("tool", "pulso/lab_query@1.0"),
 "obs-bad-tool2": lambda p: p["observations"][0].__setitem__("tool", "pulso/lab_query@1.0.0\n"), "obs-tool-multi-slash": lambda p: p["observations"][0].__setitem__("tool", "a/b/c@1.0.0"),
 "obs-tool-bigver": lambda p: p["observations"][0].__setitem__("tool", "a/b@" + "9" * 40 + ".0.0"), "obs-err-str": lambda p: p["observations"][0].__setitem__("error", "boom"),
 "obs-err-sp": lambda p: p["observations"][0].__setitem__("error", "a b"), "obs-status-bad": lambda p: p["observations"][0].__setitem__("status", "OK"),
 "obs-result-text": lambda p: p["observations"][0].__setitem__("result", {"rows": [], "text": "x"}), "obs-result-list": lambda p: p["observations"][0].__setitem__("result", []),
 "obs-result-rows-str": lambda p: p["observations"][0].__setitem__("result", {"rows": "x"}), "obs-not-list": lambda p: p.__setitem__("observations", {}),
 "tools-not-list": lambda p: p.__setitem__("tools", {}), "tools-null": lambda p: p.__setitem__("tools", None), "tools-missing": lambda p: p.pop("tools"), "obs-null": lambda p: p.__setitem__("observations", None),
 "tool-extra": lambda p: p["tools"][0].__setitem__("x", 1), "tool-no-desc": lambda p: p["tools"][0].pop("description"), "tool-no-schema": lambda p: p["tools"][0].pop("args_schema"),
 "schema-deep": lambda p: p.__setitem__("output_schema", json.loads("[" * 13 + "]" * 13)), "schema-deep12": lambda p: p.__setitem__("output_schema", json.loads("[" * 12 + "]" * 12)),
 "inputs-deep": lambda p: p.__setitem__("inputs", json.loads('{"a":' * 9 + "1" + "}" * 9)), "inputs-deep8": lambda p: p.__setitem__("inputs", json.loads('{"a":' * 7 + "1" + "}" * 7)),
 "goal-missing": lambda p: p.pop("goal"), "goal-num": lambda p: p.__setitem__("goal", 5), "inputs-str": lambda p: p.__setitem__("inputs", "x"), "feedback-none": lambda p: p.__setitem__("feedback", None),
 "feedback-501": lambda p: p.__setitem__("feedback", "x" * 501), "feedback-500": lambda p: p.__setitem__("feedback", "x" * 500), "schema-num-big": lambda p: p.__setitem__("output_schema", {"a": 10**8}),
 "schema-float-nan": lambda p: p.__setitem__("output_schema", {"a": 1e12}), "step-neg": lambda p: p.__setitem__("step", -1), "step-1001": lambda p: p.__setitem__("step", 1001),
}.items():
    p = payload(); mut(p); add(name, p)
add("not-object", [])
add("empty", {})
out = pathlib.Path(__file__).parent / "tps_parity.json"
out.write_text(json.dumps({"registry": REG, "cases": cases}, indent=0, ensure_ascii=True))
print(len(cases), sum(c["ok"] for c in cases), "ok")
