"""Offline analysis for the 2026-09-30 T-40 provenance spike.

Reads <dir>/pN.procs.tsv (every process that joined our Job, with creation/exit FILETIME, ppid,
image, full cmdline) and <dir>/pN.raw.log (claude stdout lines with the wall clock at receipt)
and prints, per claude process:
  - every DIRECT child of claude.exe: creation time, image, cmdline shape, lifetime, subtree size,
    nearest stream events (hook_started / tool_use / task_started) around its creation, and the
    hook_response whose emission follows its exit most closely;
  - an aggregate of launcher cmdline shapes.
Usage: python analyze.py <dir> [--full]
"""
import json
import os
import re
import sys
from collections import Counter, defaultdict

USER = os.environ.get("USERNAME", "")


def red(s):
    return s.replace(USER, "<user>") if USER else s


def load_procs(path):
    rows = []
    meta = {}
    with open(path, encoding="utf-8", errors="replace") as f:
        for ln in f:
            ln = ln.rstrip("\n")
            if ln.startswith("#"):
                for kv in ln[1:].split(" "):
                    if "=" in kv:
                        k, v = kv.split("=", 1)
                        meta[k] = v
                continue
            if ln.startswith("pid\t"):
                continue
            p = ln.split("\t")
            if len(p) < 12:
                continue
            rows.append(
                dict(
                    pid=int(p[0]),
                    start=int(p[1]),
                    exit=int(p[2]),
                    ppid=int(p[3]),
                    pstart=int(p[4]),
                    injob=p[5],
                    exe=p[10],
                    cmd="\t".join(p[11:]),
                )
            )
    return meta, rows


def load_raw(path):
    ev = []
    pat = re.compile(r"^(\d+\.\d) w(\d+) (OUT|IN\[[^\]]*\]|MARK|ERR) ?(.*)$")
    with open(path, encoding="utf-8", errors="replace") as f:
        for ln in f:
            m = pat.match(ln.rstrip("\n"))
            if not m:
                continue
            t, w, kind, rest = m.groups()
            d = None
            if kind == "OUT":
                try:
                    d = json.loads(rest)
                except Exception:
                    d = {"_unparsed": rest[:200]}
            ev.append(dict(t=float(t), w=int(w), kind=kind, rest=rest, d=d))
    return ev


def base(p):
    return re.split(r"[\\/]", p)[-1] if p else ""


def ms(a, b):
    return (b - a) / 10000.0


def first_tok(c):
    c = c.lstrip()
    if c.startswith('"'):
        j = c.find('"', 1)
        return c[1:j], c[j + 1 :].lstrip()
    p = c.split(" ", 1)
    return p[0], (p[1] if len(p) > 1 else "")


def shape(cmd):
    """coarse, observable shape of a cmdline (used for claude-direct children and descendants)"""
    exe, rest = first_tok(cmd)
    b = base(exe).lower()
    if b == "bash.exe":
        if rest.startswith('-c "source ') and "shell-snapshots" in rest:
            return "BASH_TOOL: bash -c \"source <snapshot> ... && eval '<cmd>' < /dev/null && pwd -P >| <cwdfile>\""
        if rest.startswith('-c -l "SNAPSHOT_FILE='):
            return 'SNAPSHOT_MAKER: bash -c -l "SNAPSHOT_FILE=..."'
        if rest.startswith('-lc "echo'):
            return 'LOGIN_PATH_PROBE: bash -lc "echo \\"$PATH\\""'
        if rest.startswith('-c "'):
            body = rest[4:]
            return "HOOK: bash -c \"<hooks.json command>\" [" + re.sub(r"\s+", " ", body)[:60] + "]"
        return "bash.exe " + rest[:80]
    return b + " " + rest[:50]


def own_marker(r):
    """does this process's OWN cmdline say where it came from?"""
    c = r["cmd"]
    if "shell-snapshots" in c:
        return "tool-marker"
    if re.search(r"hooks[\\/][\w.-]+\.sh|claude-hook\.cmd|session-lifecycle-hook|stop-review-gate|pre-build-check|python -c", c):
        return "hook-marker"
    if "SNAPSHOT_FILE" in c or '-lc "echo' in c:
        return "snapshot-marker"
    return "none"


def ev_label(e):
    d = e["d"]
    if e["kind"] != "OUT":
        return e["kind"] + " " + e["rest"][:60]
    if not isinstance(d, dict):
        return "?"
    ty = d.get("type")
    st = d.get("subtype", "")
    if ty == "system" and st in ("hook_started", "hook_response"):
        return f"{st}({d.get('hook_name')},id={str(d.get('hook_id'))[:8]})"
    if ty == "system" and st in ("task_started", "task_notification", "task_updated", "task_progress"):
        return f"{st}(task={d.get('task_id')},tool={str(d.get('tool_use_id'))[-6:]},bg={d.get('is_backgrounded')},status={d.get('status')})"
    if ty == "assistant":
        for c in d.get("message", {}).get("content", []) or []:
            if c.get("type") == "tool_use":
                return f"assistant.tool_use(id=..{c.get('id','')[-6:]},input={json.dumps(c.get('input'))[:120]})"
        return "assistant"
    if ty == "user":
        for c in (d.get("message", {}).get("content", []) or []):
            if isinstance(c, dict) and c.get("type") == "tool_result":
                return f"user.tool_result(..{c.get('tool_use_id','')[-6:]}: {str(c.get('content'))[:80]})"
        return "user"
    if ty == "command_lifecycle":
        return f"lifecycle({d.get('state')})"
    return f"{ty}/{st}"


def interesting(e):
    if e["kind"] in ("MARK",) or e["kind"].startswith("IN["):
        return True
    d = e["d"]
    if not isinstance(d, dict):
        return False
    ty, st = d.get("type"), d.get("subtype", "")
    if ty == "system" and st in ("hook_started", "hook_response", "task_started", "task_notification", "task_updated", "task_progress"):
        return True
    if ty == "assistant" and "tool_use" in e["rest"]:
        return True
    if ty == "user" and "tool_result" in e["rest"]:
        return True
    if ty == "result" or ty == "control_response":
        return True
    return False


def main():
    d = sys.argv[1]
    full = "--full" in sys.argv
    files = sorted(f for f in os.listdir(d) if re.match(r"p\d+\.procs\.tsv$", f))
    agg = Counter()
    agg_exe = defaultdict(Counter)
    desc_tab = defaultdict(Counter)
    orph_tab = defaultdict(Counter)
    for f in files:
        pi = f.split(".")[0]
        meta, rows = load_procs(os.path.join(d, f))
        ev = load_raw(os.path.join(d, pi + ".raw.log"))
        evi = [e for e in ev if interesting(e)]
        byid = {(r["pid"], r["start"]): r for r in rows}
        claude = [r for r in rows if base(r["exe"]).lower() == "claude.exe"]
        if not claude:
            print(pi, "no claude record")
            continue
        cl = claude[0]
        kids = defaultdict(list)
        for r in rows:
            kids[(r["ppid"], r["pstart"])].append(r)

        def subtree(r):
            out = []
            st = [r]
            while st:
                x = st.pop()
                for k in kids.get((x["pid"], x["start"]), []):
                    out.append(k)
                    st.append(k)
            return out

        direct = sorted(kids[(cl["pid"], cl["start"])], key=lambda r: r["start"])
        print(f"===== {pi}: claude.exe pid={cl['pid']} direct children={len(direct)} job records={len(rows)}")
        for r in direct:
            sh = shape(r["cmd"])
            cat = sh.split(":")[0] if ":" in sh else sh.split(" ")[0]
            agg[cat] += 1
            sub = subtree(r)
            for x in sub:
                par = byid.get((x["ppid"], x["pstart"]))
                orphan = bool(par and par["exit"] and x["exit"] and par["exit"] < x["exit"]) or bool(par and par["exit"] and not x["exit"])
                desc_tab[cat][(base(x["exe"]).lower(), own_marker(x))] += 1
                if orphan:
                    orph_tab[cat][(base(x["exe"]).lower(), own_marker(x))] += 1
            if not full:
                pb = [e for e in evi if e["w"] <= r["start"]]
                pbs = f"{ms(pb[-1]['w'], r['start']):+.1f}ms after {ev_label(pb[-1])}" if pb else ""
                life = round(ms(r["start"], r["exit"])) if r["exit"] else None
                print(f"  +{ms(int(meta['start_wall']), r['start']):9.1f}ms pid={r['pid']} life={life}ms sub={len(sub)} {red(sh)[:150]} | {pbs[:120]}")
                continue
            life = ms(r["start"], r["exit"]) if r["exit"] else None
            # nearest interesting events around creation
            before = [e for e in evi if e["w"] <= r["start"]]
            after = [e for e in evi if e["w"] > r["start"]]
            pb = before[-1] if before else None
            pa = after[0] if after else None
            # hook_response right after exit
            hr = None
            if r["exit"]:
                cand = [e for e in evi if e["w"] >= r["exit"] and isinstance(e["d"], dict) and e["d"].get("subtype") == "hook_response"]
                hr = cand[0] if cand else None
            exes = Counter(base(x["exe"]).lower() or "?" for x in sub)
            for k, v in exes.items():
                agg_exe[sh][k] += v
            cmd = red(r["cmd"]).replace("<LF>", "\\n")
            cmdshow = cmd if full else (cmd[:260] + ("…" if len(cmd) > 260 else ""))
            print(
                f"  +{ms(int(meta['start_wall']), r['start']):9.1f}ms pid={r['pid']} exe={red(r['exe'])} life={life if life is None else round(life)}ms "
                f"subtree={len(sub)} maxdepth={max_depth(r, kids)} shape={sh}"
            )
            print(f"      cmd={cmdshow}")
            if pb:
                print(f"      prev-event {ms(pb['w'], r['start']):+.1f}ms after {ev_label(pb)}")
            if pa:
                print(f"      next-event {ms(r['start'], pa['w']):+.1f}ms later {ev_label(pa)}")
            if hr:
                print(f"      first hook_response after exit: {ms(r['exit'], hr['w']):+.1f}ms {ev_label(hr)}")
            print(f"      subtree exes={dict(exes)}")
        print()
    print("===== aggregate claude-direct-child categories:", dict(agg))
    print("===== descendants by launcher category: (exe, own-cmdline marker) -> count")
    for k, v in desc_tab.items():
        print("   ", k, dict(sorted(v.items(), key=lambda kv: -kv[1])))
    print("===== descendants that OUTLIVED their Windows parent (parent exited first), same keys")
    for k, v in orph_tab.items():
        print("   ", k, dict(sorted(v.items(), key=lambda kv: -kv[1])))


def max_depth(r, kids, d=0):
    ks = kids.get((r["pid"], r["start"]), [])
    if not ks:
        return d
    return max(max_depth(k, kids, d + 1) for k in ks)


if __name__ == "__main__":
    main()
