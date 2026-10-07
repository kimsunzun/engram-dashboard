# 아키텍처 지도 그림 생성기 — 출력(out-*)을 architecture-map.html 해당 자리에 붙여 넣는다. 원자료 = ../tags.json (그래프 숫자는 notes 3장에서 손으로 옮김).
import json, collections, html

import os
SP = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..') + '/'
ORDER = ['AGENT', 'COMM', 'MAIL', 'CMD', 'SCREEN', 'RENDER', 'DAEMON_LIFE', 'AUTH', 'ETC']
NAME = {'AGENT': '에이전트', 'COMM': '통신', 'MAIL': '우편', 'CMD': '커맨드', 'SCREEN': '화면',
        'RENDER': '렌더링', 'DAEMON_LIFE': '데몬 수명', 'AUTH': '인증', 'ETC': '기타(바닥·조립)'}
VAR = {'AGENT': 'd-agent', 'COMM': 'd-comm', 'MAIL': 'd-mail', 'CMD': 'd-cmd', 'SCREEN': 'd-screen',
       'RENDER': 'd-render', 'DAEMON_LIFE': 'd-life', 'AUTH': 'd-auth', 'ETC': 'd-etc'}

def bar_of(f):
    p = f.split('/')
    if p[0] == 'crates':
        c = p[1].replace('engram-dashboard-', '')
        if c != 'daemon':
            return ('하위 crate (crate 통째)', c)
        rest = p[3:]
        m = rest[0].replace('.rs', '')
        if m == 'bin':
            return ('데몬 crate', 'bin/engram (CLI)')
        if m in ('status_fanout', 'main'):
            return ('데몬 crate', 'status_fanout · main')
        return ('데몬 crate', m)
    if p[0] == 'src-tauri':
        m = p[2].replace('.rs', '')
        if m in ('output_router', 'output_channel'):
            return ('셸 crate', 'output_router · channel')
        if m in ('lib', 'main'):
            return ('셸 crate', 'lib · main')
        return ('셸 crate', m)
    rest = p[1:]
    if len(rest) > 1 and rest[0] == 'components' and rest[1] in ('slot', 'layout', 'agent'):
        return ('프론트 폴더', 'components/' + rest[1])
    if len(rest) > 1 and rest[0] in ('api', 'commands', 'store'):
        return ('프론트 폴더', rest[0])
    return ('프론트 폴더', '나머지 (i18n·ui·theme·pages…)')

bars = collections.defaultdict(collections.Counter)
for fn in ('tags.json',):
    for e in json.load(open(SP + fn, encoding='utf-8'))['files']:
        k = bar_of(e['file'])
        segs = [(r['domain'], r['to'] - r['from'] + 1) for r in e['ranges']] if e['domain'] == 'MIXED' else [(e['domain'], e['lines'])]
        for d, n in segs:
            bars[k]['ETC' if d in ('BASE', 'ROOT') else d] += n

GROUPS = ['데몬 crate', '셸 crate', '하위 crate (crate 통째)', '프론트 폴더']
LX, BX, BW, TX = 16, 210, 560, 782
ROW, BH, GH = 26, 16, 34
rows = []
for g in GROUPS:
    items = [(k[1], c) for k, c in bars.items() if k[0] == g]
    items.sort(key=lambda it: -sum(it[1].values()))
    if g == '프론트 폴더':
        items.sort(key=lambda it: (it[0].startswith('나머지'), -sum(it[1].values())))
    rows.append(('G', g, None))
    rows += [('B', lab, c) for lab, c in items]

legend_h = 44
H = legend_h + sum(GH if r[0] == 'G' else ROW for r in rows) + 8
out = []
out.append(f'<svg viewBox="0 0 900 {H}" role="img" aria-label="모듈별 도메인 구성. 막대 하나가 모듈 하나이고, 색 구간이 그 모듈 안 도메인별 줄 수 비율이다.">')
# legend
x = LX
for d in ORDER:
    out.append(f'<rect x="{x}" y="12" width="12" height="12" rx="2" style="fill: var(--{VAR[d]})"/>')
    out.append(f'<text x="{x + 17}" y="22.5" font-size="12" fill="currentColor">{NAME[d]}</text>')
    x += 17 + len(NAME[d]) * 12.5 + 18
y = legend_h
for kind, lab, c in rows:
    if kind == 'G':
        out.append(f'<text x="{LX}" y="{y + 24}" font-size="13" font-weight="700" fill="currentColor">{html.escape(lab)}</text>')
        out.append(f'<line x1="{LX}" y1="{y + 30}" x2="884" y2="{y + 30}" style="stroke: var(--line)" stroke-width="1"/>')
        y += GH
        continue
    tot = sum(c.values())
    feat = [d for d in ORDER if d != 'ETC' and (c.get(d, 0) >= 80 or c.get(d, 0) >= 0.15 * tot)]
    cy = y + (ROW - BH) / 2
    out.append(f'<text x="{BX - 10}" y="{cy + 12}" text-anchor="end" font-size="12" font-family="Consolas, monospace" fill="currentColor">{html.escape(lab)}</text>')
    bx = BX
    for d in ORDER:
        n = c.get(d, 0)
        if not n:
            continue
        w = BW * n / tot
        ww = w - 2 if w > 4 else max(w, 1)
        pct = round(100 * n / tot)
        out.append(f'<rect x="{bx:.1f}" y="{cy}" width="{ww:.1f}" height="{BH}" rx="2" style="fill: var(--{VAR[d]})"><title>{html.escape(lab)} — {NAME[d]} {n:,}줄 ({pct}%)</title></rect>')
        bx += w
    tag = f' · 도메인 {len(feat)}' if len(feat) >= 2 else ''
    weight = ' font-weight="700"' if len(feat) >= 3 else ''
    out.append(f'<text x="{TX}" y="{cy + 12}" font-size="12" style="fill: var(--muted)"{weight}>{tot:,}줄{tag}</text>')
    y += ROW
out.append('</svg>')
open(SP + 'tools/out-bars.svg', 'w', encoding='utf-8').write('\n'.join(out))

# table view (html) for <details>
t = ['<table class="dep"><thead><tr><th>모듈</th>' + ''.join(f'<th>{NAME[d]}</th>' for d in ORDER) + '<th>합계</th></tr></thead><tbody>']
for kind, lab, c in rows:
    if kind == 'G':
        t.append(f'<tr><th colspan="{len(ORDER) + 2}" class="grp">{html.escape(lab)}</th></tr>')
        continue
    t.append(f'<tr><th>{html.escape(lab)}</th>' + ''.join(f'<td>{c[d]:,}</td>' if c.get(d) else '<td></td>' for d in ORDER) + f'<td>{sum(c.values()):,}</td></tr>')
t.append('</tbody></table>')
open(SP + 'tools/out-bars-table.html', 'w', encoding='utf-8').write('\n'.join(t))
print(H, len(rows))
