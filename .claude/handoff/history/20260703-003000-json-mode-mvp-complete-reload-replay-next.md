# 핸드오프: JSON 모드 MVP 완성(M0~M2+실E2E) + 리로드 두절 픽스 — 남은 것 = 러스트측 replay 재발화 트레이스

## 한 줄 상태 · 다음 첫 액션
ADR-0044(JSON 모드 = StdioTransport + 바보 통로) 기준 M0(스파이크)→M1(백엔드 seam)→M2(실스트림 E2E)까지 완성, 실 claude 왕복 실측 PASS. 이후 발견된 "리로드 시 창 출력 전면 두절"(선재 S14 버그)을 self-heal + pre-subscribe 버퍼링으로 픽스(리뷰 7건 반영). **다음 첫 액션 = 마지막 남은 결함 트레이스: 웹뷰 리로드로 Channel 재등록 시 러스트측 replay가 새 Channel로 재발화 안 함** — `src-tauri/src/commands/agent.rs`(subscribe_output → replay_slots(fresh=true)) → `output_router.rs`가 새 Channel에 실제 배달하는지. 프론트는 무죄 실측 완료(구독 존재·lastSeq -1 = 프레임 0개 도착). 워크어라운드 = 에이전트 재배정(전량 복원됨).

## repo 상태
- **HEAD = `bb8d025` (master), 오늘 커밋 4개 전부 미푸쉬:** `65005f7`(M0 RichSlot 스파이크+ADR-0044) · `209eb84`(M1 StdioTransport seam) · `05e7d54`(M2 실스트림 E2E) · `bb8d025`(리로드 두절 픽스).
- 미커밋 잔여 = `_wip/`(다른 에이전트/유저 영역 — 건드리지 말 것). ★`git add -A`/`.` 금지 — 타깃만.★
- **실행 중 프로세스(이 세션이 띄움):** tauri dev(백그라운드 태스크) · engram 데몬(pid 21144, 새 바이너리) · json claude(pid 23180) · E2E 테스트 프로필 2개(트리의 json-*, 유저 확인 후 정리). 다음 세션 시작 시 살아있을 수도/아닐 수도 — 데몬 정리 필요하면 **engram-dashboard-daemon만** kill.

## 검증 상태 (쌍)
**돌린 것:**
- 실 claude JSON 왕복 E2E(cdp): spawnJson→배정→RichSlot 렌더→입력→응답 "2"→idle. 스샷 `_wip/shots/richslot-m2-live-e2e.png`.
- self-heal 라이브 검증: 리로드 후 `transport:connected + channel:true` (픽스 전엔 영구 down).
- 게이트: `npx tsc --noEmit` · `npm test`(**181**) · `cargo test -p engram-dashboard-core`(162) · `-p engram-dashboard-protocol`(33) · `-p engram-dashboard-daemon` · `cargo build` · fmt · tauri import 0.
- 재배정 replay 전량 복원(죽은 파이프 동안 친 메시지까지 claude 도달·응답 버퍼링 확인 — 유실 0).

**검증 안 된 것 (오신뢰 금지):**
- **리로드→replay 재발화 e2e = 깨져 있음**(known-issue, 위 다음 액션). S13의 리로드-replay 실측은 transport 레벨 직접 측정 + 당시 메인창 슬롯 렌더 stub → 이 경로는 애초에 e2e 검증된 적 없음.
- IME 한글 Enter 가드(keyCode 229) — 합성 이벤트로는 검증 불가, **사람 타이핑 필요**(주 사용자 한국어).
- pre-subscribe 버퍼 >2MB 시나리오 라이브(유닛만) · registerOutputChannel single-flight 라이브 레이스.
- `cargo test -p engram-dashboard --lib` = 0xc0000139로 기동 불가(선재, 아래 do-not).

## 실패한 접근 (do-not)
- **`rustc-link-arg-tests` build.rs 우회 = 실측 탈락**(src-tauri에 tests/ 타깃 없어 cargo가 instruction 거부, 빌드 전체 깨짐). 0xc0000139 정공법 = lib 내 WS 클라 테스트(T1/T2/T4)를 비-tauri 크레이트/tests/로 이전(백로그). build.rs에 KNOWN-ISSUE 주석 있음.
- **json 모드 `--verbose` 빼지 마라** — help엔 없지만 런타임이 강제("requires --verbose"), 빼면 스폰 즉사(에이전트 소리없이 소멸 — stderr가 debug 로그라 무증상).
- **epoch를 버퍼 관측치로 조기 확정 금지** — SubscribeAck 단독 권위(ADR-0007, 리뷰 FIX 4).
- **백엔드 풀 파싱·보관 재론 금지** — 사용자 결정: 무정제 유지, ADR 개정도 안 함, "나중에 고려"(리서치 근거: CLI가 이미 `~/.claude/projects/` JSONL 보관, 하이브리드 tap 선례 확실).
- `claude.exe` 무차별 kill 금지(데몬만) · 첫 로드 핸들 미등록 레이스는 리로드 1회로 복구(vite optimize-deps, 2회 재현) · wire 필드 추가 후엔 **데몬 재빌드+교체 필수**(구데몬이 모르는 필드는 조용히 default 강등).

## 블로커/미결
- 러스트 replay 재발화(위) — 유일한 미해결 결함.
- 클라이언트 재기동 시 레이아웃 소실(메모리) → 배정 풀림 — 영속화 백로그(D-7)와 한 몸.
- M3 후보: 도구 권한 승인 · partial 델타 · json resume · 스폰 UI 정식 노출 · M0 오버레이 정리 · stderr 표면화 · RichSlot 저대비 폴리시.
- 트랙②(스킬 리팩토링)는 딴 에이전트 진행 중 · 트랙④(슬롯 UI 이주) 대기.

## 참조 (읽을 것만)
- `docs/process/step-log.md` 최신 4섹션(JSON 착수/M1/M2/리로드 디버깅 — 오늘 전 과정).
- `docs/decisions/0044-*.md`(배선 결정·불변식) + ADR-0041/0043(출력 구독 소유권·mount-replay — replay 트레이스 시 필수).
- `src/api/protocolClient.ts`(pre-subscribe 버퍼) · `src/api/tauriTransport.ts`(selfHeal·single-flight) · `src-tauri/src/output_router.rs`(다음 액션의 트레이스 대상).
- cdp 검증 절차 = CLAUDE.md GUI 검증 절 (포트 9223, `window.__richslot.spawnJson()` / `__engramLayout.assignAgent`).
