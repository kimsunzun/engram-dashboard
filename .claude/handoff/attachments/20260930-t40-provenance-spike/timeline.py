"""Merged timeline (stream events + claude-direct-child spawns/exits) for a B window.
Usage: python timeline.py <dir> <pN> <B-fg|B-bg>"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from analyze import load_procs, load_raw, shape, base, ms, ev_label, interesting, red
d, pi, lab = sys.argv[1], sys.argv[2], sys.argv[3]
meta, rows = load_procs(os.path.join(d, pi + ".procs.tsv"))
ev = load_raw(os.path.join(d, pi + ".raw.log"))
s = [e for e in ev if e["kind"] == "MARK" and e["rest"].startswith("B-START " + lab)][0]["w"]
e_end = [e for e in ev if e["kind"] == "MARK" and e["rest"].startswith("B-END " + lab)][0]["w"]
cl = [r for r in rows if base(r["exe"]).lower() == "claude.exe"][0]
out = []
for e in ev:
    if s <= e["w"] <= e_end + 10000 * 50 and interesting(e) and e["kind"] != "MARK":
        out.append((e["w"], "EV   " + ev_label(e)[:150]))
for r in rows:
    if r["ppid"] == cl["pid"] and r["pstart"] == cl["start"] and s <= r["start"] <= e_end:
        sh = red(shape(r["cmd"]))
        out.append((r["start"], f"SPAWN pid={r['pid']} {sh[:110]}"))
        if r["exit"] and r["exit"] <= e_end + 10000 * 50:
            out.append((r["exit"], f"EXIT  pid={r['pid']} ({sh.split(':')[0][:40]})"))
for w, t in sorted(out):
    print(f"{ms(s, w):+9.1f}ms {t}")
