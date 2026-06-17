# ADR-0021: 데몬 수명 — on-demand spawn + 자동재시작 없음 (tmux/wezterm 모델)

- 상태: 확정 (2026-06-17, 근거: 데몬-서버 prior-art 조사[tmux/wezterm/emacs/gpg-agent/LSP] + 사용자 결정 + ADR-0015 확장)
- 관련: ADR-0015(데몬 persist-until-kill·콘솔=detachable 뷰어·ensure-on-open) **확장/정정** · ADR-0008(S9 restore_all) · `src-tauri/src/discovery.rs`(ensure_daemon/Spawner) · `src/api/wsTransport.ts`(연결/재연결) · `crates/engram-dashboard-protocol`(StopDaemon)
- 범위: 데몬(인프라) 수명 — 자동시작/종료/크래시 처리. **에이전트 수명(ADR-0016/0019)과 다른 축.**

## 맥락

데몬은 persist 상주(ADR-0015). 문제: ensure-on-open이 **재연결 루프에서도 spawn**을 호출해, 사용자가 데몬을 끄려고 kill해도 GUI 재연결이 즉시 되살림 → 못 끔. "크래시면 살리고 의도종료면 유지"로 가려 했으나(systemd on-failure), **raw kill(taskkill /F)과 크래시는 exit code로 구분 불가** — 구분 시도 자체가 불안정.

prior-art 조사 결론(tmux·wezterm·emacs·gpg-agent·LSP): **전부 on-demand spawn + 자동재시작 없음.** 이유 — 상태가 서버 메모리에만 있어 재시작해도 빈 서버라 무의미. 그래서 "사용자가 못 끈다" 문제가 애초에 없다(watchdog가 없으니 kill=종료가 충돌 안 함). 자동재시작은 dockerd처럼 **디스크 영속** 상태일 때만 의미.

## 결정

**tmux/wezterm 모델. 자동재시작(watchdog) 없음. on-demand spawn만.**

1. **spawn(ensure) = 명시 시점만:** 부팅 연결 / 사용자 `daemon_start` / 대시보드 열기 같은 **의도적 연결**에서만 "없으면 spawn". (wezterm `no_serve_automatically=false` = 연결 실패 시 자동 spawn 패턴.)
2. **재연결 루프 = attach-only(절대 spawn 안 함):** 데몬이 죽으면 GUI 재연결은 **기존 데몬에 재부착만** 시도, 없으면 spawn하지 않고 `down` 상태 표시 + "시작" 제공. **이게 "kill하면 respawn" 친화성 문제의 핵심 수정** — ensure(명시)와 reconnect(attach-only)를 분리.
3. **자동재시작 없음:** kill이든 크래시든 데몬이 죽으면 **꺼진 채 유지.** raw kill/크래시 구분 안 함(불가능하고 불필요). 복구는 다음 **명시 연결 시 fresh 데몬 + `restore_all`**(ADR-0008, sid `--resume`)로 영속 agent를 되살림 — tmux와 달리 손실이 영구 아님.
4. **stop = 명시 종료:** `daemon_stop`(트레이/명령/우리 핸들) → StopDaemon(graceful) 또는 kill → 죽고 유지(재연결이 안 살림). systemd `systemctl stop`과 동치.
5. **command 표면(§5 — LLM·UI·트레이 동일 핸들, 플랫폼 중립):** `daemon_start`(명시 ensure) · `daemon_stop`(StopDaemon) · `daemon_status`(alive/pid/port). `set_daemon_console(on|off)`.
6. **콘솔(디버그 로그):** 기본 windowless(`CREATE_NO_WINDOW`류). `set_daemon_console(on)`이면 콘솔 창과 함께 spawn(로그 가시화). 데몬 수명과 무관(ADR-0015 detachable 뷰어 최소판).

## 거부한 대안
- **systemd on-failure / 크래시 한정 재시작(이 ADR 초안이었음):** raw kill과 크래시를 exit code로 구분 불가 → desired-state 플래그로 우회해도 raw kill은 여전히 크래시로 오분류. 게다가 데몬 죽으면 메모리 agent도 죽어 자동재시작해도 빈 데몬 — 실익이 `restore_all`(다음 연결 복원)로 이미 대체됨. 복잡도만 추가라 폐기.
- **상시 watchdog 프로세스:** "사용자가 못 끈다" 문제를 재발시킴. prior-art 어디도 안 함. over-engineering.
- **desired-state(daemon-config.json) 파일:** 모델 A에선 "꺼진 채 유지"가 *reconnect가 spawn 안 함*으로 자연 성립 → 별도 의도-플래그 불필요. (앱 재시작 후에도 꺼둠을 원하면 후속에 작은 플래그 추가 — 지금은 YAGNI.)

## 근거
- prior-art 5종(tmux/wezterm/emacs/gpg-agent/LSP) 전부 on-demand+무재시작으로 수렴. wezterm(GUI+headless mux-server 분리, GUI 닫혀도 mux 생존, 연결실패 자동 spawn)이 engram 구조와 1:1.
- engram은 sid/agents.json 영속 → 데몬 재기동 시 `restore_all`로 agent 복원. "크래시 자동재시작"의 유일한 실익(상태 유지)을 이미 보유.
- spawn은 `trait Spawner`(discovery.rs)로 추상화 → 모델·플랫폼 중립. 다른 OS = Spawner 구현 추가.

## 영향 / 불변식
- **ensure(spawn)와 reconnect(attach-only)는 분리** — 재연결 루프는 절대 spawn 호출 금지(이게 깨지면 "못 끄는" 버그 재발).
- 데몬 죽음 = 꺼진 채 유지. 복구는 명시 연결 + restore_all. watchdog/desired-state 없음.
- 데몬 인프라 수명 ≠ 에이전트 런타임 자동재시작(ADR-0019 폐기). 혼동 금지.
- command는 플랫폼 중립(Q1 경계: command=중립 / 트레이·메뉴=플랫폼 뷰). 트레이(#2)는 이 command 호출만.
- 콘솔 토글은 데몬 수명에 영향 없음.
