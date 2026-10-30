# 에이전트별 지시서 관행 — 공통 기본 + claude/codex 전용을 어떻게 가르나 (2026-10-30)

- **상태:** `/research` light — 수집자 1 · 메인 grounding 스팟체크(rulesync 저장소의 규칙 · 스킬 파일 앞머리를 `gh api` 로 직접 열어 확인) · 적대 리뷰 없음(light). 결정 대기 — S25 「지시서 · 스킬도 공통 / claude / codex 로 가르기」(사용자 제안 2026-10-30).
- **쓰임:** 프리셋 안 지시서 · 스킬 줄마다 「공통 / claude / codex」 표시를 붙이고 화면은 세 묶음 · 실행은 공통 먼저 그 백엔드 것 뒤 — 이 안이 관행과 맞나.
- **확신도 범례:** 확실 = 1차 문서 · 저장소 직접 확인 · 가능성 높음 = 1차 문서 요약 · 모름 = 못 찾음.

## 결론

1. **「공통 기본 + 에이전트별 예외」 는 있는 꼴이다.** 직접 피어 rulesync 가 규칙 파일과 스킬 파일 앞머리에 `targets` 를 둔다 — `["*"]` = 모든 도구 · `["claudecode"]` = 그 도구만(확실 — 저장소 `.rulesync/rules/coding-guidelines.md` · `.rulesync/skills/approve/SKILL.md` 직접 확인). **스킬에도 같은 장치를 쓴다.**
2. **표시를 다는 자리는 피어마다 다르다.** rulesync = 파일 앞머리(파일이 스스로 선언) · Roo Code = 폴더(`.roo/rules/` 공통 · `.roo/rules-{모드}/` 전용) · ruler = 에이전트 단위 설정뿐이고 내용별 표시는 없음. **프리셋 같은 묶음 쪽에 표시를 다는 선례는 못 찾았다.** 파일 이름 접미사(`guide.codex.md`) · 한 파일 안 칸 나누기 선례도 못 찾았다.
3. **합치기는 거의 전부 덮어쓰기가 아니라 이어 붙이기다.** 순서를 문서로 적은 곳은 Roo 하나이고 「전용 먼저 → 공통 나중」 이다(우리 안과 반대).
4. Cursor · Continue · GitHub Copilot 은 에이전트가 아니라 **적용 조건**(파일 경로 glob · 항상 적용 등)으로 가른다 — 대상 에이전트 개념이 없다.

## 피어별

| 피어 | 표시 자리 | 합치기 · 순서 | 스킬에도? |
|---|---|---|---|
| **rulesync** | 파일 앞머리 `targets: ["*"]` / `["claudecode"]` (확실) | 모름 — 도구마다 파일을 따로 만들어 내는 방식 | 예 — `SKILL.md` 앞머리에도 `targets` (확실) |
| **ruler** | 없음 — `.ruler/` 의 `.md` 를 전부 이어 붙이고 `ruler.toml` `[agents.X]` 로 켜고 끔만 (가능성 높음) | 이어 붙이기 · 루트 `AGENTS.md` 먼저 | `.ruler/skills/` 를 에이전트마다 전파 · 스킬별 대상 지정은 못 찾음 (실험 기능) |
| **Roo Code** | 폴더 — `.roo/rules/` · `.roo/rules-{모드}/` (확실) | 이어 붙이기 · 모드 전용 먼저 → 공통 나중 · 같은 폴더는 이름순 (확실) | 규칙만 |
| **GitHub Copilot** | 앞머리 `applyTo`(경로 glob) · `excludeAgent` 는 Copilot 안 표면 구분 (확실) | 이어 붙이기 · 개인 > 저장소 > 조직 | 모름 |
| **Cursor** | 앞머리로 적용 시점만(항상 · glob · 설명 · 수동) (확실) | Team → Project → User | 해당 없음 |
| **Continue.dev** | 앞머리로 적용 조건만 (확실) | 이름순 · `01-` 접두로 순서 | 해당 없음 |

출처:
- rulesync — https://github.com/dyoshikawa/rulesync (`.rulesync/rules/coding-guidelines.md` 앞머리 `root: false` · `targets: ["*"]` · `.rulesync/skills/approve/SKILL.md` 앞머리 `targets: - "*"` — 메인이 `gh api repos/dyoshikawa/rulesync/contents/…` 로 직접 확인 2026-10-30. README · 문서 사이트 첫 쪽에는 이 장이 없다)
- ruler — https://github.com/intellectronica/ruler
- Roo Code — https://roocodeinc.github.io/Roo-Code/features/custom-instructions
- GitHub Copilot — https://docs.github.com/en/copilot/how-tos/configure-custom-instructions/add-repository-instructions
- Cursor — https://cursor.com/docs/context/rules
- Continue.dev — https://docs.continue.dev/customize/deep-dives/rules

## 우리에게

| | (가) 프리셋 줄에 표시 (제안안) | (나) 파일 앞머리에 표시 (rulesync 꼴) |
|---|---|---|
| 선례 | 못 찾음 — 묶음 쪽 표시 | rulesync(직접 피어) |
| 같은 파일을 프리셋마다 다르게 | 된다 — 한 프리셋에선 공통 · 다른 프리셋에선 codex 전용 | 안 된다 — 파일이 한 번 정한다 |
| 받은 묶음(다른 루트 · G25)의 파일 | 손대지 않는다 | 남의 파일 앞머리를 고쳐야 한다 → 갱신 때 충돌 |
| 앞머리가 모델에 가는 문제 | 없음 | 지시서 `.md` 앞머리를 실을 때 걷어 내야 한다 · `SKILL.md` 에 모르는 키를 더하면 claude · codex 가 어떻게 받는지 미확인 |
| 순서 | 공통 먼저 → 전용 나중(제안) — Roo 와 반대 | 같은 선택이 남는다 |

## 공백 · 확인 못 한 것

- rulesync 의 공통 · 전용 합치기 순서 · `targets` 생략 시 기본값 — 문서에서 못 찾음.
- 파일 이름 접미사 · 한 파일 안 칸 나누기 선례 · CLAUDE.md 와 AGENTS.md 를 함께 만드는 다른 도구 — 검색하지 않았다(없다고 단정 못 함).
- Cline 의 규칙 폴더 — 직접 확인 안 함.
