# 핸드오프: S17 답장 왕복 닫힘(MCP) + 발신 pre-auth(ADR-0094) + 봉투 포맷 스파이크 1차

## 한 줄 상태 · 다음 첫 액션
- **상태:** 답장 왕복 A→B→A가 **MCP 경로로 실측 성립(닫힘)**. 발신 입구 런타임 pre-authorization(ADR-0094) 구현·리뷰·QA·커밋·push 완료. 봉투 포맷 스파이크 1차(colon/bracket/xml, sonnet 1샘플) 완료 — 전부 수용, 최소 포맷이 더 쌈. **전부 origin/master push됨(`dd01d0c`). 워킹트리 클린**(`.vs/`만 untracked IDE 노이즈).
- **다음 첫 액션:** 봉투 포맷 스파이크 **확장** — 포맷 × 다모델(opus/haiku)·다샘플로 **오차율(수용 실패율)** 차이 측정(1샘플에선 전부 수용이라 차이 안 보임). 그 데이터로 **최종 봉투 포맷을 사용자에게 제시(결정→ADR)**. 그 뒤 사다리 = 포화 등 비정상.

## 이번 세션 커밋 (전부 origin/master push)
- `24e4765` 왕복 실험 하네스(C0~C3 매트릭스) — ADR-0093
- `281c95f` 하네스 진단(B seed-후 턴 캡처)
- `422b526` 발신 입구 런타임 pre-auth(grant seam) — ADR-0094
- `0bd56c1` step-log(pre-auth+C4)
- `089586a` 봉투 포맷 스파이크 seam(`ENGRAM_WRAP_FORMAT` override + 포맷-무관 프라이밍)
- `dd01d0c` step-log(발신채널 진단+포맷 스파이크 1차)

## 핵심 실측 결과 (확실 — 단 표본 작음)
- 왕복 안 닫히던 원인 = **claude 런타임 툴-권한 게이트**(발신 툴 승인자 없어 차단). 프롬프트로 self-grant 불가(공식 문서 + C0~C3 실측 교차확증). 해결 = 스폰 시 `--allowedTools`로 **발신 입구만 최소권한 pre-auth**. C1/C2에서 B_SENT=true(mcp) + A 수용까지 → **왕복 닫힘**.
- 발신엔 **프라이밍(발신 의사) + grant(발신 허용) 둘 다** 필요. B는 **MCP 선호**(C1에서 둘 다 줘도 mcp). CLI(engram-send)는 B가 셸 CLI를 자신 있게 안 써 미작동 = **저가치**(별도 추적).
- 봉투 포맷(B→A 배달 바이트): colon `{sender}: {body}` **266** / bracket+uuid **315** / xml+uuid **402** — 전부 수용. **uuid id가 최대 오버헤드고 수용엔 불요**(발신자 prefix만으로 충분). 포맷-무관 프라이밍이면 수신자가 형식 예시 없이 파싱. **사용자 가설(양식↓) 지지.**

## 검증 상태 (쌍)
- **한 것(전부 PASS):** 각 슬라이스 `/implement standard`(코더복잡=worker-senior) + `/review code full`(doc-aware + cross-family codex — blind가 실 결함 적출: 하네스 pre-seed 오탐·A liveness·`--allowedTools` variadic 흡수 → 전부 하드닝 폐쇄) + `/qa standard`(전 멤버 0 failed·fmt·코어격리0·프론트 재사용). C0~C3 + C4(pre-auth 후) + 포맷 3후보 실 claude(sonnet) 실행. **재실행:** `cargo build -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke` → `target/debug/roundtrip-smoke.exe --priming <C0|C1|C2|C3|경로> [--model M]`. 포맷은 앞에 `ENGRAM_WRAP_FORMAT='{sender}: {body}'`.
- **안 한 것(정직):** ① 다모델(sonnet만)·다샘플(케이스당 1회) → **오차율 미검증**(n=1은 전부 수용). ② 최종 봉투 포맷 **미결정**(데이터만 수집). ③ CLI Bash 풀경로 패턴 실제 매칭 미검증(B가 시도조차 안 해서). ④ 포화·인젝션 비정상 미착수. ⑤ codex/gemini 발신 번역 TODO 스텁(미구현).

## do-not (누적)
- 루트 bare `cargo test` 금지(멤버별 `-p`). 릴리즈 `--all-targets` 금지(test-harness 유니피케이션).
- **발신 권한 넓히지 말 것** — 발신 입구 2개만, `bypassPermissions`/`--dangerously-skip-permissions` 금지(ADR-0094 최소권한 불변).
- **`--allowedTools` 그룹은 args 맨 끝 유지** — variadic이 뒤 positional(extra_args)을 흡수(리뷰 적출). 앞으로 옮기지 말 것.
- **봉투 wrap는 단일 지점**(`wrap_message`, ADR-0086) 유지 — `ENGRAM_WRAP_FORMAT`은 스파이크 전용, 기본(unset) 경로 byte-identical.
- **봉투 최종 포맷·발신 권한 정책은 사용자 결정** — 임의 확정 금지.
- 회수 프로브를 인젝션-모양으로 만들지 말 것(자연 메시지). happy-path baseline 통과 전 비정상 본격 착수 금지.

## 정지 조건
- 최종 봉투 포맷 확정 = **사용자 결정(→ADR)** — 다모델/다샘플 데이터 모은 뒤 선택지로 제시(임의 채택 금지).
- 리뷰어 정면 대립(FIX vs BLOCK)은 자율모드라도 사용자 에스컬레이션(이번 세션은 codex가 상위집합 실버그라 증거기반 하드닝으로 수렴).

## 참조
- **ADR-0093**(왕복 실험·C0~C3 매트릭스) · **ADR-0094**(발신 런타임 pre-auth·grant seam) — `docs/decisions/`.
- 코드: `crates/engram-dashboard-daemon/src/bin/roundtrip_smoke.rs`(하네스) · `.../control/ingress.rs`(`wrap_message` + `ENGRAM_WRAP_FORMAT` seam ~:550) · `crates/engram-dashboard-core/src/agent/backend/claude.rs`(grants→`--allowedTools` ~:216) · `.../control/mod.rs`(`build_grants`).
- 실험 프라이밍: `prompts/experiments/agent-priming-{send-both,send-mcp,send-cli,format-agnostic}.md`.
- step-log 최하단 3개 S17 항목.
