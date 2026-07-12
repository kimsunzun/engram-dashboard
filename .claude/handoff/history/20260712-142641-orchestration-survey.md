# 핸드오프: ADR-0069 slice③ 완료·푸시 + 오케스트레이션 OSS 서베이 자료 완료·로컬커밋(미푸시)

## 한 줄 상태 · 다음 첫 액션
- **상태:** ① ADR-0069 UI 문자열 중앙화 **3슬라이스(그릇·command·컴포넌트) 전부 완료·푸시**(slice③ = `b982b02`). ② 오케스트레이션 **설계-결정 OSS 서베이 자료 완료·로컬 커밋**(`27844b8`, **미푸시 ahead 1**). 자료지 결정 아님.
- **다음 첫 액션(택1 — 사용자 결정):** (a) **미푸시 커밋 `27844b8` 푸시**(master 직접이라 명시 승인 필요 — auto-mode가 막음) → (b) 사용자가 서베이 §6 옵션 3개 중 방향 선택 → TRD + ADR-0014 확정/후속 → `/implement` · 또는 (c) **트리·프리셋 고도화 착수**(사용자가 "다음에 이거 할 것"이라 명시 — slice③ 아니라 이것이 원래 다음 실작업).

## 무엇이 됨 (repo 상태)
- **`b982b02`(푸시됨)** — ADR-0069 slice③: `src/components/` 37 문자열 t() 마이그레이션(2패스: 스캔23 + 놓친한글 sweep14). review deep 3인 + qa full(cdp) 통과.
- **`27844b8`(로컬만·미푸시)** — `docs/research/orchestration-survey-2026-07-12.md`(오케스트레이션 서베이) + step-log + ADR-0014 back-pointer.
- **브랜치 master, `origin/master` 대비 ahead 1**(27844b8 미푸시). 워킹트리 클린. remote = github.com/kimsunzun/engram-dashboard.
- **사용자 정정(중요):** 트리·프리셋 고도화는 **미완** — 원래 다음 실작업. 이번 세션은 외출 전 "오케스트레이션 리서치→자료로 남기기"만.

## 오케스트레이션 서베이 요지 (다음 세션 재료 — `docs/research/orchestration-survey-2026-07-12.md`)
- **방법:** `/research design-decision deep` — 수집자 5명 병렬(sonnet) → 메인 grounding → Codex(cross-family blind, high) 적대 리뷰(초판 BLOCK 12건→v2 반영).
- **핵심 결론(자료):** engram이 오케스트레이션 하부 절반 보유(reaper·epoch·S9사다리·예약필드 `RestartPolicy`/`restart_count`·이벤트버스·command registry 골격). 감독(Layer A) 정답 = **A0 네이티브 baseline**(tokio+기존PTY/reaper+직접 supervisor, 프레임워크 아닌 OTP/Ractor 패턴 차용) + **프로세스 트리 격리**(engram 이미 Windows Job Object 보유·ADR-0001). 내구성 엔진(Temporal/Restate)=현규모 과함. 조율=코드소유 중앙 오케스트레이터(Claude Code Workflow형). 통신(A2A)=로컬 과함.
- **옵션셋 §6(배타 아님·순서문제):** 1 감독우선 최소증분(OnCrash 자동재시작 — ADR-0019 §후속2) / 2 중앙 오케스트레이터(태스크그래프) / 3 메시징우선. **권장순서 1→2→(3 필요시).**
- **관련 기존 문서(중복조사 회피):** `docs/research/control-surface-and-fleet.md` · `docs/research/llm-control-surface-message-command-scope-2026-06-28.md` · ADR-0022/0055(command registry)·0019(reaper·자동재시작 게이트)·0028(이벤트버스).

## 검증 상태 (쌍)
- **돌린 것:** slice③ = `npx tsc --noEmit` clean · `npm test`(vitest **530**) · cdp 실측(포트9223, t() 반환값 byte-identical + DOM 렌더). **재실행:** `npx tsc --noEmit` + `npm test`.
- **검증 안 된 것:** 오케스트레이션 서베이 = **문서만(코드 로직 0 변경)** — 검증 대상 없음. 서베이 §8 미검증분(라이브러리 버전 숫자·Unix side 손자프로세스 fork 시 process-group 격리 현황)은 **채택 전 재확인 필요**(자료 자체는 결정 아니라 무해).

## do-not / 주의
- **오케스트레이션은 결정 아니라 자료** — 임의 채택·구현 진입 금지. 사용자가 §6 옵션 방향 고른 뒤 TRD/ADR(순서 불변).
- **푸시는 master 직접이라 명시 승인 필요** — auto-mode classifier가 무단 push 차단(이번 세션 실측). `27844b8` 푸시는 사용자 확인 후.
- **동시 세션 주의:** 이 repo master에 타 세션 동시작업 가능(지속 경고). push 전 `git fetch`로 fast-forward 재확인(이번 세션 내내 ff였음).
- **bare `cargo test`·`-p engram-dashboard` = WebView2 0xc0000139 크래시**(member-scoped `-core`/`-protocol`만). slice③·리서치 모두 Rust 무관이라 해당 없음.
- **리뷰어 바인딩:** cross-family(blind) = `mcp__codex__codex`(effort high 명시 `config:{model_reasoning_effort:"high"}`, sandbox read-only, approval never). **SendMessage 툴 없음** → 코더/리뷰어 이어가기는 codex만 `codex-reply`(threadId), Claude 서브는 fresh 스폰 + 이전 산출 주입.

## 정지 조건 (다음 세션)
- **옵션 방향 선택 = 사용자.** 메인이 오케스트레이션 접근을 임의 확정하지 말 것.
- 트리·프리셋 고도화 착수 시 구체 범위(무엇을 고도화)를 사용자에게 먼저 확인(현재 미정의).
- `27844b8` 푸시 여부 = 사용자 확인.

## 미결 결정 (사용자)
- 다음 작업: 오케스트레이션 방향(§6 옵션) vs 트리·프리셋 고도화 vs 기타 로드맵(step-log "다음").
- `27844b8`(서베이 자료) origin 푸시 여부.

## 참조 (읽을 것만)
- `docs/research/orchestration-survey-2026-07-12.md` — §6 옵션셋 · §7 engram 자산지도(build-on vs 새로) · §8 한계.
- ADR-0014(`docs/decisions/0014-오케스트레이션-참조-후보.md`) — 후보·심화조사 back-pointer.
- step-log 최근 2항목(`docs/process/step-log.md`) — slice③ · 오케스트레이션 서베이.
