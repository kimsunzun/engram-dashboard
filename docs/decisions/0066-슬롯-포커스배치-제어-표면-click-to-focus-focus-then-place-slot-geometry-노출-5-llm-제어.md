# ADR-0066: 슬롯 포커스·배치 제어 표면 — click-to-focus + focus-then-place + slot geometry 노출 (§5 LLM 제어)

- 상태: 확정 (2026-07-10, 근거: 사용자 결정 2건(배치=focus-then-place 먼저+드래그 나중 · 포커스 강도 65%) + `/research medium` OSS 서베이(VS Code·JetBrains·tmux·Zellij·i3/sway·wezterm) + Codex 적대 리뷰)
- 개정: 2026-07-10 — 크로스-윈도우 active 타깃 미결 해소(§결정 5 신설, `/research medium` 재서베이 + Codex/doc 리뷰 FIX 반영 — ADR-0057 창별 active 모델과 정합). 사용자 결정: last-focused-wins.
- 관련: CLAUDE.md §5(LLM-우선 제어) · ADR-0035(레이아웃 권위=백엔드 ViewManager) · ADR-0057(창별 탭·active 소유 모델 — 크로스-윈도우 active 타깃 정본) · ADR-0022/0055(command registry) · ADR-0011(agentClient assign) · ADR-0060(SlotContent 유니온) · `src-tauri/src/layout/manager.rs`(focused_slot_id) · `src/components/layout/ViewLayoutRenderer.tsx` · step-log "슬롯 포커스·배치 제어 표면"

## 맥락

세 가지 갭이 한 뿌리(포커스 개념 부재)에서 나왔다:
1. **포커스가 클릭으로 안 옮겨진다.** `focused_slot_id`는 구조 변경(부팅·탭 생성·분할·닫기)의 사이드이펙트로만 설정된다(`manager.rs` fixup_focus/make_view/split/close). 클릭→포커스 배선이 없어 사용자가 "선택"을 못 옮긴다. 런타임 확인: 다른 슬롯 클릭해도 `focused_slot_id` 불변.
2. **트리 에이전트를 슬롯에 놓을 방법이 없다.** 에이전트 트리 항목을 특정 슬롯에 배치하는 상호작용이 없다. "현재 포커스에 놓기"를 하려면 진짜 포커스 개념이 선행돼야 한다.
3. **LLM 공간 지시("우하단에 놓아줘")를 해석할 표면이 없다.** 슬롯이 opaque id로만 노출돼 자연어 공간 참조를 id로 못 옮긴다.

굵은 §5 제어 표면 결정이라 `/research medium`(OSS 서베이 → 옵션셋 → 사용자 결정)으로 근거를 만들었다. 서베이는 Codex 적대 리뷰에서 FIX(과장 2·누락 4) 적출 → 보정 후 방향 유지 판정.

## 결정

포커스·배치·공간타겟을 **사람 클릭과 LLM 커맨드가 같은 핸들을 흔드는 단일 control surface**로 만든다(§5).

1. **포커스 = click-to-focus.** 백엔드 `manager.set_focused_slot(view, slot)` + `focus_slot` Tauri command(ADR-0035 권위 — 낙관 갱신 없이 emit 반영). 슬롯 pane 클릭 시 호출. 분할/생성 시 auto-focus는 유지(전 업계 기본). `slot.focus` command로 registry 등록 → 사람·팔레트·키바인딩·LLM(`__engramCmd.run('slot.focus', …)`)이 동일 핸들.
2. **배치 = focus-then-place 우선, 드래그 나중.** 슬롯 클릭(포커스) → 트리 에이전트 더블클릭/Enter → 포커스 슬롯에 `assign`(ADR-0011). 단일 커맨드 표면(`focus_slot` + `assign`)이라 사람·LLM 동일 경로. 드래그앤드롭은 후속으로 "마우스 편의 입력"만 얹는다(같은 assign 커맨드 호출).
3. **LLM 공간 타겟팅 = 슬롯 geometry 노출.** 각 슬롯의 `{id, x, y, w, h}`를 control surface(`__engramLayout` / 조회 커맨드)로 노출 → LLM이 수치 추론("우하단" = x+w·y+h 최대)으로 대상 id를 도출 → id로 명령. (방향 sugar 커맨드(tmux `{bottom-right}` 결)는 후순위 옵션.)
4. **포커스 표시 강도 = 65%.** border 항상 `1px --border` + `inset box-shadow color-mix(accent 65%)`(레이아웃 이동 0). 최초 "가장 은은(40%)"에서 상향 — 근거는 아래.
5. **크로스-윈도우 active 타깃 = (마지막 focus된 창) → (그 창 active 탭) → (그 탭 `focused_slot_id`)로 해소 (last-focused-wins).** ★ADR-0057 정합★: ADR-0057이 제거한 전역 `active_view_id`를 **되살리지 않는다**. ViewManager는 **마지막으로 슬롯이 focus된 창 label**만 추적하고(창 레벨 — ADR-0057 창별 active와 같은 층위), "배치 대상 슬롯"은 `windows[마지막 focus 창].active`(창별 active 탭)의 `focused_slot_id`로 **파생**한다(새 전역 View 포인터 신설 아님). 각 View는 자기 `focused_slot_id`를 독립 기억. 슬롯 클릭 → 그 창을 "마지막 focus 창"으로 + 그 창 active 탭의 focused_slot_id 갱신. 클릭 없이 창 전환(alt-tab 등)하면 그 창 active 탭이 기억한 focus로 타깃이 따라간다(창별 메모리 — 이전 창 슬롯에 고정 안 함). 시각 = 마지막 focus 창의 active 슬롯만 풀강조(65% inset), 다른 창/뷰의 focused_slot은 dimmed 메모리(동시 equal 링 금지). VS Code active editor group(창별 소유)과 동형(§근거).

## 거부한 대안

- **focus-follows-mouse (타일링 WM 패턴, i3/sway 기본):** 서베이상 GUI 에디터(VS Code·JetBrains)는 예외 없이 click-to-focus. click-driven 대시보드에서 follow-mouse는 마우스 지나가기만 해도 오포커스 유발. 거부.
- **드래그-only 배치:** ① §5 표면 분리 — LLM은 드래그를 못 해 커맨드 경로가 별도로 필요(제어 표면 갈라짐). ② WCAG 2.2 SC 2.5.7 = 드래그 기능엔 비-드래그 단일포인터 대안 필수. ③ 드래그는 가장자리 드롭존 정밀도·멀티모니터 버그 이슈. → 드래그는 커맨드(focus-then-place) 위 **보조 입력**으로만. 드래그-only 거부.
- **그룹-번호 배치 UI("3번 슬롯에 열기"):** 성숙 앱 UI는 절대 번호가 아닌 상대 방향("to the side"/"opposite")을 노출(번호는 API엔 존재 — VS Code ViewColumn 1~9). 우리는 opaque slot id + geometry가 더 적합. 번호-UI 거부.
- **방향-only 공간 타겟(geometry 미노출):** 방향 primitive(select-pane -R 등)는 상대 이동이라 "우하단"을 한방에 못 짚는다. geometry 스냅샷 노출이 이식성·범용성 높음(tmux/i3가 이 경로). 단 tmux `{bottom-right}` 토큰처럼 한방 공간 타겟이 아예 불가한 건 아님(Codex 보정) → 방향 sugar는 후순위로 열어둠. geometry-우선, 방향-only 거부.
- **너무 은은한 포커스 표시(40% inset):** GUI 에디터(VS Code #24586, JetBrains IDEA-102931)의 약한 포커스 표시가 반복 UX 불만이라 40%는 "안 보인다" 함정. 65%로 상향(은은하되 식별). 40% 거부.
- **배치를 last-focus가 아니라 정책/커맨드로 (JetBrains·Emacs):** JetBrains는 새 파일을 focus된 split의 **반대편**에 연다(공식 문서 — "focus in the left split → opened in the right split"). Emacs `display-buffer`는 buffer 배치를 정책 함수로 완전 커스터마이즈(창을 선택 없이 고르거나 생성). 성숙 대안이나 ① §5 단일 control surface와 어긋남(정책 레이어가 별도 축) ② 사용자 멘탈모델("클릭한 데로 간다")과 불일치 ③ LLM "우하단에 놓기"를 정책 함수로 표현하기 번거로움. last-focused-wins 채택, 정책 배치 거부.
- **창을 넘는 단일 전역 active 변수(창 전환 무시):** Codex 적대 리뷰가 "single global active target across windows is standard"를 REFUTED — 서베이 툴 전부 per-window/per-container 스코프 + 메모리다(tmux 윈도우별 active pane, i3 `focused_inactive`, browser 창별 active tab). 창 전환 시 타깃이 이전 창 슬롯에 고정되면 관행 위반. 게다가 이건 **ADR-0057이 이미 제거한 전역 `active_view_id`를 되살리는 것**이라 확정 결정과 충돌(doc 리뷰 FIX-1). → (마지막 focus 창)→(창별 active 탭)→(`focused_slot_id`) 파생으로 채택(결정 5), 단일 전역 변수 거부.

## 근거

- **OSS 서베이 grounding:** click-to-focus(GUI 공통)·양방향 배치(마우스 드래그 + 키보드 focus-then-open, VS Code 문서가 "활성 그룹에 열림, 먼저 클릭" 명시)·geometry 기반 공간추론(tmux `list-panes -F`·i3 IPC rect)은 다수 출처로 확인.
- **Codex 적대 리뷰 보정:** F8("방향만으론 우하단 불가")은 tmux `{bottom-right}` 토큰으로 과장 판정 → 방향 sugar 후순위로 반영. WCAG 2.5.7·NN/g latency(직접조작 0.1s) 캐비엇 추가.
- **§0 판단기준(저위험·장기):** 커맨드 표면·geometry seam은 지금 깐다(나중에 바꾸면 비쌈). 방향 sugar·드래그는 실측 후.
- **크로스-윈도우 재서베이(2026-07-10):** last-focused-wins가 배치 타깃 표준임을 다수 독립 툴로 교차확증(VS Code·tmux·wezterm·Zellij·i3/sway·screen·kitty·browser) — 확신도 '확실'. VS Code가 최근접 동형(창별 active editor group + 그룹별 memory + inactive dimming). Codex(연구)·doc 리뷰가 "창 넘는 단일 전역 변수" 프레이밍을 FIX(ADR-0057이 제거한 전역 `active_view_id` 부활 소지) → (마지막 focus 창)→(창별 active)→(`focused_slot_id`) 파생으로 정정(결정 5).

## 영향 / 불변식

- **포커스·배치·geometry는 control surface(command / `__engramLayout`) 경유** — 사람 클릭·LLM이 동일 핸들(§5). UI가 store를 직접 흔들지 않는다.
- **백엔드 권위(ADR-0035):** `focused_slot_id`·assign은 백엔드가 소유, 낙관 프론트 갱신 금지 → emit 반영. **단 focus 클릭 왕복은 직접조작이라 ~100ms(loopback) 내 또는 낙관 시각 피드백으로 즉각 느껴지게** (NN/g). 이 latency 목표를 구현·QA에서 확인.
- **active 타깃 = 백엔드 파생(ADR-0035 권위 · ADR-0057 창별 active 정본):** 배치 대상 = `windows[마지막 focus 창].active`의 `focused_slot_id` — 백엔드 ViewManager가 소유·해소한다(ADR-0035 레이아웃 권위 불변). 구체 창별 active 모델 정본 = ADR-0057(전역 `active_view_id` 부활 금지). 프론트가 별도 "active" 상태를 들면 §5 위반(리뷰 reject). first slice(click-to-focus)는 `focused_slot_id` 배선만(set_focused_slot 신설 예정), 마지막-focus-창 추적 + place는 후속 슬라이스.
- **드래그는 커맨드 위 보조 입력** — 드래그 경로가 assign 커맨드를 우회해 별도 상태를 만들면 §5 위반(리뷰 reject).
- **auto-focus-on-split 유지** — click-refocus와 공존(대체 아님).
- load-bearing 경로에 `// ADR-0066` 앵커.
- **미구현 잔여(이 ADR가 여는 것):** 방향 sugar 커맨드 · 드래그앤드롭 · 키보드 방향 포커스 이동은 후속(이 ADR는 표면·우선순위만 확정).
