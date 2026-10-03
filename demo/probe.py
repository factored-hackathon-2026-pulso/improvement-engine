import sys
sys.path.insert(0, "src")
from pulso_demo import dataset, analysis

c = dataset.build()
s = analysis.scout(c)
print([(h["key"], h["rate"], h["excess_lo"]) for h in s["hypotheses"]])
v = analysis.verify(c, s["hypotheses"])
print([(a["key"], a["verdict"], a["weeks_positive"], a["weeks_observed"]) for a in v["assessments"]])
r = analysis.judge(c, analysis.first_candidate())
print(r["improvement"])
c2 = analysis.revise(c, analysis.first_candidate(), r)
print(c2, analysis.judge(c, c2)["improvement"])
