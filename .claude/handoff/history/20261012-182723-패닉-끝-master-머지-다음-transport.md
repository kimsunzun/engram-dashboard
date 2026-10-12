# 핸드오프: 패닉 정책 끝 · master 머지 완료 · 다음 = 3단계 transport(새 세션 · 설계부터)

## 한 줄 상태 · 다음 첫 액션
master = `ba39a080`(패닉 정책 머지 — ADR-0292 · ADR-0293) · 브랜치 `v0.3.3/refactor/crate-boundaries` 는 남겨 둠(규약 — 지우지 않는다). 패닉 쪽 남은 일 0.
★다음 첫 액션 = 3단계 transport 를 CLAUDE.md 「개발 스텝」대로 설계부터(PRD/TRD → 리뷰 → 사용자 결정 → 구현). 새 주제라 master 에서 새 브랜치를 판다(이름 = `v<다음 릴리스>/<타입>/<슬러그>` — 다음 릴리스 버전은 매니페스트의 현재 버전이 아니다 · CLAUDE.md 「브랜치·커밋」).★
그다음 = 우편 주소 id 통일(T-29 — 사용자 결정 2026-10-12 로 transport 뒤로).
사용자 작업 방식: 자율 모드 「쭉 진행」 · 단위마다 로컬 커밋 · push 는 몰아서 · master 머지 · 태그는 사용자 확인.

## transport 출발점 (읽을 것)
- 순서 · 범위 정본 = `docs/refactoring/architecture-discussion-2026-09-26.md` §10 의 「3. transport」(:297) — 3-1 `protocol` 정리(후보 8 · 인증 메시지 이전 · 의존 0 게이트) · 3-2 셸 부착(어댑터 · 정지 명령 클라이언트 · 접속 정보 인터페이스) · 3-3 데몬 부착 + net 걷기(후보 7). 같은 문서 :229(인증을 따로 떼나 — 열린 질문) · :243~250(패킷 · 끝자락 어댑터 방향).
- 기존 설계: `docs/process/S21-transport-crate/trd.md`(+ `structure.html`) · ADR-0177~0182(2026-09-03~06) · crate `crates/engram-dashboard-transport`(워크스페이스 멤버 · 격리 게이트 있음).
- 걸린 메모: tracking 「다음」 줄의 3-3 후속(net 단일 인스턴스 열기 재시도는 platform `fs::retry_busy` · ADR-0291 결정 15) · 셸 `daemon_client::stop` 은 3-2 가 다시 쓸 때까지 임시 거처(ADR-0271 결정 5 · 셸 직접 의존 `tungstenite` 도 그때 걷는다).
- ★상태를 추측하지 말 것★ — transport crate 구현이 어디까지 갔는지는 step-log 의 S21 transport 항목(「구현 진행 중 · 결과 미확인」)과 코드로 먼저 확인한다.

## 이번 세션에 끝낸 것 (패닉 정책)
U1~U5 · ADR-0293(허용 목록 가둠 철거 — 가둠 = 에이전트 펌프 셋 + 입력 라이터뿐) · 데몬 sweep 의 닿지 않는 catch 철거 · ADR 일곱 개정 링크 · 로그 초기화 실패 시 기동 거부는 두지 않음 · 실 claude 시험 6/6. 상세 = step-log 끝 패닉 항목 · TRD `docs/process/S21-panic-policy/trd.md`.
- 배포판 크기(unwind 전환): 데몬 +51% · 셸 +76% · CLI +19% — 사용자에게 보고함, 문제 삼지 않음.
- 워크스페이스 회귀(실 claude 포함 · skip 없음) = 60 결과 줄 · 4371 · 0 · 31.

## 열린 것
- tracking T-59 — platform Job 멤버 명단 flake(CI 1 회 · 재실행 초록 · claude 잔여물 정리가 같은 함수를 쓴다 · 상한 늘려 맞추지 말 것).
- master 머지 CI = 이 핸드오프를 쓸 때 진행 중(확인 안 됨) — 다음 세션 첫머리에 `gh run list --branch master --workflow ci.yml --limit 1` 로 본다.

## do-not / 주의
- 실 claude 시험은 이제 로컬에서 돌린다(사용자 승인 · CLAUDE.md · qa 바인딩대로). 옛 「금지」는 걷었다.
- Git Bash 에 `rg` 없음 · python 은 스토어 바로가기라 실행 안 됨.
- 시험 · tauri build 가 `crates/engram-dashboard-agent/bindings/*.ts` · `src-tauri/Cargo.toml` 을 줄끝만 바꾼다 — `git diff --numstat` 0 이면 `git checkout -- <경로>`.
- codex blind 리뷰 지시서엔 단계 순서 같은 범위 사실을 처음부터 넣는다 · 병렬 codex 뒤 `--resume` 금지.
- 실프로세스 시험 지시서엔 정리 안전 요건(신원 대조 handle 로만 끄기 · guard 를 spawn 전 · 자식이 자기 신원 알림 · 실패 갈래 도달 증명)을 처음부터.

## 정지 조건
- 설계 단계에서 굵은 결정(PRD/TRD 선택지)은 메인이 임의 확정하지 않는다 — 사용자에게.
- 리뷰 BLOCK · 정면 대립 · 하드캡 3 초과 · master 머지 · 태그 · ADR 근거 수정 → 사용자에게.
