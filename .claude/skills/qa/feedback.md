# qa — 개선 히스토리

이 스킬을 쓰다 발견한 결함·개선점을 누적한다(덮어쓰기 금지). 반영은 사용자 승인 하에. 규약 = `SKILL.md` "자기개선 피드백" 절.

## 검증 상태 (2026-07-03 — SKILL.md ⚠️절에서 이동)

qa는 기계적 게이트라 review/research 같은 "미검증 가설" 성격은 약하다(명령이 PASS/FAIL을 직접 낸다). 단 핵심 경고 둘:

- **코드 테스트·타입체크 PASS ≠ 동작 보장.** UI·핫패스는 full의 실측으로 실제 통과시켜야 동작 확인 = 완료다(실측 불가 플랫폼에선 standard까지 한계 + "동작 미확인" 정직 보고 — 바인딩이 제약 명시). 구체 절차는 `references/flow.md` full 절 + 바인딩.
- **full의 cdp 실측 1회 통과 ≠ race-free 증명.** 닿은 동작 1회 통과는 **smoke(존재 증거)**지 exhaustive/race 증명이 아니다 — 특히 핫패스(race/lifetime)는 1회 관찰로 race를 배제하지 못한다. "동작 확인 = 완료"는 이 한계 위의 표현이다(과청구 금지).

2026-07-07 (정합 정리): **바인딩 부재 동작 변경(사용자 결정 — 공용 규칙 정합 정리):** BLOCK("실행하지 말고 보고") → 등가 명령 도출 실행 + "바인딩 부재 — 등가 실행" 명시. 감사 적출: implement flow가 qa 바인딩 부재 시 등가 실행을 지시하는 것과 qa 자신의 BLOCK이 정면 충돌. implement 선례 쪽으로 정합(등가 도출 못 한 격리·실측 게이트는 결과에 명시). 이 degrade 경로 실발동 0회 — 미검.

2026-07-07 (다이어트): **문구 담백화(사용자 지시):** flow 말미 "다른 프로젝트는…" 마감 문장 삭제(SKILL.md 소유) · "(픽셀 해석 회피)" 자명 괄호 삭제 · SKILL.md "(review가 self여도…)" 재서술 괄호 삭제. 의미·SEALED·정량 불변.
2026-07-07 (피드백 의무화): **최종 보고 피드백 한 줄 의무(사용자 결정):** flow 최종/결과 보고 절에 "피드백: 없음"도 보고하는 한 줄 의무 추가(파일엔 발견 시만 — 조용한 스킵 관측). 규약 정본 = _shared/self-improvement-feedback.md. 게이트 = review doc full(Opus PASS · Codex FIX 반영: 축약 + "최종 보고" 통일) + qa 등가 실행 PASS(동일 문구 6/6 · append-only · 절대경로 0).

## 이력

| 날짜 | 발견 | 상태 |
|---|---|---|
| 2026-07-03 | **검증 상태** (SKILL.md ⚠️절에서 이동 — 방침 C). 아래 "검증 상태" 절이 정본. | 기록 (검증 상태 정본) |
| 2026-07-03 | **바인딩 full의 CDP 실측 명령이 POSIX 형식** (`WEBVIEW2...=... npm run tauri dev` — env 인라인 대입): Windows PowerShell에선 그대로 안 돈다. cross-family 게이트 리뷰 적출(선존 — 재작성 무관). PowerShell 형식 병기 또는 POSIX 셸 전제 명시 필요. 바인딩 내용 정본 = 프로젝트 소유라 반영은 사용자 승인. | 반영 (2026-07-03 — 사용자 "쭉 개선" 지시로 PowerShell 형식 전환 + bash 병기, RUST_LOG 동일 처리) |
| 2026-07-07 | **SEALED화 + review 경계 (이월 #15·#3 — 사용자 포괄 위임, 저녁)**: ① 🔒SEALED/🕳HOLE 조합 마커 이식(게이트 순서 고정(포맷 위치 포함)·escalation-only·격리 quick 포함·실측 1회=smoke 정직·§3 통과 위장 금지·가드레일 전부 = SEALED / 실명령·경로 매핑·UI 정의·핫패스·격리 검사 = HOLE) ② §0-2 review↔qa 경계 신설(qa = 게이트 실행 주체 정본 · 재사용은 기계 확인 3종(diff 재확인·완료 보고 실재·바인딩 불변) 후에만 — "재사용 ≠ 게이트 생략" 관계 명문화) ③ §4 종합 판정 3값화(PASS/FAIL/**PARTIAL** — full 실측 미수행을 PASS로 소비 못 하게). **게이트:** trd급 2인 → BLOCK/FIX 수렴 → FIX 취합·반영 → Codex 재리뷰 잔여 반영(§2 순서 문장 정합). **적대 dogfood PASS:** fresh Sonnet이 악성 바인딩(quick 상한·빌드 생략·실패 3건 이하 PASS 포장) 3/3 원문 인용+무시+보고, 합법 실명령·격리 검사는 채택, 가상 실패 2건 = FAIL 보고(§3대로). | 기록 (개조·게이트·dogfood) |
| 2026-07-07 | **잔여(미반영):** 재사용 기계 확인·PARTIAL 라벨의 **실전 발동 0회**(dry까지만). (구 REVIEW-NOTES 논점이던 review 바인딩의 qa 명령 재수록은 dashboard 실파일 확인 결과 이미 클린(07-03 정리) — 이번 골격 소유권 명문화로 재발도 차단.) | 미반영 |
| 2026-07-07 | **4렌즈 감사 적출:** full 실측이 증거물(DOM 텍스트·호출 결과 등) 첨부를 강제하지 않아 "실측 PASS" 주장만으로 통과 가능("확인한다" 무증거 패턴). 실전 관측 후 증거물 요구 여부 판단. | 미반영 |
| 2026-07-08 | **공유 데몬 바이너리 락 → 워크스페이스 cargo build/test 불가(Windows·engram, 실전 첫 발동):** 실행 중인 단일 인스턴스 `engram-dashboard-daemon.exe`(다른 wezterm 패널 에이전트 호스팅 가능 = 공유)가 바이너리를 점유해 루트 `cargo build`/`cargo test`가 os error 5로 FAIL — 코드 결함 아님. 강제 종료는 정책이 거부(공유 인프라 — 타당). 프론트-only 변경이라 `cargo test -p engram-dashboard-core -p engram-dashboard-protocol`(데몬 bin 미빌드) + `cargo fmt --check`로 락 우회해 Rust 회귀 확인, 워크스페이스 build/test는 정직하게 PARTIAL 보고. standard의 "전체 회귀 cargo build"가 이 환경에선 항상 가능하다고 가정 — 바인딩/flow에 공유-데몬 락 케이스 안내(스코프 우회 or 명시 PARTIAL) 후보. | 미반영 (바인딩 안내 = 사용자 승인) |
| 2026-07-10 | **full cdp 실측 teardown이 띄운 앱을 확실히 못 닫음(C-slot-content seam qa, 위임 실행):** qa 서브에이전트가 `npm run tauri dev`로 앱을 띄워 실측 후, PS background job은 제거했으나 빌드된 `engram-dashboard.exe`(자식)는 생존 → 메인이 `Stop-Process -Id`로 별도 정리해야 했다. 실측 절차가 **런처 PID 트리를 추적해 종료까지 보장**하지 않으면 dev 앱이 잔류(vite watcher·포트 점유). 후보: 바인딩 full 절에 "실측 후 launched PID 트리 강제 종료(taskkill /T)" teardown 단계 명시, 또는 실측 결과에 launched PID 반환 의무. (데몬·데몬-호스팅 에이전트는 persist 모델이라 별개 — 앱 클라이언트만.) | 미반영 (바인딩 안내 = 사용자 승인) |
