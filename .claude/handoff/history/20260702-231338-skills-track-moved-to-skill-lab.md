# 핸드오프: 스킬 트랙 → agents/skill-lab 이주 완료 (dashboard 잔여 = JSON 트랙 + 푸쉬 대기)

## 한 줄 상태
스킬 리팩토링 트랙은 이 repo를 **떠났다** — 작업장 = `I:\Engram\agents\skill-lab`(Engram repo 정식 구성원), 그쪽 핸드오프 = `agents/skill-lab/.claude/continue/latest.md`. 스킬 작업을 이으려면 그 폴더에서 세션을 열 것.

## 이 repo에 남은 것
- **JSON 렌더 트랙(본류):** M1 `209eb84`(StdioTransport + stream-json seam, ADR-0044) · M2 `05e7d54`(실스트림 E2E 완주) — **병렬 세션이 진행 중**, 이 트랙 상태는 그 세션 핸드오프가 정본.
- **research 스킬 정본:** `.claude/skills/research/`(v3.2 다이어트본 — 게이트 통과 + medium 1회 실측 PASS). 조사 보고서: `docs/research/flaky-test-ci-gate-practices-research-2026-07-02.md`(qa 재설계 근거).
- `_wip/` = `shots/`(gitignore됨)만. 슬롯 UI 이주(트랙 D)는 여전히 이월.

## repo 상태
미푸쉬 커밋 5개(`89de415` flaky 조사 · JSON M1/M2 · `736c7cb` 이주 등) — 푸쉬는 사용자 승인 필요.

## 검증 상태
- 돌린 것: research dogfood 1회(위 보고서). JSON M1/M2 검증 상태는 병렬 세션 기록 참조(이 세션 관할 아님).
- 주의: 글로벌룰이 바뀜 — `## 위임 우선`(구 컨텍스트 위생 흡수, Engram `c3059a9`). 서브에이전트 스폰 시 모델 명시(Fable 워커 금지 — 사용자 결정).
