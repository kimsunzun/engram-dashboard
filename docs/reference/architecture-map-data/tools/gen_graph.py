# 아키텍처 지도 그림 생성기 — 출력(out-*)을 architecture-map.html 해당 자리에 붙여 넣는다. 원자료 = ../tags.json (그래프 숫자는 notes 3장에서 손으로 옮김).
import math

import os
SP = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..') + '/'
W2, H2 = 72, 30  # half sizes of node boxes
NODES = {
    'CMD':   (520, 62,  '커맨드',   'd-cmd'),
    'COMM':  (520, 212, '통신',     'd-comm'),
    'AGENT': (150, 212, '에이전트', 'd-agent'),
    'SCREEN': (890, 212, '화면',    'd-screen'),
    'MAIL':  (240, 372, '우편',     'd-mail'),
    'AUTH':  (520, 372, '인증',     'd-auth'),
    'LIFE':  (820, 372, '데몬 수명', 'd-life'),
}
# (a, b, a->b, b->a)
PAIRS = [
    ('AGENT', 'COMM', 69, 10),
    ('COMM', 'SCREEN', 6, 8),
    ('CMD', 'COMM', 17, 7),
    ('AGENT', 'CMD', 39, 10),
    ('SCREEN', 'CMD', 7, 2),
    ('MAIL', 'AGENT', 11, 2),
    ('MAIL', 'AUTH', 5, 2),
    ('COMM', 'LIFE', 4, 4),
]

def exit_point(cx, cy, ux, uy):
    tx = W2 / abs(ux) if ux else 1e9
    ty = H2 / abs(uy) if uy else 1e9
    t = min(tx, ty)
    return cx + ux * t, cy + uy * t

def width(n):
    return round(1.2 + n / 12, 1)

out = ['<svg viewBox="0 0 1040 420" role="img" aria-label="백엔드 도메인 의존 그래프. 에이전트가 통신과 커맨드를 가장 많이 부르고, 통신이 여러 도메인과 양방향으로 엮여 있다.">',
       '<defs><marker id="ah3" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="11" markerHeight="11" orient="auto"><path d="M0,0 L10,5 L0,10 z" fill="currentColor"/></marker></defs>']
lines, labels = [], []
for a, b, ab, ba in PAIRS:
    ax, ay = NODES[a][:2]
    bx, by = NODES[b][:2]
    dx, dy = bx - ax, by - ay
    L = math.hypot(dx, dy)
    ux, uy = dx / L, dy / L
    nx, ny = -uy, ux
    sx, sy = exit_point(ax, ay, ux, uy)
    ex, ey = exit_point(bx, by, -ux, -uy)
    mx, my = (sx + ex) / 2, (sy + ey) / 2
    off = 6
    for n, (x1, y1, x2, y2), side in (
        (ab, (sx - nx * off, sy - ny * off, ex - nx * off, ey - ny * off), -1),
        (ba, (ex + nx * off, ey + ny * off, sx + nx * off, sy + ny * off), 1),
    ):
        thin = n <= 3
        dash = ' stroke-dasharray="4 3"' if thin else ''
        lines.append(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke-width="{width(n)}"{dash}/>')
        lx, ly = mx + side * nx * 20, my + side * ny * 20
        labels.append(f'<text x="{lx:.1f}" y="{ly + 4:.1f}" text-anchor="middle">{n}</text>')
out.append('<g stroke="currentColor" fill="none" marker-end="url(#ah3)">' + ''.join(lines) + '</g>')
out.append('<g font-size="12.5" font-weight="700" fill="currentColor" style="paint-order: stroke; stroke: var(--bg); stroke-width: 4px">' + ''.join(labels) + '</g>')
for k, (cx, cy, name, var) in NODES.items():
    out.append(f'<rect x="{cx - W2}" y="{cy - H2}" width="{2 * W2}" height="{2 * H2}" rx="8" style="fill: var(--card); stroke: var(--{var})" stroke-width="2.5"/>')
    out.append(f'<text x="{cx}" y="{cy + 5}" text-anchor="middle" font-size="14" font-weight="700" fill="currentColor">{name}</text>')
out.append('</svg>')
open(SP + 'tools/out-graph.svg', 'w', encoding='utf-8').write('\n'.join(out))
print('ok')
