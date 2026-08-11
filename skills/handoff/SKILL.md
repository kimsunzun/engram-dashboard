---
name: handoff
description: 세션 핸드오프(인계 메모)를 기록·복원한다 — save = 지금 맥락 기록, load = 이어받기. 트리거 /handoff [save|load].
---

# Handoff

**실행 전 `references/flow.md`를 반드시 Read — 안 읽고 진행 금지.**

세션 **핸드오프**(다음 세션이 맥락을 이어받는 인계 메모)를 **기록·복원**한다. 1순위는 핸드오프가 확실히 남는 것이다. 파일명·본문은 Claude가 채운다 — 사용자는 `save`/`load`만 친다. (구 `continue` 스킬의 후계다.)

## 오퍼레이션

| op | 동작 |
|---|---|
| **load**(기본) | `latest.md`를 로드해 이어감 |
| **save** | `history/`에 영구 기록 + `latest.md` 갱신 |

## 불변 (핵심 설계)

- **기록 append-only · 읽기 고정** — save는 `history/`에 타임스탬프 파일로 **영구 기록**(덮어쓰기·삭제 금지)하고 `latest.md`를 갱신한다. load는 `latest.md` 하나만 읽는다 — "최신 찾기" 추정이 없어 결정적이다. `latest.md`는 의도적으로 덮어쓰는 편의 사본이고, 진짜 기록은 `history/`가 보존한다.
- **save = 세션 닫는 신호** — 핸드오프를 쓰기 전에 휘발 산출물(미커밋·미저장)을 영속화하거나 핸드오프에 명시한다(유실 방지). 그 외 정리는 필요한 것만(린).

## 트리거

`/handoff [save|load]` — op 생략 시 load. 파일명·내용은 Claude가 자동(사용자가 메모/이름을 넣지 않는다). 파싱·기본값·절차 = `references/flow.md`.

## 프로젝트 바인딩

경로 등 환경 의존은 바인딩으로 주입한다(정본 = `references/flow.md §0`). 별도 프로젝트 `handoff` 스킬을 만들어도 전역을 덮지 못한다(Claude Code 우선순위 User > Project).

## 자기개선 피드백

결함·개선점은 그 자리서 고치지 말고 작업 종료 후 `feedback.md`에 누적한다(검증 상태도 그쪽이 정본). 전체 규약 = `../_shared/self-improvement-feedback.md`.
