# 핸드오프: continue 스킬 재설계 — append-only history + latest.md (양 repo 커밋 완료)

## 한 줄 상태 · 다음 첫 액션
continue 스킬을 save/load로 재설계하고 `.claude/continue/` 구조로 이관, 두 repo 커밋·푸시까지 완료. **다음 첫 액션:** 새 세션에서 `/continue`(load)를 쳐서 이 핸드오프(`latest.md`)가 떠 맥락이 복원되는지 확인 = load 실측(미검증).

## 변경/완료 + repo 상태
- 브랜치: 양쪽 `master` (engram-dashboard, Engram).
- **engram-dashboard `2f430c5` (pushed):** CLAUDE.md 핸드오프 규약 줄, `.gitignore`에 `.claude/continue/`, 리서치 보고서, new-skill feedback.
- **Engram(claude-global-shared) `7586fc3` (pushed):** `core/claude-global-shared/skills/continue/` — SKILL.md 재작성 + `references/flow.md` 신규 + `agents/openai.yaml` 제거.
- `.claude/continue/` = `latest.md` + `history/`(구 핸드오프 22개 이관 + 이 파일). `.ccb/` 삭제됨.
- 모델 opus 고정: `~/.claude/settings.json` `"model": "claude-opus-4-8"`.
- **미커밋(타 패널 작업 — 건드리지 말 것):** `src/lab/richslot/*`, `src/components/slot/TerminalSlot.tsx`, `src/lab/main.tsx` (dashboard1/dashboard-main 소관, RichSlot 랩).

## 검증 상태 (쌍으로)
- **돌린 것:** 양 repo `git commit`+`push` 성공(`2f430c5`, `7586fc3`). `/review trd`(opus Architect-breaker + Codex Designer) + 핸드오프 내용 light 리뷰(Codex) 반영 완료.
- **재확인 명령:** `git -C "I:/Engram/apps/engram-dashboard" log --oneline -1` · `git -C "I:/Engram" log --oneline -1`.
- **검증 안 된 항목:** `/continue save`는 **이게 첫 실행**(이 파일이 그 산출물). `/continue load` 실측은 **아직 안 함** → 다음 세션 첫 액션. 이번 변경은 문서·스킬뿐이라 cargo/npm 빌드·테스트는 안 돌림(코드 변경 0 — 의도된 생략).

## 실패한 접근 (do-not)
- continue를 **멀티스트림(패널 정체성/라벨 자동매칭)**으로 설계 → 폐기. 사용자가 "지금은 반수동·단일 스트림" 결정. **wezterm/panel/orchestra 개념을 골격에 넣지 말 것**(범용 위반).
- 핸드오프 **per-panel `<panel>.md` 덮어쓰기** 모델 → 폐기. append-only `history/` + `latest.md`로 확정.
- 설계 검토를 **deep research로 escalate** → 과함. 이런 스킬 설계는 new-skill 인터뷰 + medium research로 충분.

## 블로커/미결
- `.gitignore`의 `.ccb/` 줄이 vestigial(이미 폴더 삭제). 제거하려면 소커밋 1개 필요 — 사용자 미결정(무해해서 보류 중).

## 참조 (읽을 것만)
- `I:/Engram/core/claude-global-shared/skills/continue/SKILL.md` · `references/flow.md` (스킬 정본)
- `docs/research/session-handoff-continue-resume-research-2026-06-27.md` (설계 근거)
- `.claude/skills/new-skill/feedback.md` (new-skill 리서치 단계 부재 갭)
