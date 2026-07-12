# ⚠️ 임시 사본 — 정본 아님

이 폴더의 스킬들(adr·handoff·implement·qa·research·review + `_shared`)은 **원래 user 레벨 전역 스킬**이다. engram-dashboard repo를 단독으로 떼어 시연하기 위해 2026-07-12에 임시 복사했다.

- **정본(SSOT):** Engram repo `core/claude-global-shared/skills/` (`~/.claude/skills` 심링크로 전 프로젝트 로드). 제작·유지보수 작업장 = `agents/skill-factory`.
- **같이 복사된 사본:** `.claude/references/dictionary.md`(전역 사전) · `.claude/agents/worker-senior.md`(프리셋) — 같은 성격.
- **이 사본은 읽기 전용이다.** 스킬 문구·규칙 수정, feedback.md 누적을 여기에 하지 않는다 — 수정은 정본 경로(Engram)에서만. 여기 고치면 정본과 갈라져 유실된다.
- Engram 전역 심링크가 살아있는 PC에선 project 레벨인 이 사본이 user 레벨 정본을 shadow한다(동작 동일 전제 = 동기 사본일 때만).
- 시연 종료 후 이 폴더는 삭제 예정 — 삭제 전 여기 feedback.md에 누적분이 생겼는지만 확인해 정본으로 병합한다.
