# 핸드오프: 3단계 transport TRD 5판 · 결정 대기 — 다음 = 판단 갈리는 결정부터

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.4.1/refactor/transport-attach`(master `ba39a080` 에서 팜) · HEAD `d5d941cd` · 작업트리 깨끗 · **전부 로컬 커밋 · push 안 함**. 코드 변경 0 — 설계 문서만.
★다음 첫 액션 = 사용자에게 「판단 갈리는 결정」을 하나씩 묻는다(사용자 지시 2026-10-13). 순서: D19 → D11 · D24 → D4 · D5 → D26 · D22 → D25 → D20 → D6 → D23. 나머지(D1 · D2 · D3 · D7 · D8 · D9 · D10 · D12 · D13 · D14 · D15 · D16 · D17 · D18 · D21)는 추천대로 일괄 승인받는 묶음 — 걸리는 것만 빼서 따로.★
사용자는 개념 브리핑을 먼저 본다(사용자가 직접). 질문 규율: 목록은 전부, 펼치는 건 하나(AskUserQuestion 은 두 번 거절됨 — 평문으로 묻는다).

## 읽을 것
- 사용자용: `docs/process/S21-crate-boundaries/trd-3-transport-brief.html`(개념 브리핑 · 그림 11).
- 명세 정본: `docs/process/S21-crate-boundaries/trd-3-transport.md`(5판 · §8-3 = 결정 D1~D26 선택지 + 추천 · 맨 위 판별 변경 표).
- 그림 상세: `trd-3-transport.html`(5판 기준 · 길다).
- 실측: `docs/research/transport-stage3-measurements-2026-10-13.md`. 조사: `docs/research/transport-security-build-vs-adopt-2026-10-12.md` · `screen-sharing-oss-2026-10-12.md`.

## 결정에 걸린 사실 (요점)
- 3단계 = net 걷고 셸 · 데몬 양쪽을 transport 로. 내보내는 동작은 셸 하나 + 로컬 데몬 하나. 다중 데몬 · 데몬끼리는 「붙을 수 있는 구조」까지만(사용자 방향 2026-10-12). 사용자가 「다중 데몬 지금 구현 아님」 확인 — D4 · D5 는 줄일 1순위 후보(D3 데몬 ID 는 Hello 칸 하나라 지금이 쌈).
- 새 단계 3-0 = 붙이기 전 transport 보강(지금 transport 받는 줄이 차면 버린다 — 셸 · 데몬 둘 다 못 견딤). 3-0 ∥ 3-1 병행 가능(게이트 사본 커밋만 3-1 U2 → 3-0 U7 → 3-1 U5 직렬).
- D11 데몬 받는 구조 = 리뷰마다 바뀌다 ②′ 로 수렴(상대별 무손실 줄 + 바이트 예산 하나 · 데몬은 net 연결별 read_task 를 그대로 옮겨 그 줄을 읽는다). ②′ 는 4회차 리뷰에서 교착 없음 확인.
- ★실측: `.engram-dev\daemon\run\daemon.json` 이 같은 PC 다른 로그인 사용자에게 읽기 · 수정 가능(I:\ 루트 상속 · icacls)★ → D26 추천(「3단계 범위 밖」)을 다시 볼 근거. 배포판은 압축 푼 자리 상속(미실측).
- Origin: 운영 클라이언트(셸 Rust · 끄기 · Node 스크립트) 안 보냄 · CLI 는 WS 안 씀 → D21 ① 안전. 요청 시한: codex 첫 기동 ~9.7 s 기록 → D15 ①(30 s). RawValue 전환: protocol 102 통과 · 골든 바이트 동일(`Value` 경유 경로는 바이트 비보존 — 잠재).
- 브랜치 버전 접두 떼기 = tracking T-60(나중에 · 사용자 결정).

## 검증 상태
- TRD 리뷰: `/review trd` deep 3회(매회 codex 설계 + Claude 불변식 + Claude 보안) + 4회차 좁게 2인 — 전부 FIX → 5판에 반영. **5판 자체는 재리뷰 안 함**(하드캡 초과 · 사용자 지시로 종료).
- 빌드 · 시험: 코드 변경 없음이라 안 돌림. RawValue 는 격리 작업 사본 실험(지움).
- HTML 두 개: 워커가 headless Edge 로 다크 · 라이트 · 360 px 확인(일부 그림은 마지막 수정 뒤 재렌더 안 함).

## do-not / 주의
- 결정 전 구현 진입 금지(CLAUDE.md 「개발 스텝」). 결정 나면 ADR 박제(`/adr`) → 구현.
- 사용자는 긴 문서 · 코드 위주 설명을 싫어한다 — 개념 · 구조 · 흐름 · 결정 위주로 짧게. 인계(handoff)는 제안만 하지 말고 지시하면 바로 한다.
- Git Bash: `rg` 없음 · `python` 은 Store 스텁(`py` 사용). headless Edge 는 창 폭 504 px 에서 잘린다 — 폰 폭은 CDP `Emulation.setDeviceMetricsOverride`.
- codex 리뷰는 `codex:codex-rescue` 서브에이전트 · 전경 · `--fresh` · read-only · `--effort xhigh`.
- 시험 · 빌드가 `bindings/*.ts` · `src-tauri/Cargo.toml` 줄끝만 바꾸면 `git checkout -- <경로>`.

## 정지 조건
- 결정(D1~D26)은 메인이 확정하지 않는다 — 사용자에게.
- 리뷰 BLOCK · 정면 대립 · master 머지 · 태그 · push → 사용자에게.
