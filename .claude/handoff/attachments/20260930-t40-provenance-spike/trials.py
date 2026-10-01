"""Per-trial table from the harness stdout log (main.log / pilot.log).
Usage: python trials.py <log>
"""
import re
import sys

log = sys.argv[1]
lines = open(log, encoding="utf-8", errors="replace").read().splitlines()
rows = []
cur = None
for i, ln in enumerate(lines, 1):
    m = re.match(r"^TRIAL (\S+) (\S+) \| esc@send\+(\d+)ms ctrl\+(\S+) (result@esc\+(\d+)ms\(([^)]*)\)|no-result)", ln)
    if m:
        cur = dict(line=i, label=m.group(1), kind=m.group(2), res=m.group(6), sub=m.group(7), cands3="", kills="", fu="", fuline="")
        rows.append(cur)
        continue
    if cur is None:
        continue
    if ln.startswith("  [S3 "):
        c = re.search(r"TRD-cands=\[(.*?)\] outsideJob", ln)
        cur["cands3"] = c.group(1) if c else ""
    elif ln.startswith("  Q4: "):
        k = re.search(r"kills=\[(.*?)\] killStart->result=(\d+)ms", ln)
        cur["kills"] = (k.group(1) + f" ->result {k.group(2)}ms") if k else ln.strip()
    elif ln.startswith("  followUp"):
        cur["fu"] = ln.split("=", 1)[1].split(" [")[0]
        cur["fuline"] = i
    elif not ln.startswith("  "):
        cur = None
print("label | line | kind | result(ms after Esc, subtype) | S3 TRD candidates {L=launcher} | Q4 kills | follow-up (line)")
for r in rows:
    print(
        f"{r['label']} | {r['line']} | {r['kind']} | {r['res']} {r['sub']} | {r['cands3'][:220]} | {r['kills'][:200]} | {r['fu']} {('(:' + str(r['fuline']) + ')') if r['fuline'] else ''}"
    )
a = [r for r in rows if not r["label"].endswith("bg")]
h = [r for r in a if r["kind"] == "HANG"]
print(f"\nA trials={len(a)} HANG={len(h)} LATE={sum(r['kind']=='LATE' for r in a)} ok={sum(r['kind']=='ok' for r in a)}")
fu = [r["fu"] for r in h]
print("A follow-ups:", fu)
bg = [r for r in rows if r["label"].endswith("bg")]
print("bg trials:", [(r["label"], r["kind"], r["fu"]) for r in bg])
