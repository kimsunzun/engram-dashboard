# ADR-0019: 세션 종료 분류 — 의도는 "행동 지점"에서, 프로필 disposition으로 죽음/예약/복원 가름

- 상태: 확정 (2026-06-16, dashboard4 세션 — 사용자 결정)
- 관련: ADR-0016(수명 모델 — restart_policy 런타임 해석 **일부 폐기**), ADR-0017(죽음 정의 — **구체화**), ADR-0001(kill 2동사), ADR-0005(finalize 1회), ADR-0015(데몬 persist)
- 범위: claude 세션이 "끝났을 때" 무엇을 죽음으로 보고, 프로필을 지울지 남길지, 어떻게 복원할지. 진행중 죽음과 부팅 복원을 분리한다.

## 맥락
세션이 끝나는 경로가 여럿이다 — 유저 kill / claude `/exit` / 크래시 / 데몬 강제종료. 이를 **종료를 관측(exit 신호)해서만 분류하면** 데몬 셧다운의 Job-kill이 유저 kill로 오분류되는 등 섞인다. 또 ADR-0016은 "런타임 재시작(restart_policy Always)"과 "부팅 복원"을 한 덩어리로 다뤘는데, 둘은 다른 로직이다.

## 결정
**원칙: 의도는 종료를 관측해 추론하지 않고, 종료를 일으킨 "행동 지점"에서 정한다.** 4채널로 분리:

| 종료 원인 | 감지 | 죽음? | 세션 | 프로필 | auto_restore |
|---|---|---|---|---|---|
| 유저 kill (UI/LLM 커맨드) | 커맨드 핸들러에서 의도 태깅 | 죽음 | 정리 | **삭제** | — |
| claude `/exit` (정상, exit 0) | PTY EOF + `Exited{0}`, 우리 kill 아님 | 죽음 | 정리(터미널 같이 종료) | **삭제** | — |
| claude 크래시 (exit≠0/signal) | PTY EOF + 비정상 | 아님(사고) | 정리 | **유지** | →false (예약 복귀) |
| 데몬/앱 강제종료·리부팅 | 데몬 셧다운 = death 기록 안 함 | 아님 | (데몬과 함께 끊김) | **유지** | 그대로 true → 부팅 복원 |

1. **핵심 규칙 = 프로필 disposition.** 의도된 종료(유저 kill·`/exit`)=세션 정리+**프로필 삭제**(완전 제거). 비의도(크래시·강제종료)=세션 정리+**프로필 유지**.
2. **크래시 → 예약(대기)로 복귀**(auto_restore=false). **런타임 자동 재시작 없음** — 사람이 수동 재실행. (즉시 `대기`로 보임.)
3. **auto_restore = "지금 떠 있어야 하는가".** 예약(깡통)=false, **활성화 시 true로**(강제종료 후 부팅 복원 대상이 되게), 크래시 복귀=false.
4. **데몬 셧다운은 자식 종료를 per-agent 죽음으로 기록 안 함**(shutting-down suppress). 그래서 강제종료된 running 들은 auto_restore=true 그대로 남아 **부팅 복원**된다. ↔ 크래시는 데몬이 살아있어 reaper가 돌아 auto_restore=false로 떨어뜨린다 → 두 경우가 "reaper가 돌았나"로 자연 구분.
5. **런타임 죽음 ≠ 부팅 복원** — 로직 분리. 부팅 복원은 ADR-0016 §부팅 복원(저장된 auto_restore=true를 1회 되살림) 유지.
6. **감지 = PTY EOF(현행).** `cmd /c claude` 구조라 claude 종료(=/exit·크래시) → cmd도 종료 → master reader EOF → pump finish. 별도 child watcher 불필요.

## 거부 / 대체한 것
- **ADR-0016 restart_policy=Always를 "런타임 자동재시작"으로 본 해석 → 폐기.** 런타임 자동재시작은 없다. 크래시는 예약 복귀(수동 재실행). "죽으면 되살린다"는 **부팅 복원** 맥락으로 한정. (ADR-0016 §결정 3의 런타임 의미만 supersede, 부팅 복원·가드 카운터·Failed 영속은 추후 재검토 — 이 ADR은 종료 분류에 집중.)
- **exit 신호만 관측해 분류** — 데몬 셧다운 Job-kill을 유저 kill로 오분류. 그래서 의도는 source(커맨드 핸들러/셧다운 경로)에서 태깅.
- **cmd /k(좀비 터미널)** — ADR-0017 유지(거부).

## 영향 / 불변식
- **ADR-0001(kill 2동사)·ADR-0005(finalize 1회) 무변경** — reaper는 기존 done 신호(`OutputCore.finish`→done_tx)를 **소비**해 sessions 맵에서 빼고 disposition을 적용할 뿐, kill 인과·finalize 횟수를 바꾸지 않는다.
- **유저 kill = 프로필 삭제**로 ADR-0016의 "kill≠삭제, pause 없음"을 이 모델에선 "kill=완전 종료=삭제"로 구체화(pause 없음은 유지 — 잠깐 안 쓰면 idle).
- "restart"(의도적 재시작 = 프로필 유지하고 재spawn)는 kill(삭제)과 **다른 별도 커맨드**로 둘 수 있다 — 필요 시 추후.

## 구현 정밀화 (consult 교차검증 `20260616-215346`, judge 최신뢰=GPT → TRD `reaper-trd.md`)
reaper 동시성 설계를 GPT·Gemini·Claude 블라인드 교차검증. 종합 결과 두 가지를 결정에 추가:
1. **의도(intent)는 finish 순간 snapshot.** reaper가 reap 시점에 intent를 live로 읽으면 "크래시로 죽은 뒤 reaper 처리 전 유저가 kill→크래시를 유저kill로 오분류→프로필 삭제(데이터 손실)" race가 생긴다(GPT 단독 적출, Gemini·Claude 놓침). 그래서 pump가 finish 승자일 때 intent·shutting_down을 **그 순간 snapshot해 종료 이벤트(ReapMsg)에 담아** 발행하고, reaper는 그 frozen 값으로 판정한다.
2. **reap 전 epoch 일치 검증(ADR-0007 재사용).** 단일 supervisor + `sessions.remove()` Some-승자로 idempotency를 보장하되, 늦게 도착한 옛 done이 같은 AgentId로 재spawn된 새 세션을 오삭제하지 않게 remove 전에 epoch를 비교한다(GPT·Claude 포함, Gemini 누락→좀비화 버그). 새 RunId 타입 신설 대신 기존 epoch 재사용.

세부 인터페이스·흐름·테스트는 `docs/process/S12-daemonization/reaper-trd.md`.

## 후속 (reviewer-deep 적출 — 데몬화 전 해결, 지금은 블로커 아님)
구현 후 reviewer-deep 검수에서 나온, 지금 단계엔 잠복이나 **데몬화(S12)·자동재시작(OnCrash) 도입 전 반드시 못 박을 것**:
1. **`shutting_down` 수명.** 현재 `shutdown_all`이 set한 뒤 false로 되돌리는 경로가 없다. AgentManager가 프로세스 수명과 1:1이면 무해하나, **데몬 상주 모델**(ADR-0013 tmux식)에서 shutdown_all이 "이 클라 정리"로 의미가 바뀌면 한 번 set된 플래그가 이후 모든 종료를 KeepAsIs(disposition 스킵)로 만든다. 데몬화 전 "manager 1회용 vs 재사용" 정책을 ADR로 확정.
2. **status 오분류(Killed vs Exited).** 유저 kill인데 watcher가 shutdown 플래그 set보다 먼저 child 죽음을 봐 master를 drop하면 pump가 `Exited{code}`로 status를 낼 수 있다. **disposition은 intent 우선이라 안전**(영향 0)하나, `RestartPolicy::OnCrash` 자동재시작이 status를 입력으로 쓰는 순간 분기가 틀어진다. OnCrash 도입 시 reason/status 분기에 intent를 반영할 것.
3. **watcher 단일화.** 자연종료 감지 watcher가 세션당 스레드(50ms 폴링)다 — reaper는 단일 supervisor인데 비대칭. 다중세션 확장성 위해 watcher도 단일 폴링 스레드로 합칠 것(장기).
4. **reaper 테스트 커버리지 갭.** epoch race(옛 ReapMsg가 재spawn된 새 세션 오삭제 방지)·idempotency가 unit decide + 구조 보장만 있고 통합 실측이 없다 — reap_one 수정 시 회귀 그물 보강 필요.
