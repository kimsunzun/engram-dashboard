# Study Note — 멀티뷰어 터미널 출력 버퍼 아키텍처 (2026-06-29, light)

## 쟁점 (왜 조사했나)
S14 T6b 출력 평면에서 N1 결함: 같은 에이전트를 여러 창(main+popup)에 띄울 때 per-agent 단일 high-water seq로는 창별 진도를 못 챙긴다. 사용자 지적: "새 창이 같은 에이전트 보려면 또 데몬에 네트워크 재요청 → 구조가 이상하다." → 게이트웨이(src-tauri) 캐시가 맞는지, 중앙 보관이 맞는지 OSS 확인.

## 방법 (light)
Claude(Sonnet) 1 + Codex 1 BLIND 병렬(서로 결과 안 봄). 적대검증 생략, 교차 대조만. tmux/screen/Zellij/mosh/ttyd·gotty/VS Code 비교.

## 근거 → 결론 (두 모델 수렴)
**Q1 버퍼 위치 = 중앙 데몬 보관 + attach 시 재요청(redraw/replay)이 표준 (확실).**
- tmux: 서버가 pane grid/scrollback 보관(`grid.c` hsize/hlimit), 클라 stateless, attach 시 `screen-redraw.c`가 현재 viewport만 redraw(scrollback 전체 replay 안 함 — copy mode로만).
- Zellij: `zellij-server` Grid가 보관, attach 시 `ServerToClientMsg::Render`로 서버가 per-client 렌더 push.
- VS Code: pty host(=실질 서버)가 `PersistentTerminalProcess`로 보관, 재연결 시 replay 이벤트. 렌더러(xterm.js)=클라.
- 게이트웨이 캐시는 **비표준** — Codex: "common only in web/UI renderers, not as the authoritative scrollback store." ttyd/gotty는 서버 replay 없어서 "tmux 얹으라"가 공식 권장.

**Q2 멀티뷰어 진도 = 콘텐츠 중앙 공유 + per-viewer 렌더상태, 출력에 단일 consumed cursor 없음 (확실).**
- ★핵심(Codex)★: "터미널 출력은 큐처럼 consume하는 게 아니라 state-rendered/broadcast/redraw된다. 그래서 단일 'how far consumed' cursor가 없다."
- tmux/screen: live viewport는 공유 단일, scrollback 탐색(copy mode)은 per-client 독립. 창 크기 다르면 `window-size` 정책(largest/smallest/manual/latest).

## 우리 설계 시사 (N1 근본 진단)
- 우리는 출력을 **seq 큐 소비 모델**(per-agent high-water + dedup)로 다루는데 업계는 **state-render 모델**. 이 미스매치가 N1의 근본.
- seq는 **데몬↔src-tauri 단일 채널 전송 무손실용으로는 맞다**. 문제는 그 단일 seq에 **창별 화면 채우기를 묶은 것**.
- tmux는 1계층(서버↔클라), 우리는 2계층(데몬↔게이트웨이↔창)이라 정확한 선례 드묾(VS Code pty-host가 가장 가까우나 1계층). 2계층에선 src-tauri가 "창들의 서버" 역할이라 게이트웨이 캐시(B안)가 오히려 자연스러울 여지.

## 미해결 (T7 결정)
- 새 창 채우기: (A) 데몬 재요청 유지(표준·단순·2홉) vs (B) src-tauri 화면상태 캐시(2계층 적합·비표준·동기화 비용).
- seq(전송 무손실)와 렌더(창 채우기) 분리 = N1 해소 방향.

## 한계
light라 적대검증 없음. ttyd 중앙 replay 의미는 두 모델 다 불확실. 2계층 게이트웨이 캐시의 정확한 OSS 선례는 못 찾음(VS Code 1계층이 최근접).
