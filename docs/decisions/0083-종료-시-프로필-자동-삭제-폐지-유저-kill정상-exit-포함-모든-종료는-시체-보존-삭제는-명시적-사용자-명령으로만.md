# ADR-0083: 종료 시 프로필 자동 삭제 폐지 — 유저 kill·정상 exit 포함 모든 종료는 시체 보존, 삭제는 명시적 사용자 명령으로만

- 상태: 확정 (2026-07-14, 근거: 유저 실측 — 트리에서 에이전트 종료 후 우클릭 재활성화 시 "실패" 즉시 재현 + 코드 확정: `UserKill → reaper DeleteProfile → profiles.remove` → 재활성화 시 `profile not found`)
- 관련: Amends ADR-0019 (유저 kill·정상 exit(code0) → 프로필 삭제 조항 폐지: 모든 종료는 프로필 시체 보존(KeepDisableAutoRestore), 자동 삭제 없음 — 삭제는 명시적 사용자 명령으로만) · ADR-0082(활성화=resume 전용·시체 보존 정책 — 이 ADR 이 그 정책을 kill/exit 종료 경로까지 확장해 §열린항목 ② code-0 갭을 닫음) · ADR-0016(auto_restore 수명 모델) · `agent/reaper.rs::decide`/`apply_disposition` · `daemon/connection_core.rs::{SpawnProfile, DeleteProfile}` · step-log

## 맥락
ADR-0082 가 "활성화=resume 전용, 실패해도 시스템이 새 대화를 만들지 않고 프로필을 시체로 보존한다"는 정책을 확정했다. 그런데 그 확정은 **resume 실패 경로(claude 가 "No conversation found" 로 exit≠0)** 만 시체 보존(`KeepDisableAutoRestore`)으로 처리했을 뿐, **종료 자체를 분류하는 reaper 처분(ADR-0019)** 은 그대로 뒀다.

ADR-0019 §결정 표는 "의도된 종료(유저 kill·claude `/exit` exit0)=프로필 **삭제**"로 정해져 있다:

| 종료 원인 | 처분(reaper `decide`) |
|---|---|
| 유저 kill (UI/LLM 커맨드) | `DeleteProfile` |
| claude `/exit` (exit 0) | `DeleteProfile` |
| 크래시 (exit≠0/signal/EOF) | `KeepDisableAutoRestore` |
| 데몬 셧다운 | `KeepAsIs` |

**유저 실측 버그(즉시 재현):** 에이전트 트리에서 에이전트를 종료(kill)한 뒤 우클릭 → 재활성화하면 "실패"가 뜬다. 원인은 resume 가 아니다 — **resume 경로에 닿기도 전에** 프로필이 사라진다. 체인(코드 확정):
1. 트리 종료 → `manager.rs::kill_agent` 가 `TerminationIntent::UserKill` 태깅.
2. 에이전트 죽음 → pump finish → reaper `decide`: `(UserKill, _) => DeleteProfile` (`reaper.rs`).
3. `apply_disposition` → `profiles.remove(id)` → 프로필이 레지스트리+디스크(`agents.json`)에서 삭제(`claude_session_id` 포함).
4. 재활성화 → `connection_core.rs::SpawnProfile` 가 `profiles().get(id)` = `None` → `Err("profile not found")` → 화면 "실패".

즉 ADR-0082 의 "아무것도 죽지마·삭제하지마·시체로라도 남겨" 사용자 정책이 **resume 경로에만 반영되고 kill/exit 종료 경로엔 반영되지 않아**, 사용자가 명시적으로 종료한 에이전트는 시체가 아니라 완전 삭제된다. ADR-0082 §열린항목 ②(code-0 조기종료 시 삭제돼 시체 안 남음)가 정확히 이 갭의 일부다.

## 결정
**reaper 는 어떤 종료에도 프로필을 자동 삭제하지 않는다.** ADR-0019 §결정 표의 "유저 kill·정상 exit(code0) → `DeleteProfile`" 조항을 폐지하고, 두 경우 모두 **시체 보존(`KeepDisableAutoRestore` — 프로필 유지, `auto_restore=false`)** 으로 내린다.

- **모든 런타임 종료(유저 kill·정상 exit·크래시·EOF)** = 세션은 맵에서 수거하되 **프로필은 보존**하고 `auto_restore=false` 로 내린다(부팅 자동복원 대상에서만 제외 — 다음 부팅에 저절로 되살아나지 않음). 데몬 셧다운은 기존대로 `KeepAsIs`(auto_restore 그대로 → 부팅 복원).
- **자동 삭제 경로는 존재하지 않는다.** 프로필 삭제는 **명시적 사용자 명령**(`AgentCommand::DeleteProfile` / Tauri `delete_profile`)으로만 일어난다 — reaper 는 삭제 처분을 산출하지 않는다.
- **재활성화 = 시체 resume.** 종료된 에이전트는 프로필+`claude_session_id` 를 그대로 갖고 남으므로, 우클릭 재활성화 시 `--resume <sid>` 로 정상 이어받는다(대화가 실재하면 resume 성공, 없으면 ADR-0082 대로 Failed+원인 로그).

이는 ADR-0082 정책("안 죽이고·안 새로 만들고·프로필 보존·원인 로그")을 **종료 분류 계층까지** 확장·완결한 것이다. ADR-0082 §열린항목 ②(code-0 갭)를 닫는다.

## 거부한 대안
- **유저 kill·정상 exit = 프로필 삭제 유지(ADR-0019 현행)** — "사용자가 명시적으로 죽였으니 지워도 된다"는 옛 전제. 거부: 사용자 정책이 번복됐다("삭제하지마, 시체로라도 남겨"). 이 전제가 남으면 (a) 종료→재활성화가 `profile not found` 로 깨지고 (b) 종료 순간 대화 맥락(`claude_session_id`)이 유실되며 (c) ADR-0082 의 시체 보존이 kill 경로에서 무효화된다.
- **UserKill 만 보존, 정상 exit(code0)은 삭제 유지** — /exit 는 사용자가 claude 안에서 의도적으로 끝낸 것이니 삭제로 둔다. 거부: 사용자 명시 결정 = **둘 다 보존**. code-0 도 대화가 실재해 resume 가능한 시체이며, 삭제 유지 시 ADR-0082 §열린항목 ② 갭이 그대로 남는다. "의도된 종료냐"로 삭제를 가르는 것 자체가 폐기 대상.
- **재활성화 진입점(`SpawnProfile`)에서 프로필 없으면 즉석 재생성** — 삭제는 그대로 두고 재활성화 시 프로필을 다시 만들어 붙인다. 거부: (1) 삭제된 시점에 `claude_session_id`·대화가 이미 유실돼 재생성해도 이어받을 게 없다(빈 새 대화 = ADR-0082 가 폐지한 fresh-fallback 재발) (2) 근본 원인(자동 삭제)을 놔두고 증상만 덮는 우회다.
- **명시적 삭제 명령도 폐지(영구 누적만)** — 아무것도 못 지우게. 거부: 사용자 원안이 "새로 만드느니 **삭제하고** 다시 만드는 게 좋음" — 삭제는 사용자가 **의도적으로** 하는 행위로 남긴다(자동 삭제만 폐지). `AgentCommand::DeleteProfile` 경로 유지.

## 근거
- **유저 실측:** 트리 종료 → 우클릭 재활성화 → "실패" 즉시 재현. `agents.json` = `{"profiles": []}`(종료 후 프로필이 실제로 삭제된 실물 증거).
- **코드 확정:** `reaper.rs::decide` `(UserKill,_)=>DeleteProfile` / `(None,Exited{0})=>DeleteProfile` → `apply_disposition` → `profiles.remove`. `connection_core.rs::SpawnProfile` 의 `profiles().get()==None → Err("profile not found")`.
- **사용자 원안(ADR-0082 계승):** "아무것도 죽지마. 세션 터져도 새로 만들지마. 새로 만드느니 삭제하고 다시 만드는 게 좋음. 시체로라도 남겨." → 자동 삭제 전면 폐지 + 삭제는 수동. 이번 세션에서 보존 범위(UserKill·code0 둘 다)를 사용자가 명시 결정.
- **§5 정합:** 종료 원인을 시체+`auto_restore=false` 로 보존해 LLM(두뇌)이 상태를 읽고 재활성화/삭제를 판단(사람은 보조). 시스템이 사용자 모르게 프로필을 지워 판단 근거를 없애지 않는다.

## 영향 / 불변식
- **reaper `decide` 단순화** — `shutting_down => KeepAsIs`, 그 외 모든 종료 → `KeepDisableAutoRestore`. `Disposition::DeleteProfile` 은 reaper 가 더는 산출하지 않는다(이제 크래시와 유저 kill/정상 exit 이 같은 처분 = 프로필 보존, `auto_restore=false`). ADR-0019 §결정 1·§결정 표의 "의도된 종료=삭제" 만 폐지하고 나머지(intent frozen snapshot·epoch 검증·shutting_down suppress·auto_restore 수명)는 유지.
- **삭제 단일 경로** — 프로필 삭제 = `ProfileRegistry::remove` 를 부르는 **명시적 사용자 명령**(`AgentCommand::DeleteProfile`/Tauri `delete_profile`)뿐. reaper 는 세션 수거·`auto_restore` 다운그레이드만.
- **ADR-0082 완결** — resume 실패 시체 보존이 kill/정상exit 종료 경로에서도 성립. §열린항목 ②(code-0 갭) 닫힘.
- **누적 주의(수용된 트레이드오프)** — 모든 종료가 시체를 남기므로 프로필이 무한 누적될 수 있다. 사용자 정책상 의도된 동작이며, 정리는 명시적 삭제로 한다. 시체의 시각 표시(별도 "죽음/실패" 마커 vs 현행 예약 노드)는 ADR-0082 §열린항목 ①대로 **사용자 추후 판단**(이 ADR 범위 밖).
- **앵커:** `// ADR-0083` (`reaper.rs::decide`). **회귀 테스트(강제):** ① `decide(UserKill) == KeepDisableAutoRestore`(기존 `decide_user_kill_deletes` 갱신) ② `decide(Exited{0}) == KeepDisableAutoRestore`(기존 `decide_clean_exit_deletes` 갱신) ③ 통합: 에이전트 kill → 프로필 보존(`profiles.get(id).is_some()` + `auto_restore==false` + `claude_session_id` 유지) → 재활성화가 `profile not found` 없이 resume 진입.
