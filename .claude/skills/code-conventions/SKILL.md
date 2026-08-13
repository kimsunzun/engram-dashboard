---
name: code-conventions
description: 코딩 규약(현재 = 주석)의 정본 — 코더·리뷰어 지시서에 주입해 쓰고, 기존 코드 일괄 정리는 retrofit으로 돌린다. 트리거 /code-conventions retrofit <규약> [경로...].
---

# code-conventions

규약 텍스트의 정본. 해당 파일을 코더·리뷰어 지시서에 주입해 쓴다.

## 규약 목록

- **주석**(`comments`) → `references/comments.md` — 코드를 쓰거나 주석을 만질 때(= 코딩 상시)

**주입은 그 파일 하나로 끝난다** — 프로젝트 추가분·보호 항목까지 그 문서가 안내한다. 규약 파일은 `기준` · `양식` · `보호 항목` · `프로젝트 추가분`을 갖춰야 하고, 빠진 규약은 retrofit 실행을 거부한다(판정 = 요소 실재 기준).

## retrofit (기존 코드 일괄 정리)

**`references/retrofit-flow.md` Read 강제 — 안 읽고 워커 스폰 금지.** 규약별 판정 규칙(`references/<규약>-retrofit.md`)만 읽고 스폰하면 워커 슬롯 하한·커버 대조·게이트 하한이 통째로 빠진다.

## 자기개선 피드백

결함·개선점은 그 자리서 고치지 말고 작업 종료 후 `feedback.md`에 누적한다(검증 상태도 그쪽이 정본). 전체 규약 = `../_shared/self-improvement-feedback.md`.
