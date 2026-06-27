# AI 코딩 에이전트 세션 간 컨텍스트 관리 조사 보고서

**상태:** 완료  
**날짜:** 2026-06-27  
**방법:** Claude(Sonnet) 팬아웃 × 4 갈래(15~25회 검색 + WebFetch) + Codex BLIND 독립 교차 + 교차 대조·적대 검증  
**조사 대상:** Cursor, Aider, Continue.dev, OpenHands, Devin, GitHub Copilot Chat, Claude Code, Cline, Roo-Code  
**확신도 범례:** 확실 = 공식 문서 직접 확인 / 가능성 높음 = 복수 출처 합의 / 불확실 = 단일 출처 또는 추론

---

## 1. 도구별 발견

### Cursor

**컨텍스트 저장 방식:** 하이브리드. 대화 이력은 인메모리(세션 내). 영구 규칙은 파일 기반(`.cursor/rules/*.mdc`). Memories는 프로젝트별 로컬 저장, Cursor Settings에서 관리(파일 직접 노출 없음).

**글로벌/프로젝트 분리:** 있음.
- User Rules: 전역(모든 프로젝트, Cursor Settings에서 관리)
- Project Rules: `.cursor/rules/*.mdc` (리포 커밋, 팀 공유)
- Team Rules: Cursor 대시보드(조직 단위)
- 적용 우선순위: Team → Project → User, 앞이 충돌 시 우선

**핸드오프 지원:** 제한적. 명시적 "세션 요약 → 다음 세션 주입" 메커니즘 없음. Memories(자동 누적)와 `.cursor/rules/*.mdc`(항상 로드)가 세션 간 연속성 제공. 커뮤니티 패턴으로 `memory-bank/` 마크다운 방법론 존재(Cline과 유사).

**Rules 파일 계층:**
- `.cursor/rules/*.mdc` (신규 — 4가지 유형: Always Apply / Auto-attached / Agent-requested / Manual)
- `.cursorrules` (프로젝트 루트, legacy — deprecated)
- `AGENTS.md` (프로젝트 루트, 메타데이터 없는 대체 형식)
- `.mdc` 파일만 인식; `.md` 파일은 무시

**출처:** https://cursor.com/docs/rules.md, https://forum.cursor.com/t/rules-hierarchy-in-cursor/108589  
**확신도:** 가능성 높음 (공식 docs.md 직접 확인; Memories 저장 내부 구조는 불확실)

---

### Aider

**컨텍스트 저장 방식:** 파일 기반 + 인메모리 혼합.
- `.aider.chat.history.md`: 대화 이력 저장
- `.aider.input.history`: 입력 이력
- `.aider.llm.history`: LLM 트랜스크립트(옵션)
- 세션 중 repo map은 동적 생성(메모리)

**글로벌/프로젝트 분리:** 있음.
- `.aider.conf.yml` 검색 순서: 홈 디렉토리 → git 저장소 루트 → 현재 디렉토리 (나중에 로드된 파일이 우선)
- 규칙 파일 표준 이름 없음 — `CONVENTIONS.md` 등 임의 이름 사용, `--read` 또는 `aider.conf.yml`의 `read:` 로 로드

**핸드오프 지원:** 있음 (경량).
- `/save`: 현재 세션 파일 목록을 재구성 가능한 명령 파일로 저장
- `/load`: 저장된 파일 실행
- `/copy-context`: 현재 chat context를 마크다운으로 클립보드 복사 (웹 UI 전환용)
- `--restore-chat-history`: 이전 대화 이력 복원 (기본값 False)

**Rules 파일 계층:** 고정 이름 없음. 사용자 정의 마크다운 파일을 `--read` 또는 `read:` 설정으로 주입. `.aider.conf.yml`로 세션마다 자동 로드 가능.

**출처:** https://aider.chat/docs/config/aider_conf.html, https://aider.chat/docs/usage/commands.html  
**확신도:** 확실

---

### Continue.dev

**컨텍스트 저장 방식:** 파일 기반. 설정과 규칙은 모두 디스크 파일. 대화 이력은 IDE 확장이 관리(내부 구조 공식 문서에 미노출).

**글로벌/프로젝트 분리:** 있음.
- 전역: `~/.continue/config.yaml` (macOS/Linux), `%USERPROFILE%\.continue\config.yaml` (Windows)
- 프로젝트: `.continuerc.json` (전역 위에 merge/overwrite), `.continue/rules/` 디렉토리
- 프로젝트 규칙이 전역 설정보다 우선 (`mergeBehavior: overwrite` 설정 시)
- **주의:** `config.yaml` 사용 시 `.continuerc.json` 무시 버그 존재(2025.12 기준 미해결)

**핸드오프 지원:** 없음. 명시적 세션 요약·이전 메커니즘 없음. CLI(`cn --resume`)로 이전 대화 재개 가능.

**Rules 파일 계층:**
- `.continue/rules/*.md` (워크스페이스 루트 기준)
- 파일 frontmatter: `alwaysApply`, `globs`, `regex`, `description`
- `alwaysApply: true` → 항상 시스템 메시지 포함
- `alwaysApply: false` + globs → 파일 패턴 일치 시
- 전역 rules: `~/.continue/rules/` (공식 docs에 경로 미명시; 검색 결과에서 언급)

**출처:** https://docs.continue.dev/customize/deep-dives/configuration, https://docs.continue.dev/customize/deep-dives/rules  
**확신도:** 가능성 높음 (`.continuerc.json` 버그는 GitHub 이슈로 확인)

---

### OpenHands

**컨텍스트 저장 방식:** 파일 기반(이벤트 로그). `persistence_dir/<conversation_id>/` 구조.
- `base_state.json`: 핵심 상태
- `events/event-<seq>-<id>.json`: 개별 이벤트 JSON
- 기본 위치: `workspace/conversations/`, 자기호스팅은 `~/.openhands`

**글로벌/프로젝트 분리:** 있음.
- 항상-로드 컨텍스트: 리포 루트 `AGENTS.md` (시스템 프롬프트 주입)
- 사용자 레벨 스킬: `~/.agents/skills/`
- 프로젝트 레벨 스킬: `.agents/skills/` (리포 내, 프로젝트가 우선)
- deprecated: `.openhands/skills/`, `.openhands/microagents/`

**핸드오프 지원:** 없음 (명시적 handoff 아티팩트 미지원). 동일 `conversation_id`로 재개하면 이벤트 로그 기반 전체 상태 복원 가능(세션 resume ≠ 핸드오프).

**Rules 파일 계층:**
- `AGENTS.md` (프로젝트 루트) — 모델 불문 항상 로드
- `GEMINI.md`, `CLAUDE.md` — 모델 특화 변형 지원
- `.agents/skills/` → `.openhands/skills/` → `.openhands/microagents/` 순서로 탐색(deprecated 역방향)

**출처:** https://docs.openhands.dev/sdk/guides/convo-persistence, https://docs.openhands.dev/openhands/usage/microagents/microagents-overview  
**확신도:** 가능성 높음 (SDK 공식 docs 확인; 클라우드 제품 내부는 불확실)

---

### Devin

**컨텍스트 저장 방식:** 제품 관리형 하이브리드(클라우드). Knowledge = 세션 횡단 DB(저장 방식 내부 미공개). AGENTS.md = 파일 기반 리포 컨텍스트. 세션 로그 = 클라우드 VM.

**글로벌/프로젝트 분리:** 있음 (가장 정교한 계층).
- 엔터프라이즈/조직 Knowledge: 전체 조직 공유
- 사용자 Knowledge: 개인, 특정 리포 고정 또는 전체 리포 적용
- 리포 Knowledge: `AGENTS.md` 파일 기반
- Knowledge 트리거(키워드): 관련 시 자동 회수

**핸드오프 지원:** 있음 (가장 명시적인 도구).
- `/handoff`: Devin CLI 내장 명령 — 리포/브랜치/미커밋 diff(100KB 이내)/에이전트 컨텍스트를 패키지화해 클라우드 Devin 세션 시작
- Devin Handoff Plugin: 타 에이전트(Claude Code, Codex, Cursor 등)에서 Devin으로 전환 지원
- 전달 내용: `git remote`, `git rev-parse` 감지, `git diff HEAD`, 에이전트 학습 컨텍스트

**Rules 파일 계층:**
- `AGENTS.md` (프로젝트 루트, 시작 전 자동 로드)
- Knowledge 자동 수집 소스: `.rules`, `.mdc`, `.cursorrules`, `.windsurf`, `CLAUDE.md`, `AGENTS.md`
- 일반 `.md` 파일은 자동 수집 대상 아님

**출처:** https://docs.devin.ai/onboard-devin/agents-md, https://docs.devin.ai/onboard-devin/knowledge-onboarding, https://docs.devin.ai/work-with-devin/devin-handoff  
**확신도:** 확실 (공식 docs 직접 확인)

---

### GitHub Copilot Chat

**컨텍스트 저장 방식:** 파일 + SQLite DB 하이브리드.
- `~/.copilot/session-state/<sessionId>/`: 체크포인트 파일, `plan.md`, `files/`
- `~/.copilot/session-store.db`: SQLite DB (sessions, turns, session_files, checkpoints 테이블)
- 세션 동기화: 기본값 GitHub 클라우드 동기화, 로컬 전용 설정 가능
- Copilot Chat(IDE)은 별도 저장 구조(공식 문서 미노출)

**글로벌/프로젝트 분리:** 있음.
- 개인 instructions: GitHub 계정 설정 (모든 프로젝트)
- 리포 instructions: `.github/copilot-instructions.md` (리포 전체)
- 경로별 instructions: `.github/instructions/NAME.instructions.md` (glob 매칭)
- 조직 instructions: Org 수준 설정
- 우선순위: 개인 > 리포 > 조직 (모두 포함되어 전달)
- JetBrains/Xcode 전역 파일: macOS `~/.config/github-copilot/intellij/global-copilot-instructions.md`

**핸드오프 지원:** 있음 (CLI 기준). 동일 `sessionId`로 `resumeSession()` 호출 → 전체 대화 이력·도구 결과 복원. IDE Chat은 명시적 핸드오프 미지원.

**Rules 파일 계층:**
1. `.github/copilot-instructions.md` (리포 전체, 항상 로드)
2. `.github/instructions/NAME.instructions.md` (경로별, glob frontmatter)
3. `AGENTS.md` (에이전트 모드, 디렉토리 트리 가장 가까운 파일 우선)

**출처:** https://docs.github.com/en/copilot/how-tos/configure-custom-instructions-in-your-ide/add-repository-instructions-in-your-ide, https://docs.github.com/en/copilot/how-tos/copilot-sdk/use-copilot-sdk/session-persistence, https://github.com/github/copilot-cli/issues/3046  
**확신도:** 확실 (SDK 공식 docs + GitHub 이슈 직접 확인)

---

### Claude Code

**컨텍스트 저장 방식:** 파일 기반 + 로컬 Auto Memory.
- CLAUDE.md 파일들: 세션 시작마다 컨텍스트 윈도우에 주입
- Auto Memory: `~/.claude/projects/<project>/memory/` (MEMORY.md + 토픽 파일들)
- Auto Memory 저장 한도: MEMORY.md 첫 200줄 또는 25KB만 세션 시작 시 로드

**글로벌/프로젝트 분리:** 있음 (5계층).

| 범위 | 위치 | 공유 대상 |
|---|---|---|
| Managed Policy | macOS: `/Library/Application Support/ClaudeCode/CLAUDE.md` 등 | 조직 전체 (제외 불가) |
| User | `~/.claude/CLAUDE.md` | 개인 (모든 프로젝트) |
| Project | `./CLAUDE.md` 또는 `./.claude/CLAUDE.md` | 팀 (git 커밋) |
| Local | `./CLAUDE.local.md` | 개인 (현재 프로젝트, gitignore 권장) |
| Subdirectory | `./subdir/CLAUDE.md` | 해당 디렉토리 파일 읽을 때 lazy-load |

**로드 순서:** CWD에서 위로 디렉토리 트리 탐색; root→CWD 방향으로 연결(덮어쓰기 아님); 각 레벨에서 CLAUDE.local.md가 CLAUDE.md 뒤에 추가.

**핸드오프 지원:** 있음 (구조적).
- `/compact` 후 project-root CLAUDE.md 자동 재주입
- Auto Memory: Claude가 세션 중 학습한 내용 자동 저장
- `/memory` 명령: 로드된 instruction 파일 목록 + auto memory 토글 + 수동 편집
- `@import` 구문: CLAUDE.md에서 다른 파일 임포트 (재귀 4단계)
- `--restore` 플래그: 세션 ID로 재개(SDK 기준)

**Rules 파일 계층:**
1. Managed Policy CLAUDE.md (제외 불가)
2. `~/.claude/CLAUDE.md` + `~/.claude/rules/*.md` (사용자 전역)
3. `./CLAUDE.md` 또는`./.claude/CLAUDE.md` (프로젝트)
4. `./CLAUDE.local.md` (로컬 개인)
5. `.claude/rules/*.md` (path-scoped 가능, YAML frontmatter `paths:`)

Claude Code는 `AGENTS.md`를 직접 읽지 않음 → `@AGENTS.md`를 CLAUDE.md에서 import하거나 symlink로 연결.

**출처:** https://code.claude.com/docs/en/memory (공식 문서 전문 직접 확인)  
**확신도:** 확실

---

### Cline

**컨텍스트 저장 방식:** 파일 기반 + 확장 관리 상태.
- Memory Bank: `memory-bank/` 디렉토리 내 마크다운 파일들 (projectbrief.md, productContext.md, activeContext.md, systemPatterns.md, techContext.md, progress.md)
- Checkpoints: shadow git 저장소(실제 리포와 분리) — 파일·작업 롤백용
- 규칙: `.clinerules/` 디렉토리

**글로벌/프로젝트 분리:** 있음.
- 전역 rules: Cline 시스템 Rules 디렉토리 (IDE 확장 설정)
- 워크스페이스 rules: `.clinerules/` 디렉토리 (워크스페이스 우선)
- 전역 AGENTS.md: `~/.agents/AGENTS.md` 지원

**핸드오프 지원:** 있음 (방법론적).
- Memory Bank는 내장 기능이 아닌 커스텀 지시 패턴 — `.clinerules/memory-bank.md` 또는 시스템 프롬프트로 구현
- 핸드오프 절차: "update memory bank" 요청 → `activeContext.md` 갱신 → 새 대화 시작 → "커스텀 지시 따르라" 요청 → 모든 Memory Bank 파일 읽기
- Auto Compact: 자동 컨텍스트 압축 + 수동 체크포인트 업데이트 조합

**Rules 파일 계층:**
- `.clinerules/*.md` 또는 `.txt` (워크스페이스, 우선)
- 전역 Cline Rules (IDE 확장 설정에서 관리)
- 자동 인식: `.cursorrules`, `.windsurfrules`, `AGENTS.md`, `~/.agents/AGENTS.md`
- Conditional rules: YAML frontmatter `paths:` (프롬프트 경로, 열린 탭, 편집 파일 기반 활성화)

**출처:** https://docs.cline.bot/features/memory-bank, https://docs.cline.bot/features/custom-instructions  
**확신도:** 확실

---

### Roo-Code

**주의:** 2026년 5월 15일 리포 아카이브 (읽기 전용, 버그 수정·모델 업데이트 중단).

**컨텍스트 저장 방식:** 파일 기반 + IDE 확장 관리 상태.

**글로벌/프로젝트 분리:** 있음.
- 전역: `~/.roo/rules/`, `~/.roo/rules-{modeSlug}/` (Linux/macOS), `%USERPROFILE%\.roo\rules\` (Windows)
- 워크스페이스: `.roo/rules/`, `.roo/rules-{modeSlug}/` (전역 후 로드, 충돌 시 워크스페이스 우선)

**핸드오프 지원:** 제한적. 명시적 핸드오프 아티팩트 없음. Cline 계열이므로 Memory Bank 방법론 적용 가능.

**Rules 파일 계층 (로드 순서):**
1. 전역 rules (`~/.roo/rules/`)
2. 워크스페이스 rules (`.roo/rules/`)
3. `AGENTS.md` 또는 `AGENT.md` (워크스페이스 루트, AGENTS.md 우선)
4. 레거시 fallback: `.roorules`, `.clinerules` (위 디렉토리에 내용 없을 때만)
- 모드별 rules (`rules-{modeSlug}/`)는 일반 rules보다 먼저 로드

**출처:** https://roocodeinc.github.io/Roo-Code/features/custom-instructions  
**확신도:** 확실 (단, 아카이브 시점 이후 변경 없음)

---

## 2. 교차검증표 (Claude ↔ Codex)

| 항목 | Claude 조사 | Codex 조사 | 판정 |
|---|---|---|---|
| Cursor rules 파일 위치 | `.cursor/rules/*.mdc` + legacy `.cursorrules` | 동일 | 수렴 |
| Copilot CLI SQLite 저장소 | `~/.copilot/session-state/` 파일 기반 | `~/.copilot/session-store.db` SQLite 추가 | 보완 — GitHub 이슈로 SQLite 확인 |
| Aider `/save`, `/copy-context` | `/save` 세션 재구성, `/copy-context` 마크다운 출력 | 동일 + `/load` 명시 | 수렴 |
| OpenHands 핸드오프 | 없음 | 없음 | 수렴 |
| Cursor Memories 저장 | 프로젝트별 로컬, Settings 관리 | 제품 관리형, 파일 직접 노출 없음 | 수렴 (저장 내부 구조 불확실) |
| Devin `/handoff` | CLI 명시 지원, git diff 패키지 | 동일, git diff 100KB 한도 명시 | 수렴 |
| Claude Code 5계층 구조 | 공식 문서 전문 확인 | 동일 | 수렴 |
| Cline Memory Bank = 방법론 | 내장 기능 아님, 커스텀 지시 패턴 | 동일 | 수렴 |

**불일치 해소:** Copilot SQLite는 GitHub 이슈(#3046)로 실제 존재 확인됨 — Codex가 옳음. Cursor Memories 내부 저장 방식은 두 family 모두 불명확(공통 공백).

---

## 3. 핵심 패턴 요약

### 저장 방식별 분류

| 도구 | 주 저장 방식 |
|---|---|
| Cursor | 파일(rules) + 제품 관리(Memories) |
| Aider | 파일(`.aider.*.md`, `.aider.conf.yml`) |
| Continue.dev | 파일(config.yaml, `.continue/rules/`) |
| OpenHands | 파일(이벤트 로그 JSON) |
| Devin | 클라우드 DB(Knowledge) + 파일(AGENTS.md) |
| GitHub Copilot | 파일(session-state/) + SQLite(session-store.db) + 클라우드 |
| Claude Code | 파일(CLAUDE.md) + 로컬 파일(auto memory MEMORY.md) |
| Cline | 파일(`.clinerules/`, `memory-bank/`) |
| Roo-Code | 파일(`.roo/rules/`) |

### 명시적 핸드오프 지원 여부

| 도구 | 핸드오프 지원 | 방식 |
|---|---|---|
| Devin | 있음 | `/handoff` CLI — git diff + 에이전트 컨텍스트 패키지 |
| Aider | 있음 (경량) | `/save`(파일 재구성) + `/copy-context`(마크다운 복사) |
| Cline | 있음 (방법론) | Memory Bank — `activeContext.md` 수동 갱신 |
| Claude Code | 있음 (구조적) | Auto Memory 자동 누적 + CLAUDE.md 재주입 + `/compact` |
| GitHub Copilot | 있음 (CLI) | `sessionId`로 `resumeSession()` — 전체 이력 복원 |
| Cursor | 제한적 | Memories 자동 누적; 명시적 handoff artifact 없음 |
| Continue.dev | 없음 | — |
| OpenHands | 없음 | conversation_id resume만(핸드오프 아님) |
| Roo-Code | 없음 | — |

### rules 파일 표준 파일명 비교

| 파일명 | 인식 도구 |
|---|---|
| `AGENTS.md` | Cursor, Devin, GitHub Copilot, OpenHands, Cline, Roo-Code |
| `CLAUDE.md` | Claude Code (native), Devin, OpenHands |
| `.cursorrules` | Cursor (deprecated), Devin, Cline (자동 인식) |
| `.cursor/rules/*.mdc` | Cursor (현재 표준) |
| `.github/copilot-instructions.md` | GitHub Copilot Chat |
| `.clinerules/` | Cline (native), Roo-Code (fallback) |
| `.roo/rules/` | Roo-Code (native) |
| `.continue/rules/` | Continue.dev (native) |
| `CONVENTIONS.md` 등 임의 파일 | Aider (`--read`로 명시 로드) |

---

## 4. 공백 및 한계

- **Cursor Memories 내부 저장 위치:** 공식 문서에 파일 경로 미노출. 설정 UI 기반 관리만 확인 (불확실).
- **Continue.dev 글로벌 rules 경로:** `~/.continue/rules/` 언급은 검색 결과 기반; 공식 docs에 명시 없음 (불확실).
- **Copilot Chat(IDE) 저장 구조:** SDK 기준 `session-state/` + SQLite는 확인됨. VS Code 확장 내부 chat 저장 구조는 미노출.
- **OpenHands 클라우드 제품 Knowledge:** SDK 기준 이벤트 로그만 확인; hosted 제품 내부 DB 구조 미공개.
- **Roo-Code:** 2026년 5월 이후 유지보수 중단 — 현재 사용자는 Cline 또는 Kilo Code 마이그레이션 권고됨.
