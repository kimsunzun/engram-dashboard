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

## TUI(대화형) 실측 결과 — wezterm 패인 + 파일 관찰

`claude --session-id <U2>` 를 TUI로 spawn하고 send-text로 조작, `~/.claude/projects/` 관찰.

| 항목 | 결과 |
|------|------|
| TUI에서 `--session-id` | ✓ `-p`와 동일 — 우리 지정 `<U2>.jsonl` 생성 |
| **`/clear`** | **새 세션 ID 생성!** 프로세스 그대로인데 `U2.jsonl` → 새 `<ae718820>.jsonl` 추가, 이후 대화는 새 파일에. **→ AgentId/session_id 분리 필수의 결정적 증거** |
| `/clear` 파일 감지 | 새 `.jsonl` **즉시 생성** → **파일 watch로 능동 감지 가능** (사용자 아이디어 실현 확정) |
| 신규 cwd trust 프롬프트 | **이 환경에선 안 뜸**(이미 신뢰됨, "Welcome back"). 단 완전 새 머신/계정에선 뜰 수 있음 — 운영 시 주의 |

### 남은 경미 항목 (위험 낮음)
- **fresh PTY redraw**: 우리 대시보드가 cols/rows를 관리(resize command)하므로 우리 책임 영역. claude는 받은 크기로 렌더 — 큰 위험 아님.
- **`/resume` 픽커**: `/clear`로 "TUI 내 세션 변경 → 새 파일" 메커니즘이 확인됐으므로 `/resume`도 동류. 파일 watch가 커버.

### release notes 주의 (관찰)
"Fixed sessions not saving transcripts (...) when launched from VS Code integrated terminal" — PTY/통합터미널 환경에서 세션 저장 안 되던 버그가 **수정됨**. 우리도 PTY spawn이라 **claude 버전 최신 유지**가 안전망(구버전이면 세션 미저장 위험).

## 정리 필요
- 임시 데이터: `C:/temp/claude-spike` + `~/.claude/projects/C--temp-claude-spike` (실측용, 삭제 예정)
