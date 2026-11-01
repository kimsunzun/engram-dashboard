# 핸드오프: S25 구현 — 2 · 4 단계 완료 push(CI 초록) · ADR-0339 확정 · 다음 = 새 기능(폴더 지정) PRD → 16 단계

## 한 줄 상태 · 다음 첫 액션
- 서버 1판 단계 1 · 2 · 3 · 4 완료 · 원격 = `f03ba511`(CI 초록 run 41289479530). 미커밋 코드 없음.
- **다음 첫 액션 = 사용자가 새로 요청한 기능의 PRD 단계부터**(사용자 2026-11-01 「중요한 기능이 하나 추가되야함」):
  - 원문: 「폴더 경로 지정하면 거기있는 폴더를 해당 에이전트 룰로 파싱하는거하고, 고정경로 저장하는거. 결국 Claude 나 codex 가 폴더 지정하는것과 같은 옵션임」
  - **메인 해석(사용자 미확인)**: ① 사용자가 폴더 경로를 지정하면 그 폴더 안 내용(스킬 · 지침 등)을 그 에이전트 백엔드 규칙대로 읽어 붙인다 — Claude `--add-dir` · codex 추가 디렉터리(extra roots) 같은 성격 ② 그 경로를 고정으로 저장(에이전트 또는 프리셋에 — 어디에 둘지 미정).
  - 진행 = CLAUDE.md 개발 스텝: `/research`(Claude `--add-dir` · codex 추가 디렉터리 실제 동작 · 피어) → 선택지 → **사용자 결정** → PRD/TRD 반영 → 구현 위치(어느 단계에 끼울지) 결정. 먼저 사용자에게 해석이 맞는지 한 줄 확인.
- 그 뒤 TRD 순서 = **16 단계(실행 답 넓히기)** — 정지 단계(진행 전 사용자 확인).

## repo 상태 (브랜치 v0.5.0/feat/preset-library · wt1)
- 원격 = 로컬 = `f03ba511`. 순서: `7ba2ad66` 2 단계 platform fs → `94702a5e` ADR-0339 번호 선점 → `f89108fa` ADR-0339 확정 · PRD · TRD 문서 → `f03ba511` 4 단계(P1~P4b-2 한 커밋).
- 미커밋: 핸드오프 파일뿐. `src-tauri/bindings/*.ts` · protocol bindings 2 개의 `M` 은 줄끝(LF/CRLF)만 — 내용 diff 0(무시).

## 오늘 결정(기록 = ADR-0339 · 결정 기록 ROUND 16 G58–G63 · M99–M110)
- 덮어쓰며 이동 = 삭제 + 이름 바꾸기 · 참조는 자리를 따름(선택 없음) · 모든 경로 바꿔 쓰기 뒤 한 목록 안 같은 경로(same_path)는 처음 자리만(에이전트 · 프리셋 · 같은 창 초안).
- 삭제 · 참조된 스킬 `SKILL.md` 이름 바꾸기 · 폴더 밖으로 옮기기 = 참조 처리 선택 — 사람 = 모달 [취소] [참조도 빼고 …] [기본] · LLM = `--yes` + `--detach-refs`(자리표시) · 기본 남기기 · 뺀 참조는 되살려도 안 붙음 · 끊기면 남기기와 같음.
- 메타 파일(사이드카)·파일 안 id 안 씀(사용자 「메타파일은 하지말고」). OS 파일 신원 · 해시는 「바깥 이동 뒤 되찾기」 후보로 tracking T-74 에만.
- 에이전트는 프리셋만 붙는다 — SKILL.md 계열 참조 빼기는 프리셋 파일의 스킬 줄만.

## 검증 상태
- 4 단계: 조각마다 Claude · codex 리뷰(P4a · P4b 는 DEEP + 렌즈 추가) PASS 까지 · QA full(워크스페이스 61 결과 줄 · 5159 통과 · 0 실패 · 36 무시 · 프론트 vitest 2760 · tsc · 격리 게이트 전부 · 실 앱 smoke = claude 에이전트 띄워 답 받고 kill → 목록에서 사라짐) · CI 초록.
- 2 단계: 리뷰 PASS · CI 초록.
- **미검**:
  - codex 실 CLI 계약 시험(`cargo test -p engram-dashboard-agent --test backend_contract -- --ignored --test-threads=4`) 6 건 FAIL(q4 · q5 · q6 · q8 · q11 · q12) — codex-cli 0.163.0 app-server 가 준비 안 됨 + q6 `--no-alt-screen` 값 어긋남. 환경 문제로 추정이나 **기준(7ba2ad66)과 대조 안 함**. 6 단계(codex 번역) 전에 확인.
  - QA 4f 게이트는 시험 3 줄(`cfg!(windows)`) 지운 뒤 로컬 재실행 못 함(이 셸에 rg 없음) — CI 가 같은 게이트를 초록으로 돌림.
  - `real_wmi` 레인 안 돌림(손대지 않은 경로).
  - 1 단계 GUI PASS 외 3 · 4 단계 화면 영향은 없음(4 단계 실 앱 smoke 만).

## 다음 단계가 지킬 것(TRD §5-2 4 아래 「P4b · 8 단계가 지킬 것」 (a)~(e) · (c2) — 정본은 TRD)
- 실행 재료는 점검한 계획에서만(다시 읽은 프로필 X) · 지침 해시는 kit 에서 · prepare 는 원본 사라짐/바뀜 = Stale.
- 회수 단일 주인 = reaper(자동 복원 끄기 · 목록 통지 뒤) · release 는 빠르고 패닉 없고 (id, epoch) 만 · 부팅 쓸기가 받침.
- 8 단계: 라이브러리 쓰기 락 쥔 채 activate 금지(재진입 교착) · asset-library 가 manager 를 강하게 쥐면 Arc 순환 → Weak.
- 16 단계: bus `wake_existing` 의 LaunchBlocked · LaunchRaced → CONFLICT(지금 INTERNAL 로 접혀 요청 번호 붙듦).
- 5/6 단계: materialize 쓰기 경로는 `kit.volatile_dir` 에 join 한 것만(대소문자 · 접두 변형 금지 — 가둠이 원래 철자로 거른다).

## 문서 단계(15a/18 — 사용자 확인 뒤)로 넘긴 것
- CLAUDE.md: 「핵심 불변식 — 소유권 분할」 에 세션 필드 `launch_lease`(세션은 들기만 · reaper 하나가 꺼내 release) · 「의존성」 junction 2.1.0(default-features off) · windows feature Win32_Storage_FileSystem · Win32_Globalization / 모듈 맵 platform fs(link · path_key · identity) / 「화신 표식」 의 `agents.json` → 프로필 파일 이름 / base `file` 항목의 「agents · presets 저장소」.
- 코드 `// ADR-0061` 앵커 → ADR-0070 · 옛 프리셋을 말하는 docs/reference · ADR-0163/0291/0264 의 agents.json 언급.
- T-76 의 낡은 주석 둘(`spawn_agent` doc · `backend/codex/transport.rs` ~2716 「도달 불가」).

## 사용자 지시 — 계속 유지
- codex 리뷰어에 같은 자료 · 질문은 대화로 하나씩 · 예시 위주 · push 허락 · 서버 먼저 · 단순 · 관행.
- 정지 조건: 멈춤(앞) 단계(16 · 18 · 8 · 10a) 리뷰 전 · CLAUDE.md 수정 · master 머지 · 리뷰 BLOCK/불일치 · 재수정 한도 초과.
- LLM 명령 꼴은 「LLM 이 헷갈리지 않게」 기준으로 메인이 정하고 보고(사용자 위임 2026-11-01).

## 실패한 접근 · 비용
- 큰 조각을 코더 하나에 = P4a 53만 토큰 · 도구 165 회(한도 근처) · P4a 수정 코더는 사용량 한도로 도중 사망(부분 커밋으로 이어받음). P4b 를 둘로 나눈 뒤 조각당 27~35만.
- 줄끝이 섞인 파일(blob LF · 작업 파일 CRLF/LF 혼재 · autocrlf)에서 `sed -i`(한 줄이어도) · perl 이 CR 을 지우거나 조용히 실패 → 코더 지시서에 「모든 편집은 Edit 도구」 를 넣는다. python 은 PATH 의 스텁이라 안 돈다.
- 돌연변이 시험 뒤 파일을 되돌리면 수정 시각이 꼬여 낡은 시험 바이너리가 재사용됨 → 되돌린 뒤 `touch`.
- 히스토리 정리는 plumbing(임시 GIT_INDEX_FILE · commit-tree · update-ref + `git reset -- <path>`)으로 작업 트리를 건드리지 않고 했다(리뷰어가 읽는 중에도 안전).
