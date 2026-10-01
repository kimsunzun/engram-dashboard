"""Hook-event <-> process correlation for the 2026-09-30 provenance spike.

For each batch of hook_started events (same hook_event, emitted within 5 ms of each other):
  - the HOOK-shaped claude-direct children (bash -c "<hooks.json command>") created in
    [first hook_started, +150 ms] -> spawn delay after hook_started, count match
  - for each such launcher: the first hook_response emitted after the launcher's exit, and whether
    that response's hook_id equals the hook_started id at the same ORDINAL (i-th started <-> i-th spawned)
Also the Bash-tool launchers vs task_started / tool_use timing.
Usage: python corr.py <dir>
"""
import os
import re
import statistics as st
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from analyze import load_procs, load_raw, base, ms, shape  # noqa: E402


def main():
    d = sys.argv[1]
    files = sorted(f for f in os.listdir(d) if re.match(r"p\d+\.procs\.tsv$", f))
    spawn_delays = []
    resp_after_exit = []
    ord_ok = ord_bad = ord_na = 0
    ua = {True: 0, False: 0}
    count_ok = count_bad = 0
    tool_rows = []
    unmatched = []
    for f in files:
        pi = f.split(".")[0]
        meta, rows = load_procs(os.path.join(d, f))
        ev = load_raw(os.path.join(d, pi + ".raw.log"))
        cl = [r for r in rows if base(r["exe"]).lower() == "claude.exe"][0]
        direct = sorted([r for r in rows if r["ppid"] == cl["pid"] and r["pstart"] == cl["start"]], key=lambda r: r["start"])
        hooks_l = [r for r in direct if shape(r["cmd"]).startswith("HOOK")]
        used = set()
        hs = [e for e in ev if isinstance(e["d"], dict) and e["d"].get("subtype") == "hook_started"]
        hr = [e for e in ev if isinstance(e["d"], dict) and e["d"].get("subtype") == "hook_response"]
        # batches
        batches = []
        for e in hs:
            if batches and e["d"].get("hook_event") == batches[-1][0]["d"].get("hook_event") and ms(batches[-1][0]["w"], e["w"]) < 5:
                batches[-1].append(e)
            else:
                batches.append([e])
        for b in batches:
            t0 = b[0]["w"]
            ls = [r for r in hooks_l if t0 - 10000 * 5 <= r["start"] <= t0 + 150 * 10000 and r["pid"] not in used]
            ls = ls[: len(b)] if len(ls) >= len(b) else ls
            for r in ls:
                used.add(r["pid"])
            if len(ls) == len(b):
                count_ok += 1
            else:
                count_bad += 1
                unmatched.append(f"{pi} {b[0]['d'].get('hook_name')} started={len(b)} launchers={len(ls)} t={b[0]['t']:.0f}")
            ids = {x["d"].get("hook_id") for x in b}
            taken = set()
            order = {r["pid"]: i for i, r in enumerate(ls)}
            for r in ls:
                spawn_delays.append(ms(t0, r["start"]))
            for r in sorted(ls, key=lambda r: r["exit"] or 1 << 62):
                i = order[r["pid"]]
                if not r["exit"]:
                    ord_na += 1
                    continue
                cand = [e for e in hr if e["w"] >= r["exit"] and e["d"].get("hook_id") in ids and e["d"].get("hook_id") not in taken]
                if not cand:
                    ord_na += 1
                    continue
                resp = cand[0]
                taken.add(resp["d"].get("hook_id"))
                resp_after_exit.append(ms(r["exit"], resp["w"]))
                # unambiguous = no other launcher of this batch exited within 10 ms, and the
                # response came within 10 ms of this exit (i.e. not a stalled hook)
                others = [abs(ms(r["exit"], x["exit"])) for x in ls if x is not r and x["exit"]]
                unamb = (not others or min(others) >= 10) and ms(r["exit"], resp["w"]) <= 10
                agree = resp["d"].get("hook_id") == b[i]["d"].get("hook_id")
                if agree:
                    ord_ok += 1
                else:
                    ord_bad += 1
                if unamb:
                    ua[agree] += 1
        # Bash tool launchers
        for r in direct:
            if shape(r["cmd"]).startswith("BASH_TOOL"):
                ts = [e for e in ev if isinstance(e["d"], dict) and e["d"].get("subtype") == "task_started"]
                tu = [e for e in ev if isinstance(e["d"], dict) and e["d"].get("type") == "assistant" and '"tool_use"' in e["rest"] and '"name":"Bash"' in e["rest"]]
                near_ts = min(ts, key=lambda e: abs(e["w"] - r["start"])) if ts else None
                prev_tu = [e for e in tu if e["w"] <= r["start"]]
                marker = "T40BG" if "T40BG=1" in r["cmd"] else ("T40FG" if "T40FG=1" in r["cmd"] else "?")
                tool_rows.append(
                    f"{pi} {marker} pid={r['pid']} life={round(ms(r['start'], r['exit'])) if r['exit'] else 'alive'}ms "
                    f"tool_use->spawn={ms(prev_tu[-1]['w'], r['start']):.0f}ms " if prev_tu else f"{pi} {marker} pid={r['pid']} "
                )
                if near_ts:
                    dd = near_ts["d"]
                    tool_rows[-1] += f"| task_started(bg={dd.get('is_backgrounded')}) at spawn{ms(r['start'], near_ts['w']):+.0f}ms"
    q = lambda xs: (f"n={len(xs)} min={min(xs):.1f} median={st.median(xs):.1f} max={max(xs):.1f}" if xs else "n=0")
    print("hook batches: launcher count == hook_started count:", count_ok, "mismatch:", count_bad)
    for u in unmatched:
        print("   mismatch:", u)
    print("spawn delay (first hook_started line received -> launcher creation), ms:", q(spawn_delays))
    print("hook_response received after launcher exit, ms:", q(resp_after_exit))
    print(f"ordinal mapping (i-th hook_started <-> i-th launcher, checked via the response after its exit): agree={ord_ok} disagree={ord_bad} n/a={ord_na}")
    print(f"  among unambiguous exits (no other batch exit within 10 ms, response <=10 ms after exit): agree={ua[True]} disagree={ua[False]}")
    print("Bash tool launchers:")
    for t in tool_rows:
        print("  ", t)


if __name__ == "__main__":
    main()
