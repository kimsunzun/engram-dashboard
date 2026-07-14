# ADR-0084: 재활성화(resume) epoch bump + apply_disposition epoch-guard — stale reap 산-세션 강등·프론트 재구독 누락 차단

- 상태: 확정 (2026-07-14, 근거: ADR-0083 리뷰에서 cross-family(codex) + deep lifecycle 렌즈 2인이 독립 수렴 적출 — `bump_epoch` 프로덕션 호출자 0개 + reap↔재활성화 auto_restore 레이스)
- 관련: Amends ADR-0019 (apply_disposition epoch-guard 추가 — stale reap 이 재활성화된 산 세션을 강등 못 하게) · ADR-0007(epoch 맵교체 재구독 — 재활성화 respawn 도 그 불변식 대상임을 재확인) · ADR-0083(자동 삭제 폐지 — 시체 재활성화 경로를 실제로 도달 가능하게 만들어 이 갭을 노출) · ADR-0082(fresh-fallback 폐지 — `fallback_fresh` 제거로 유일한 `bump_epoch` 호출자가 사라짐) · `agent/manager.rs`(spawn_agent epoch·activate_profile resume) · `agent/reaper.rs`(apply_disposition) · `src/components/slot/TerminalSlot.tsx`(구독 deps)

## 맥락
ADR-0083 이 "reaper 는 어떤 종료에도 프로필을 삭제하지 않는다(시체 보존)"로 바뀌면서, **종료된 에이전트를 재활성화(resume)하는 경로가 처음으로 실제 도달 가능**해졌다(이전엔 kill→프로필 삭제→재활성화 시 `profile not found`로 막혀 이 경로에 닿지 못했다). 그 경로를 리뷰(3인 적대, cross-family codex 포함)에서 적대 검증하니 두 결함이 드러났다 — 둘 다 **재활성화 시 epoch 가 오르지 않는다**는 한 뿌리에서 나온다.

1. **epoch 재사용 → 프론트 재구독 누락(ADR-0007 위반).** `spawn_agent` 은 epoch 를 프로필에서 **읽기만** 하고 올리지 않는다(`manager.rs`). 원래 respawn 마다 epoch 를 올리던 유일한 호출자는 fresh-fallback 의 `fallback_fresh` respawn 이었는데, ADR-0082 가 fresh-fallback 을 폐지하며 그 호출자를 지웠다 → `bump_epoch` 는 정의만 남고 **프로덕션 호출자가 0개**가 됐다. 그 결과 시체를 **같은 슬롯**에서 재활성화하면 새 세션이 죽은 세션과 **동일한 `[agentId, epoch]`** 를 갖는다. 프론트 구독 effect deps 가 `[viewId, agentId, epoch]`(ADR-0007)라 셋 다 안 바뀌면 재구독이 안 돌아 **resume 된 새 세션 출력이 화면에 안 붙고 옛 seq/cursor 상태가 새 스트림에 잘못 적용될 수 있다**. 즉 ADR-0083 이 "실패(profile not found)"를 없애도 재활성화가 **빈 슬롯**으로 끝나 사용자 목표("대화 이어받아 보기")에 닿지 못한다.
2. **reap↔재활성화 auto_restore 레이스.** reaper `reap_one` 은 sessions write-lock 안에서 세션을 제거·해제한 **뒤**, lock 밖에서 `apply_disposition`(= `auto_restore=false` 다운그레이드)을 쓴다. 그 사이 다른 커넥션이 재활성화하면 새 세션 insert + `auto_restore=true`(활성화 시 참, ADR-0019 §결정 3)가 먼저 실행되고, **뒤늦게 도착한 옛 reap 의 `apply_disposition` 이 그 산 세션을 `auto_restore=false` 로 강등**한다 → 데몬 재시작 시 부팅 복원에서 그 산 에이전트가 누락된다. sessions.remove 는 epoch-guard 로 이런 stale 처리를 막지만(ADR-0007/0019), `apply_disposition` 에는 그 가드가 없어 epoch 가 안 오르면 구분조차 불가능하다.

## 결정
1. **재활성화(resume)는 epoch 를 bump 한다.** reap 으로 세션이 맵에서 빠졌다가 재활성화로 새 세션이 같은 AgentId 로 들어오는 것은 **맵 교체**다 → ADR-0007 불변식("같은 AgentId 맵 교체마다 epoch +1")을 그대로 적용한다. resume respawn(그리고 향후 어떤 respawn 경로든)은 새 세션 생성 직전에 `bump_epoch` 를 호출해 `[agentId, epoch]` 를 바꾼다. 프론트는 바뀐 epoch 로 재구독하고 `terminal.reset()` 후 replay→live 를 새로 받는다.
2. **`apply_disposition` 에 epoch-guard 를 추가한다.** reaper 는 `ReapMsg.epoch` 를 disposition 적용까지 들고 가서, **현재 프로필/세션의 epoch 가 reaped epoch 와 일치할 때만** `auto_restore` 를 내린다(불일치 = 그 사이 재활성화로 epoch 가 올라간 새 세션 → 손대지 않음). sessions.remove 의 epoch-guard(ADR-0007/0019)와 같은 원리를 disposition 계층까지 확장한다.
3. **불변식 명문화:** "respawn(재활성화·resume 포함)마다 epoch++" 를 ADR-0007 의 respawn 사례로 명시하고, `bump_epoch` 는 더 이상 dead 가 아니라 재활성화 경로의 필수 호출점으로 앵커(`// ADR-0084`)한다.

## 거부한 대안
- **epoch 안 올림(현상 유지)** — 재활성화가 이어받기(resume)이니 "같은 대화 = 같은 epoch"로 두자. 거부: (1) resume 이어받기든 fresh 든 **맵에서 세션이 빠졌다 새로 들어오는 건 물리적 교체**라 프론트 구독 인스턴스가 갈린다(ADR-0007 의 "맵 교체" 정의에 부합) (2) 재구독이 안 돌아 빈 슬롯 + 옛 seq/cursor 오적용 (3) apply_disposition epoch-guard 를 세울 구분자 자체가 없어져 레이스도 못 막는다.
- **프론트가 `agent-list-updated` 목록 제거를 재구독 트리거로** — corpse 가 목록에서 빠지는 걸 프론트가 감지해 재구독. 거부: epoch 계약(ADR-0007)을 우회하는 프론트 땜질이라 백엔드 respawn 불변식과 계약이 두 갈래로 갈린다(rot). 또 레이스(#2)는 백엔드 문제라 프론트로 못 막는다.
- **apply_disposition 을 lock 안으로 이동해 레이스만 국소 차단** — epoch-guard 없이 disposition 을 sessions lock 안에서 수행. 거부: ADR-0006 락 순서(ProfileRegistry mutate=디스크 IO 는 lock 밖) 위반 + 재구독 누락(#1)은 여전히 안 풀린다. 두 결함이 한 뿌리(epoch 미증가)이므로 뿌리를 고친다.

## 근거
- **cross-family 수렴:** deep lifecycle 렌즈(Claude)와 blind 리뷰어(codex/GPT)가 **서로의 결과를 모른 채** epoch 재사용을 독립 적출 — 신뢰도 높음. codex 는 추가로 auto_restore 레이스(#2)를 별도 적출.
- **코드 확정:** `bump_epoch` 프로덕션 호출자 0(`rg bump_epoch` → 정의 + unit test 뿐, 과거 유일 호출자 `fallback_fresh` 는 ADR-0082 로 제거). `spawn_agent` 은 `profiles.get(...).epoch` 를 읽기만. `apply_disposition` 은 epoch 무관 `update_with`.
- **ADR-0007 정합:** 이 결정은 새 규칙 신설이 아니라 ADR-0082 가 실수로 끊은 "respawn 마다 epoch++" 불변식의 **복원 + 명문화**다. apply_disposition epoch-guard 만이 ADR-0019 reaper 정밀화에 대한 신규 amend.

## 영향 / 불변식
- **재활성화 = epoch++.** `activate_profile` 의 Resume 갈래(및 resume respawn 진입점)가 `bump_epoch` 를 부른다. 프론트 `[agentId, epoch]` 재구독(ADR-0007)이 시체 재활성화에서도 성립 → resume 출력이 화면에 붙는다.
- **apply_disposition epoch-guard.** `ReapMsg.epoch` 로 조건화 — 재활성화로 올라간 산 세션을 stale reap 이 강등 못 함. downgrade-only 성질(ADR-0019)은 유지, 조건만 추가.
- **`bump_epoch` 재활성(dead 아님).** `// ADR-0084` 앵커로 필수 호출점 표시 — 다음 세션이 "미사용"으로 오인해 지우지 못하게.
- **회귀 테스트(강제):** ① kill→재활성화 시 새 세션 epoch 가 죽은 세션보다 큼(epoch++ 단언) ② reap 후 재활성화(에폭 상이) 상황에서 stale `apply_disposition` 이 산 세션 `auto_restore` 를 강등하지 않음(epoch-guard) ③ claude 백엔드에서 재활성화 시 `build_command_spec(Resume, sid=Some)` 이 `--resume <sid>` 를 실제 조립(ADR-0083 회귀 ③의 약한 Shell 테스트 보강 — sid 전달 실증).
- **미해결(범위 밖):** 프론트 슬롯이 kill 후 corpse 의 agentId 를 계속 쥐는지(같은 슬롯 재활성화 트리거 조건)와 시체 vs 예약 시각 구분(ADR-0082/0083 §열린항목 ①)은 프론트 과제로 남긴다. 이 ADR 은 백엔드 epoch·disposition 계약을 고정한다.
