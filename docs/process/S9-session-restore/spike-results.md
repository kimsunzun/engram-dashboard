# S9 Spike 실측 결과 (claude 세션 ID 메커니즘)

실측: 2026-06-11, `claude -p`(비대화형) + `~/.claude/projects/` 파일 관찰. 격리 cwd `C:/temp/claude-spike`.
**※ 이번 데이터는 `-p` 비대화형 기준. TUI(대화형) 특유 동작은 미확인(하단).**

## 폴더/파일 구조 원리

```
~/.claude/projects/<cwd-치환>/<sessionId>.jsonl
```
- **디렉토리 = cwd 경로 문자 치환** (`/`·`:`·`\` → `-`). 해시 아님. 가역적이나 **대소문자·슬래시 구분**(`f--…`와 `F--…` 공존 확인) → 같은 폴더도 표기 다르면 세션 분실. **spawn 전 cwd canonicalize 필수.**
- **파일명 = 세션 ID(uuid).jsonl**, 첫 줄 `sessionId`와 일치.
- **resume 시 새 파일 안 생기고 기존 jsonl에 append.**

## 동작 실측 (전부 재현됨)

| 테스트 | 명령 | 결과 |
|--------|------|------|
| ID 지정 | `claude -p ... --session-id <U>` | exit 0, json `session_id`=U. **우리가 ID 통제** |
| **resume fork 여부** | `--resume <U>` | exit 0, **session_id=U 유지**, 새 파일 없음(append). **fork 안 함** |
| ID 재사용 | `--session-id <기존U>` | **exit 1** `Session ID … is already in use.` |
| 없는 세션 resume | `--resume <랜덤>` | **exit 1** `No conversation found with session ID: …` |

## 설계 확정 근거

1. **`--resume` fork 안 함** → fable이 "설계 성립 좌우"라던 spike #4 통과. **후퇴선(fresh-new-id) 불필요.**
2. **세션 ID 완전 통제** (`--session-id` 지정 + json 반환) → 비결정성 0.
3. **세션 변경 감지 = 파일 watch** (파일명=세션ID, 새 jsonl 생성) → 능동 동기화 실현 경로.
4. **fallback = 새 uuid** (기존 ID 재사용 exit 1로 불가 확인 → 설계대로).
5. **복원 실패 신호 명확** (exit 1 + stderr, Hang 아님).

## 미확인 — TUI(대화형) 특유 (PTY spike 필요)

우리는 `-p`가 아닌 **대화형 TUI**로 spawn. 세션 파일 메커니즘은 동일 가능성 높으나(같은 projects 경로) 다음은 PTY 실측 남음:
1. TUI 내 `/clear`·`/resume` 시 세션 ID 변경 여부 (사용자 질문)
2. fresh PTY(다른 크기)에서 `--resume` redraw
3. 신규 cwd 첫 spawn 시 trust/온보딩 프롬프트 → 복원 Hang 가능성

## 정리 필요
- 임시 데이터: `C:/temp/claude-spike` + `~/.claude/projects/C--temp-claude-spike` (실측용, 삭제 예정)
