# 숨은 탭 터미널의 출력 구독·크기 보고 정책 — 업계 서베이

- 상태: **확정 — 사용자 결정 2026-09-25: PC 는 keep-alive 유지(ADR-0056), 숨은 탭 크기 문제는 「잴 수 없는 크기는 보내지 않는다」 가드로 고친다, 모바일 절전(숨으면 구독 해제)은 모바일 클라이언트를 만들 때 설계한다** · 날짜: 2026-09-25
- 방법: medium(설계-결정 모드) · 수집자 2(주도 경량) 병렬 — A 숨은 탭 구독 정책 · B 숨은 동안 크기 보고 · 로컬 클론(`I:\Engram_Workspace\opensource\`) 우선 · 메인 grounding(원문 대조) · cross-family 적대 리뷰 1 회(FIX — 아래 반영).
- 확신도: 확실(독립 교차확증) · 가능성 높음(단일 1차 출처) · 불확실(미검·지식 기반).
- 발단: 숨은 탭 터미널이 구독 직후 크기를 못 잰 채 PTY 크기를 보낸다(`src/components/slot/TerminalSlot.tsx:304-305` — 다른 세 전송 경로 `:99-101`·`:285`·`:355` 는 가려지면 건너뛴다). 사용자 제안 「숨은 탭은 구독하지 않다가 전환 때 구독」을 업계 규칙에 비춰 보았다.
- 선행 조사: ADR-0056 이 xterm.js 계열(VS Code·Tabby·Hyper·Theia)을 조사해 keep-alive 를 골랐다. 이번은 서버형 멀티플렉서·에이전트 관리자와 크기 보고를 더했다.

## 결론

1. **숨은 터미널의 출력 파싱을 멈추는 곳은 조사 범위에 없었다.** 갈리는 것은 「늘 먹이는 에뮬레이터가 어디 사나」다. (가능성 높음)
   - **뷰 keep-alive** — VS Code·Tabby·Hyper·Theia(ADR-0056) · vibe-kanban · t3code(상한 10 스레드까지) · paseo(유지 탭). 숨은 뷰도 계속 쓰고 그리기만 멈춘다. **우리 방식(ADR-0056).**
   - **서버 VT + 숨은 뷰 전달 차단** — tmux · zellij · herdr · orca(Phase 4 「hidden-delivery gate」). 서버가 VT 를 쥐고 있어 보일 때 **현재 화면을 VT 상태에서 다시 그린다**(orca 는 스크롤백까지 스냅숏). orca 는 렌더러 부하 때문에 keep-alive 에서 이쪽으로 옮겼다.
2. **「숨으면 끊고, 전환 때 원시 바이트를 되감아 따라잡기」는 드물고, 확인된 사례 하나는 되돌렸다.** (가능성 높음)
   - **CARB/IDE** 는 숨은 탭 렌더러를 내리고 전환마다 스크롤백 전체를 되감았다가, 되감은 출력 속 터미널 질의(`CSI 6n` 커서 위치 · `OSC 11;?` 배경색)에 xterm 이 다시 답해 **그 답이 셸 입력으로 샜고**(#87), 전환 때 `xterm.reset()` 이 버퍼를 잃었다 → 「열린 탭마다 렌더러 하나 + fit 은 0×0 가드」로 바꿨다(ADR-014).
   - **Codeman** 은 뒤 탭 전환에서 1 MB 꼬리를 받아 되감는다 — 밑에 tmux 가 있고, 그 잘림이 「출력이 사라진 것처럼 보인다」는 문제로 올라왔다(#258).
   - 나머지 되감기는 콜드 경로(처음 열기·상한 넘어 내린 뒤 다시 열기·재시작 복원)에만 나온다(t3code · paseo 첫 부착 · orca 콜드 복원).
3. **크기: 잴 수 없는(가려짐으로 0 이거나 측정 불가) 크기는 보내지 않고 마지막 크기를 유지한다 — 조사한 xterm 기반 피어 공통.** (확실 — VS Code · paseo · orca · CARB 원문)
   - 「숨은 동안은 무조건 안 보낸다」가 아니다 — **VS Code 는 숨은 동안 잴 수 있는 크기 변화를 유휴 때로 미뤄 보내고, 보이면 `flush()` 한다.** 0 이하 layout 만 버린다.
   - xterm.js `FitAddon`(0.11.0)은 가려짐을 확인하지 않는다. 부모 계산 너비가 `auto` 면 NaN 이라 `fit()` 이 아무것도 안 하고, 0 이면 2×1 로 줄인다(`FitAddon.ts:23-24,36,72-74,87-88`). 그래서 피어마다 자기 가드를 둔다(paseo `offsetWidth/offsetHeight === 0` · orca 48×24 px·8×4 셀 하한 · VS Code 0 이하 무시 · CARB 0×0 가드).
   - 한 번도 못 잰 첫 부착: VS Code = 다른 터미널이 마지막으로 잰 격자 → 없으면 80×30 · 서버형 = 설정된 헤드리스 기본값(herdr 120×40 · zellij 감시 클라이언트 80×24).
   - 한 PTY 를 여러 뷰가 볼 때 숨은·비활성 뷰는 크기를 정하지 않는다(tmux `window-size` 기본 latest · herdr 전경 클라이언트 · paseo 명시 소유자).

## 발견 표

| 피어 | 숨은 동안 | 보일 때 | 크기 | 출처 | 확신도 |
|---|---|---|---|---|---|
| VS Code 통합 터미널 | 계속 씀 | 해당 없음 | 0 이하 layout 무시(`:2021`) · 숨은 동안 전송을 유휴로 미룸(`terminalResizeDebouncer.ts:61`)·보이면 `flush()`(`:1449`) · 첫 크기 = 마지막 격자(`:2092`) → 80×30(`:105-106`) | microsoft/vscode@main `terminalInstance.ts` · `terminalResizeDebouncer.ts` · ADR-0056 | 확실 |
| Tabby · Hyper · Theia | 계속 씀 | 해당 없음 | (미조사) | ADR-0056 조사 | 가능성 높음 |
| vibe-kanban | 모든 탭 렌더·각자 xterm+WebSocket | 해당 없음 | (미조사) | vibe-kanban@`packages/ui/src/components/TerminalPanel.tsx:14` | 가능성 높음 |
| t3code | 상한 10 까지 마운트·쓰기 유지, 안 보이면 프레임 생략 | 상한 넘어 내린 것은 서버 `open` → 스냅숏+이력 | 0 크기면 그리기 생략(PTY 전송 쪽은 미확인) | t3code@`apps/web/src/terminal/ghostty/surface.ts:1808-1816` · `apps/server/src/terminal/Manager.ts:93-94` | 가능성 높음 |
| paseo | 유지 탭은 스트림 유지(서버에 헤드리스 xterm) | 첫 부착·복원 = 서버 `TerminalState` 스냅숏 | `offsetWidth/offsetHeight === 0` 이면 fit·전송 안 함 · 서버는 명시 소유자만 크기 갱신 | paseo@`packages/app/src/terminal/runtime/terminal-emulator-runtime.ts:371` · `packages/server/src/terminal/terminal-size-ownership.ts:11-38` | 확실(크기) · 가능성 높음(스트림 — e2e `terminal-retained-tab-stream.spec.ts:46` 은 부착 덮개가 안 뜨는 것만 단언) |
| orca | 헤드리스 xterm(스크롤백 5000)이 전부 파싱, 숨은 렌더러 전달은 버림(기본 켜짐·끄는 스위치) | 모델 스냅숏(화면+스크롤백), seq 가드 | 48×24 px·8×4 셀 미만이면 fit 미룸 | orca@`src/main/ipc/pty-hidden-delivery-gate.ts:1-11` · `src/renderer/src/lib/pane-manager/pane-fit-measurability.ts:8-11` (스냅숏 df7460af — upstream 미재확인) | 확실 |
| tmux | 서버 격자는 모든 창 갱신, 출력은 보통 그 창을 보는 클라이언트에만(보이지 않는 pane 용 예외 경로 `TTY_CTX_INVISIBLE_PANES` 가 있다) | 창 전환 = 격자에서 다시 그리기 · 스크롤백은 서버에 두고 명시 당김 | `window-size` latest(기본)/largest/smallest/manual | tmux@`screen-write.c:138-150` · `server-fn.c:94-104` | 확실 |
| zellij | 서버 격자 갱신, 그 탭을 보는 클라이언트가 없으면 `render` 조기 반환 | 활성 탭 전체 렌더 | 클라이언트별 크기(최종 PTY 크기 경로 미추적) | zellij@`zellij-server/src/tab/mod.rs:4835-4850` | 확실(렌더) · 불확실(크기) |
| herdr | 서버 헤드리스, 아무도 안 보는 PTY 갱신은 버림 | 서버 pane 상태로 다시 그림 | 전경 클라이언트가 정함 · 클라이언트 없으면 120×40 | herdr@`src/render_signal.rs:14-15` · `src/server/headless.rs:237-243` | 가능성 높음 |
| CARB/IDE | 지금: 열린 탭마다 렌더러 유지 · 예전: 숨은 탭 렌더러 없음 | 예전: 전환마다 스크롤백 전체 되감기 → 질의 응답 누출(#87)로 폐기 | fit 0×0 가드 | [ADR-014](https://docs.carbidecore.online/design/decisions/adr-014-terminal-renderer-model/) | 확실(원문 확인) |
| Codeman | (tmux 백엔드) | 뒤 탭 전환에서 1 MB 꼬리 되감기 · 잘림 표시가 약해 손실로 보였다 | (미조사) | [Ark0N/Codeman#258](https://github.com/Ark0N/Codeman/issues/258) | 가능성 높음 |
| WezTerm · Windows Terminal · kitty · iTerm2 · ttyd/gotty | 계속 파싱, 보이는 것만 그림 | 해당 없음 | (미조사) | 지식 기반(출처 없음) | 불확실 |

## grounding (메인 원문 대조)

- 지지: orca 게이트 주석 · tmux `server_redraw_window`·`screen-write.c` 거름 · paseo 0 크기 조기 반환 · zellij `render` 조기 반환 · VS Code `width <= 0 || height <= 0`·80×30·`_isVisible` 미룸·`flush()` · CARB ADR-014 본문 · Codeman #258 본문.
- **정정(수집자 오진):** 「가려진 부모에서 `fit()` 이 2×1」 → 계산 너비 `auto` 면 NaN 이라 no-op, 0 일 때만 2×1. 우리 컨테이너(`width:100%`, `TerminalSlot.tsx:381`)가 가려진 조상 아래에서 어느 쪽인지는 실측 안 했다.

## 적대 리뷰 (codex · FIX → 반영)

- 「되감기는 콜드 경로에만」 → 반례 둘(CARB·Codeman). **결론 2 를 고쳤다.** CARB 가 되돌린 사유(질의 응답 누출)는 우리 되감기 경로에도 해당할 수 있다 — 아래 「우리에게 남는 것」.
- 「숨은 동안 크기를 안 보낸다가 예외 없는 규칙」 → VS Code 는 잴 수 있으면 미뤄 보낸다. **결론 3 을 「잴 수 없는 크기」로 좁혔다.**
- tmux 「그 창을 보는 클라이언트에만」 → `TTY_CTX_INVISIBLE_PANES` 예외. **표에 단서를 달았다.**
- paseo e2e 인용은 부분 지지 → 확신도 낮춤.

## 선택지와 결정

| 선택지 | 무엇 | 얻는 것 | 잃는 것 | 선례 | 결정 |
|---|---|---|---|---|---|
| A keep-alive 유지 + 잴 수 없는 크기는 안 보냄 | 구독 직후 초기 전송에 다른 세 경로와 같은 가드 | 1번 결함 해소 · 전환 즉시·무손실 · 작은 변경 | 숨은 탭도 출력 처리 비용(그리기는 이미 멈춤) | VS Code·Tabby·Hyper·Theia·vibe-kanban·t3code·paseo · CARB(되돌린 뒤) | **PC 채택** |
| B 서버 VT + 숨은 뷰 전달 차단 | 데몬에 헤드리스 VT, 보일 때 현재 화면 스냅숏 | 숨은 뷰 비용 0 · 복원이 되감기보다 견고 | 데몬 VT 신설(큰 변경) | tmux·zellij·herdr·orca | 보류 — 숨은 탭 비용이 실측으로 문제될 때 |
| C 숨으면 구독 해제 + 보일 때 되감기(= 새 칸을 여는 경로) | 지금 재구독 경로 재사용 | 숨은 뷰 비용 0 · 구현 가벼움 | 전환마다 되감기 지연·흔들림 · 되감기 질의 응답 누출 위험(CARB #87) · 버퍼 한도 넘은 출력 손실 | CARB(폐기) · Codeman(tmux 밑) | **PC 거부 · 모바일 후보**(대역폭·배터리, 한 번에 한 칸) — 모바일 클라이언트를 만들 때 설계 |

## 우리에게 남는 것

- **1번 결함 수정(A)** — 브랜치 `v0.3.2/fix/terminal-cursor-ime-resize`.
- **되감기 질의 응답 누출 확인** — 우리 입력 가드(`TerminalSlot.tsx` `onData`)는 에이전트 부재일 때만 막는다. 실행 중인 에이전트를 새 칸에 띄울 때 되감은 출력 속 `CSI 6n`·`OSC 11;?` 에 xterm 이 답해 PTY 로 새는지 재현이 필요하다(`docs/todo.md`).
- 모바일 절전 — `docs/todo.md` 트리거 대기.

## 한계·공백

- WezTerm·Windows Terminal·kitty·iTerm2·ttyd/gotty 는 원문 미확인(지식 기반) · superset·Coder v2·sshx 미조사.
- orca 클론은 upstream 보다 한 달 뒤다.
- 모바일 클라이언트의 숨은 뷰 처리는 따로 조사하지 않았다.
