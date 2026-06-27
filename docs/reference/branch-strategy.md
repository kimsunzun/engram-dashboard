# 브랜치 전략 (engram-dashboard)

**상태:** draft — `/review doc` 대기(canonical·master 머지 전 검토 필요). 멀티 에이전트(dashboard1·2…) 협업 컨벤션.
**적용:** 작업 브랜치 네이밍·lifecycle. (왜 이 형태인지·버린 대안 상세는 추후 ADR.)

## 네이밍

- 작업 브랜치 = `wip/aN` (a1, a2, …).
- 규칙: **비어 있는 가장 낮은 번호**를 쓴다 — `wip/a1`이 없으면 a1, 살아 있으면 a2로 올린다.
- 브랜치명은 *내용*이 아니라 *작업 흐름*을 나타낸다(체크포인트엔 단일 주제명이 안 나오므로). 세션 라벨(`dashboard1`)·날짜는 안 쓴다(삭제-재생성하면 충돌 없음).

## Lifecycle

`branch → 작업 → master 머지 → 브랜치 삭제(로컬+원격) → 다음 작업 때 다시 `wip/a1` 생성`.

- 삭제하면 이름이 비므로 번호가 안 올라간다. 번호가 오르는 건 이전 브랜치가 *아직 살아있을* 때뿐.
- 삭제: `git branch -d wip/aN`(로컬, 머지됐으면 `-d`/안 됐으면 `-D`) + `git push origin --delete wip/aN`(원격).

## 통합(integration) — 두 방향

- **master → wip/aN** (브랜치 신선 유지): dashboard2가 master에 계속 커밋하므로, 주기적으로 `git merge master`로 당겨와 나중 충돌을 줄인다. 오래 끌수록 어긋남이 커진다.
- **wip/aN → master** (작업 전달): **`/review code` + QA 통과분만** master로 머지한다. 미검증 WIP를 master에 올리지 않는다.

## 전제 — 같은 트리 공유 (중요)

현재 에이전트들은 **하나의 working tree(단일 worktree)를 공유**한다 → HEAD가 하나뿐이라 **동시에 서로 다른 브랜치에 있을 수 없다.**

- 그래서 **순차로** 쓰거나(한 번에 한 에이전트), 한쪽이 master 고정이어야 안전하다.
- 동시 작업 중엔 `git checkout`(공유 HEAD 이동)·`git add -A`(상대 파일까지 staging)를 피하고 **자기 파일만** 커밋한다.
- "진짜 동시에 각자 분기"가 필요해지면 **git worktree**(에이전트별 작업 폴더, `.git` objects 공유)로 격리한다 — 지금은 비채택(과함). 별도 클론은 더 무겁다(objects 2벌·remote 거쳐야 공유).
