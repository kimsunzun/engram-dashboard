# ADR-0082: 활성화=이어받기(resume) 전용 — fresh-fallback 폐지, 실패는 Failed(시체)+원인 로그, LLM 에이전트가 분석·에스컬레이션

- 상태: 확정 (2026-07-14, 근거: 정상 실행 중 에이전트 재활성화 → 터미널·JSON 둘 다 Failed+빈 슬롯 유저 실측 + a4aac1a 체인 코드/git 확정)
- 관련: Supersedes ADR-0077 · Amends ADR-0008 (resume 조기종료 → fresh-fallback 조항 폐지) · Amends ADR-0076 (fallback_fresh 관련 불변식·"fresh-fallback 유효" 문구 폐지) · `agent/manager.rs::activate_profile`/`resume_with_fresh_fallback`/`fallback_fresh` · `daemon/connection_core.rs::SpawnProfile` · CLAUDE.md §5(LLM-우선 제어) · step-log

## 맥락
ADR-0008/0076/0077 이 확정한 **fresh-fallback**(resume 가 실패·조기종료하면 새 sid 로 빈 새 대화를 자동 시작)을 사용자가 번복한다.

두 겹의 문제가 드러났다.
1. **회귀(a4aac1a, ADR-0077):** fresh-fallback 을 수동 활성화 경로(`activate_profile` → `resume_with_fresh_fallback`)로 확장하면서, **정상 실행 중(대화 있는) 에이전트를 재활성화하면** `spawn_agent` 의 이중-spawn 가드가 돌려주는 "already running" Err 를 **"resume 실패"로 오인** → `fallback_fresh` 가 **멀쩡히 돌던 에이전트를 kill** → epoch++ → 빈 fresh 로 교체. 결과: 터미널·JSON 에이전트 둘 다 `Failed` + 슬롯 빈 화면(유저 실측 스크린샷).
2. **정책 자체:** 설령 재활성화 회귀만 국소 수정해도, "resume 실패 시 시스템이 사용자 모르게 대화를 파괴하고 빈 새 대화로 갈아엎는" 근본 동작이 남는다. 이는 (a) 대화 맥락을 유실시키고 (b) 실패 원인을 자동복구 뒤에 은폐해 진단을 막으며 (c) CLAUDE.md §5(LLM 이 메인 조작 주체 — 원인을 노출해 LLM 이 판단)와 배치된다.

## 결정
- **활성화(activate) = 이어받기(resume) 전용.** 이어받을 세션이 실패/조기종료/삭제/손상이어도 시스템이 **임의로 새 대화(fresh)를 만들지 않는다.**
- **실패 = 종점(terminal)으로 끝내고 프로필은 시체로 보존한다.** 아무것도 kill 하지 않고, 새로 만들지도 않는다. resume child 는 스스로 종료(주로 claude 가 "No conversation found" 로 exit code≠0)하고 reaper 가 그 세션을 맵에서 수거하되 **프로필은 삭제하지 않고 `auto_restore=false` 로 내려 시체로 보존한다**(사용자가 조치할 때까지 멈춤). **시체의 시각 표시(별도 "죽음" 마커를 띄울지 vs 현행 예약 노드로 둘지)는 사용자가 실물을 보고 추후 판단한다 — 이 ADR 은 "안 죽이고·안 새로 만들고·프로필 보존·원인 로그"까지만 확정하고 표시 방식은 열어 둔다.**
- **실패 원인을 로그로 남긴다** — claude 종료 사유(예: "No conversation found with session ID …")·조기종료 감지를 로그에 명확히 기록한다.
- **제어 LLM 에이전트가 그 로그를 분석해 사용자에게 에스컬레이션한다**(§5 — LLM 이 메인 조작 주체, 사람은 보조). 재생성 여부는 **사용자가 수동으로** 결정한다(시체 삭제 + 재생성).
- **살아있는 에이전트 재활성화 → 아무것도 죽이지 않고 놔둔다**(이미 실행 중 신호만). pre-a4aac1a 무해 동작 복원.
- **진짜 신규 에이전트의 첫 시작(Fresh)은 유지** — 이어받을 게 없는 새 프로필을 사용자가 명시적으로 시작하는 것은 fallback 이 아니라 정상 생성이다(ADR-0076 "Fresh=새 sid" 유효). 걷어내는 것은 **"resume 실패를 자동으로 fresh 로 대체"** 하는 fallback 뿐이다.

## 거부한 대안
- **자동 fresh-fallback (ADR-0008/0076/0077 — 이 ADR 이 폐지)** — resume 실패 시 새 sid 로 fresh 자동 시작. 거부: (1) 사용자 모르게 살아있는/기존 대화를 빈 새 대화로 갈아엎어 대화 맥락 유실 (2) 실패 원인이 자동복구 뒤에 묻혀 진단 불가 (3) a4aac1a 가 이를 수동 활성화로 확장하며 산 에이전트 파괴 회귀 유발. 사용자: **"아무것도 죽지마. 세션 터져도 새로 만들지마. 새로 만드느니 삭제하고 다시 만드는 게 좋음. 시체로라도 남겨."**
- **이중-spawn 가드 Err 만 fresh-fallback 에서 제외(국소 수정)** — 재활성화 회귀만 막고 fresh-fallback 은 유지. 거부: "resume 실패를 자동 갈아엎기로 처리"하는 근본 정책이 남아, 빈/손상 세션 활성화 시 여전히 사용자 모르게 fresh 로 대체된다. 사용자 결정은 fresh-fallback **폐지 자체**다(회귀 패치가 아니라 정책 번복).
- **spawn 전 빈 세션 선제 감지 후 처리** — claude 세션 저장소를 미리 읽어 "대화 있음/없음"을 판정. ADR-0077 이 이미 거부(claude 내부 포맷 결합·백엔드 지식 격리 ADR-0004 침해). 반응형(resume child 조기종료 신호)이 견고하다 — 다만 그 신호를 이제 fresh-fallback 이 아니라 **Failed + 로그 + 에스컬레이션**으로 번역한다.

## 근거
- **유저 실측(스크린샷):** 정상 실행 중 터미널·JSON 에이전트를 재활성화하면 둘 다 Failed + 빈 슬롯. 코드+git 으로 a4aac1a 체인 확정(가드 Err → `resume_with_fresh_fallback` 오인 → `fallback_fresh` 가 산 에이전트 kill → epoch++ → 빈 fresh). 결정적 진입(가드 Err → 항상 fallback_fresh → 항상 산 에이전트 kill), 최종 Failed 여부만 fresh 생존에 따라 비결정.
- **사용자 원안(이전 세션):** "활성화는 이어받기만, 실패 원인은 로그로 알리고 에이전트가 분석해 사용자에게 에스컬레이션." ADR-0076/0077 이 이 원안과 어긋나게 fresh-fallback 을 확정했던 것을 바로잡는다.
- **§5 정합:** 시스템이 실패를 은폐·자동복구하지 않고 원인을 로그로 노출 → LLM(두뇌)이 판단 → 사용자 에스컬레이션. "손발/두뇌 분리" 멘탈모델과 한 몸.

## 영향 / 불변식
- **fresh-fallback 경로 제거·무력화** — `resume_with_fresh_fallback`·`fallback_fresh` 의 "실패 → 새 sid fresh 재spawn" 은 걷어낸다. resume 실패/조기종료는 **종점(주로 Exited code≠0)으로 직행**(자동 fresh 재spawn 없음), 프로필은 시체로 보존(reaper `KeepDisableAutoRestore`).
- **재활성화 가드** — `activate_profile`/`SpawnProfile` 핸들러가 이미 실행 중인 세션(`sessions.contains_key(id)`)을 재활성화하면 **kill/재spawn 없이 그대로 둔다**(이미 실행 중 신호). 이중-spawn 가드 Err 는 파괴 트리거가 아니다.
- **실패 원인 로그 = 제어 표면 입력** — resume 실패 사유를 로그로 남겨 LLM 에이전트가 읽고 에스컬레이션한다. 원인을 삼키면(옛 fresh-fallback) §5 위반.
- **신규 첫 시작(Fresh) 보존** — `new_session_id` 로 새 sid 발급하는 명시적 신규 생성은 유효(ADR-0076). 이건 fallback 이 아니다.
- **살아남는 상위 결정** — ADR-0008 sid 통제(`--session-id`/`--resume` 무손실 복원)·ADR-0076 활성화=resume·Fresh=새 sid·sid 발급 단일점(`spawn_agent`)은 유효. 이 ADR 은 그 위에서 **"실패 시 자동 fresh 대체"만** 걷어낸다.
- 앵커: `// ADR-0082` (manager.rs `activate_profile`·resume 실패 처리, connection_core.rs `SpawnProfile` 재활성화 가드).
- **회귀 테스트(강제):** ① 이미 실행 중 에이전트 재활성화 시 원본 에이전트 kill·재spawn 안 됨(epoch 불변 + spawn 카운트 불변) ② resume 실패 시 종점으로 남고(프로필 시체 보존·`auto_restore=false`) 새 대화(fresh) 안 생김(spawn 정확히 1회).
- **알려진 엣지·열린 항목(사용자 추후 판단):** ① 시체의 시각 표시(별도 "죽음" 마커 vs 현행 예약 노드) 미확정 — 사용자가 실물 보고 결정. ② claude 가 exit code 0 으로 조기종료하면 reaper 기존 disposition 정책상 프로필이 삭제돼 시체가 안 남는다(문서화된 "No conversation found" 는 code 1 이라 보존 — code-0 서브케이스는 잔여 갭). ③ 재활성화 가드의 cross-connection TOCTOU(동시 활성화 시 이중 spawn)는 HEAD 부터 있던 기존 레이스 — 이 ADR 범위 밖, 후속 과제.
