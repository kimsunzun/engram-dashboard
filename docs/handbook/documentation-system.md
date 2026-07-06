# Engram 문서 시스템 (documentation-system)

- 상태: **living** (git이 이력 — 파일 내 변경이력 섹션 안 둠) · 갱신: 만지는 사람이 boy-scout
- 이 문서 = "engram 문서들이 **개발 플로우 어디에 어떻게 박히나**"의 지도. 문서·설계 작업 전 `../README.md`(허브)와 이 문서를 본다.
- 근거: `../research/documentation-architecture-research-2026-06-24.md` (업계 관행 조사).

## 1. 개발 플로우가 척추다

전체 개발 흐름의 **정의는 `../../CLAUDE.md`** — "★ 개발 스텝 (매크로 흐름) ★"(리서치→PRD→TRD→DDD→구현+TDD) + "★ 구현 실행 규약 ★"(코더→리뷰어→QA)에 있다. **여기 다시 적지 않는다(SSoT).** 이 문서는 그 흐름 *위에 문서를 얹는 레이어*다.

## 2. 단계별 — 산출 문서 · 게이트 · 기록

| 단계 | 산출 문서 | 검증 게이트 | 기록 |
|---|---|---|---|
| **리서치** | `research/` 보고서(+`status:`) | research 내부 교차검증 | step-log |
| **결정(PRD)** | **ADR**(왜 + 거부한 대안) | `/review prd` → 사용자 확인 | step-log + `/adr` |
| **설계(TRD)** | TRD/design(`process/SN/spec/`) + 설계 ADR | `/review trd` → 사용자 확인 | step-log + `/adr` |
| **구현** | 코드 + 테스트 + `// ADR-` 앵커 | `/review code` → `/qa` | step-log |

- **ADR은 한 단계 전용이 아니다** — 굵은 결정이 날 때마다(PRD·TRD 양쪽) 쓴다.
- **step-log**(`process/step-log.md`)는 전 단계에서 *언제/무엇*을 남긴다(타임라인 단일 출처).
- **reference**(`reference/`)는 흐름 옆에서 코드 동기 캐논으로 갱신된다(컨벤션 등).
- **핸드오프**(`.ccb/`)는 일회성 세션 인계 도구다 — **문서 아키텍처 밖, 기록 안 남김.**

## 3. 불변식 (이것만 지키면 나머진 유연)

- **SSoT** — 복사 말고 링크. 중복 = stale 비용.
- **고아 금지** — 새 문서는 발견 체인(`README.md`·`tracking.md`·코드 앵커) 중 하나에 반드시 연결.
- **soft + hard** — 문서(의도·이유, soft) + 도구(lint/test/CI, hard) 짝. 불변식은 hard로도 박는다.
- **수명** — 불변·누적(ADR·step-log) vs living(README·reference, git이 이력). design/research는 승인·freeze 후 snapshot.

## 4. 자동화 맵 (무엇이 자동, 무엇이 수동)

| 노드 | 도구 | 상태 |
|---|---|---|
| 리서치 | `/research` | ✅ |
| ADR(채번·양방향·인덱스·lint) | `/adr` + `.claude/skills/adr/scripts/adr.mjs`(스킬 내장) | ✅ |
| 단계 게이트 | `/review`(단계 인자) · `/qa` | ✅ |
| step-log 기록 | (수동) | 수동 |
| 문서 lint/CI(고아·링크·freshness) | — | 나중 (hard 보강) |
| CLAUDE.md 슬리밍(always-load 비용) | — | 나중 (큰 변경, 방향만) |

스킬 4종(research/review/qa/adr)이 이미 흐름의 **산출·게이트 자리에 박혀** 있다. 빈 칸은 step-log(수동)뿐.

## 5. 어디에 넣나 / 무슨 문서가 있나

타입·배치·"새 내용 어디에"는 **`../README.md`가 정본** — 여기 복붙하지 않는다(SSoT). 이 문서는 *플로우 매핑*만 담당하고, 배치 규약은 허브를 본다.
