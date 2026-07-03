# Review 바인딩 — engram

골격(`../flow.md`)이 "프로젝트 코드 불변식 체크리스트"·"프로젝트 QA 명령"·"프로젝트 결정 기록"이라 부르는 자리에 끼우는 **engram 전용 체크리스트·연동**이다. 골격은 어느 프로젝트나 동일한 범용 리뷰 엔진이고, 이 파일이 engram으로 바인딩한다.

> **정본 = 코드의 `// ADR-NNNN` 앵커 주석 + 각 ADR(`docs/decisions/`).** 이 파일은 리뷰 때 꺼내 쓰는 점검용 목록일 뿐 — 코드·ADR이 바뀌면 rot한다. 통째 복붙해 정본으로 착각하지 말고, 의심되면 `rg "ADR-"`로 코드 앵커를 확인한다.

## code 단계 게이트 — 우리 불변식 (골격 §2 code 행 "체크리스트")

code 단계 Adversary(doc-aware breaker)는 다음 불변식 위반을 공격 표면으로 삼는다. **doc-aware 렌즈에만 준다**(blind 렌즈엔 주지 않는다 — 앵커링 차단).

- **kill 인과(2동사)** — `transport.shutdown()`(child.kill+wait → TerminateJobObject → master drop) → `core.join_pump(5s)`. master drop → reader EOF → pump break → `core.finish` → done_tx. (ADR-0001)
- **finalize 1회** — `OutputCore.finalized.swap(AcqRel)` — terminal 전이/알림 정확히 1회(pump 단독). (ADR-0005)
- **락 순서** — sessions RwLock은 Arc clone 후 즉시 해제 → 그 뒤 내부 접근. status lock 보유 중 외부 호출 금지. emit은 subscribers clone 후 lock 미보유 send. (ADR-0006)
- **상태 알림 분담** — 과도기 `Exiting`=manager, terminal(`Killed`/`Exited`/`Failed`)=pump 단독. 프론트는 `agent-list-updated`로 terminal 판정(status_changed로 판정 금지). (ADR-0005)
- **epoch 재구독** — 같은 AgentId 맵 교체(restart/fresh fallback)마다 +1 → 프론트 `[agentId, epoch]` 재구독. (ADR-0007)
- **replay→live** — subscribers lock 보유 중 replay 전송(순서 역전 방지) + 프론트 seq dedup.
- **코어 tauri import 0** — 코어 crate는 Tauri import 금지(ADR-0003). 격리 위반이면 코어가 전송 방식에 묶인 것 = 회귀.
- **관찰성/로깅** — load-bearing 경로가 로그 규약(`docs/reference/logging-conventions.md`) 준수 계측됐나. 위반(무계측·레벨오용·토큰 로깅) FIX. **판정 기준·계측 의무는 그 문서가 정본**(여기선 가리키기만).

근거·거부 대안 상세는 CLAUDE.md "핵심 불변식" 섹션과 각 ADR. **이 목록이 정본이 아니다 — 코드의 `// ADR-` 앵커가 단일 출처다.**

## QA 실측 게이트 (골격 §5)

리뷰 판정과 무관하게 항상 돈다. **build/test·GUI 실측 실명령은 qa 스킬 바인딩(`../../qa/references/bindings/engram.md`)이 정본** — 여기 베끼지 않는다. 이 절은 정책만 남긴다:

- build/test·GUI 실측을 강도별로 돌린다(실명령 = qa 바인딩 §강도별 실명령).
- **코드(test/tsc) PASS ≠ 동작 보장** — 화면·동작이 걸린 변경은 GUI 실측까지 가야 동작 확인이다.

## doc-aware 컨텍스트 준비 (골격 §1·§3)

doc-aware 렌즈(trd·code·doc 단계)에 줄 ADR·불변식 묶음을 미리 추린다. 위 "code 단계 게이트" 목록 + 닿은 코드의 `// ADR-` 앵커(`rg "ADR-"`)가 출발점이다. 로깅이 걸린 변경이면 `docs/reference/logging-conventions.md`도 넣는다(레벨·계측 의무·보안 준수 판정용). **blind 렌즈에는 주지 않는다.**

## 결정 기록 (골격 §6)

- 굵은 설계 결정 → ADR(`docs/decisions/`, 인덱스 `README.md`). 번복은 *폐기당한* ADR에 `폐기 (Superseded by ADR-NNNN)` 박기.
- 흐름(언제/무엇) → `docs/process/step-log.md`.
- **스킬·리뷰어는 기록하지 않는다 — 메인(오케스트레이터)이 처리한다.**
