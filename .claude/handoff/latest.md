# 핸드오프: 패닉 정책 구현 U1~U5 + ADR-0293 완료 · 실 claude 통과 · 남은 = 사용자 판단 3건

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.3.3/refactor/crate-boundaries` · HEAD = ADR-0293 철거 `17a91520` 뒤 문서 커밋(push) · 워킹트리 깨끗. 패닉 정책 구현(TRD `docs/process/S21-panic-policy/trd.md` · ADR-0292) U1~U5 전부 착지.
★다음 첫 액션 = 아래 「사용자 판단 거리」를 하나씩 묻는다(3번부터 — 1 · 2번은 해소).★ 그다음 순서(옛 핸드오프 · step-log): 우편 주소 id 통일(T-29) → 3단계 transport.
사용자 지시: 자율 모드 · 단위마다 로컬 커밋 · push 는 몰아서 · master 머지 · 태그는 사용자 확인.

## 이번 세션 커밋
U3 `f03dd10c`(훅 설치) · U4-C `c9d15023`(주석 40 파일) · U4-T `06c23844`(CLI 정리 실프로세스 시험 · CLAUDE.md 수치 60 · 4371) · U4 문서 `37e58f9a` · U5 `40b8a08e`(`panic = "abort"` 삭제 + step-log 항목). 상세 = step-log 끝 항목.
- 결정 A 성립: 가둔 펌프 패닉 뒤 CLI 트리가 명시 종료 없이 수 ms~20 ms 에 사라짐(세 펌프 · Windows).
- U5 크기(abort → unwind · 같은 빌드 명령): 데몬 +51% · 셸 +76% · CLI +19% — 사용자에게 보고함, 문제 삼을지 미답.
- 배포판 셸 + 데몬 실 창 실측 PASS · graceful 종료 exit 0/0.

## 사용자 판단 거리 (하나씩 묻기)
1. ~~실 claude 시험 6건~~ — ★해소(사용자 승인 2026-10-12)★: U5 커밋 뒤 qa 바인딩 실 claude 레인으로 6/6 통과(claude 2.1.284). 옛 「실 claude 시험 금지」 주의는 걷는다 — CLAUDE.md · qa 바인딩대로 로컬에서 돌린다.
2. ~~U2 허용 목록 범위~~ — ★해소(사용자 결정 2026-10-12)★: 허용 목록 명령 둘의 가둠을 걷었다(ADR-0293 `fcebe727` · 코드 `17a91520`). 가둠 = 에이전트 펌프 셋 + 입력 라이터뿐.
3. 데몬 sweep(`daemon/src/lib.rs` 6.6)의 catch 갈래 — 어느 빌드에서도 닿지 않음. 걷을지.
4. 로그 초기화 통째 실패 시 기동 거부 정책(지금은 TRD §7 잔여로 문서화만).
5. 근거가 「release=abort」에 기대던 ADR(0199:46 · 0226:43,75 · 0233:46 · 0262:105 K7 · 0275:16,52,87 S2 · 0281:46 · 0142:57)의 개정 링크(ADR 근거 수정 = 결정 날조 금지라 사용자 확인). ADR-0275 S2 는 근거가 실효로도 거짓(가둔 펌프에서 운영 오염 가능 — base `sync` 계약 3 은 고침).
- 손대지 않은 이력 TRD 의 같은 서술: usage-limit-slot:67 · storage:301,918 · codex-backend resume-after-first-turn:101-103,298,529 · crate-boundaries 1-1:64,201,325,556 · chat-ux t40:262 · S20:629.

## 검증 상태
- 돌린 것: 단위마다 `/review code`(deep · full · light) · `/qa`(full · standard). 마지막 `/qa full`(U5) = 60 결과 줄 · 4371 · 0 · 30 · 격리 · tsc · npm 2021 · 배포판 GUI.
- 실 claude 6건: 따로 돌려 6/6 통과(5 건 9.7 s · `c1_park_then_spawn_auto_delivers` 20 s).
- 안 돌린 것: `real_wmi` · real-claude/usage/backend_contract 레인 · 비-Windows. `stop_smoke` 의 `send_stop_makes_real_daemon_self_exit` = T-52 선재 실패(Timeout).
- 미측정(TRD · 시험 헤더에 적음): 활성화 판정 창 · codex 핸드셰이크 중 패닉.

## do-not / 주의
- Git Bash 에 `rg` 없음(QA 워커는 scratchpad `qa/bin/rg.exe` 를 썼다 — 다음 세션엔 없을 수 있음). python 은 스토어 바로가기라 실행 안 됨.
- 시험 · tauri build 가 `crates/engram-dashboard-agent/bindings/*.ts` · `src-tauri/Cargo.toml` 을 줄끝만 바꿔 ` M` — `git diff --numstat` 0 이면 `git checkout -- <경로>`.
- 재리뷰는 리뷰어 resume. codex 병렬 두 개 뒤 `--resume` 금지. codex blind 지시서엔 단계 순서 같은 범위 사실을 처음부터.
- 실프로세스 시험 지시서엔 정리 안전 요건(신원 대조 handle 로만 끄기 · guard 를 spawn 전 · 자식이 자기 신원 알림 · 실패 갈래 도달 증명)을 처음부터(implement feedback 적립).

## 정지 조건
- 리뷰 BLOCK · 정면 대립 · 하드캡 초과 → 사용자에게. master 머지 · 태그 · ADR 근거 수정 → 사용자 확인.
