# 브랜치·커밋 절차

규칙은 `../../CLAUDE.md` 「브랜치·커밋」. 이 문서는 주제를 시작할 때(`S<n>` 받기)와 통합할 때 여는 절차다.

## 주제 시작 — `S<n>` 받기

- 자기 주제 브랜치를 팔 때 번호를 받는다. 그 번호를 커밋 접두 · 설계 폴더 `docs/process/S<n>-<슬러그>/` · step-log 헤딩 `## S<n> — …` 에 함께 쓴다.
- 고르는 법 = `git fetch` 뒤 모든 ref 를 훑은 가장 큰 정수 n + 1. 지금 브랜치만 보면 워크트리끼리 같은 번호가 나온다. step-log 는 시간순이 아니라 마지막 헤딩을 잇지 않는다. n 은 정수부다(`S20.19` = 20).
  ```bash
  git log --all --format=%s | grep -oE '^S[0-9]+' | tr -d S | sort -n | tail -1
  git log --all --name-only --format= -- docs/process | grep -oE 'docs/process/S[0-9]+' | tr -dc '0-9\n' | sort -n | tail -1
  for r in $(git for-each-ref --format='%(refname)' refs/heads refs/remotes); do MSYS_NO_PATHCONV=1 git show "$r:docs/process/step-log.md" 2>/dev/null | grep -oE '^## S[0-9]+'; done | tr -dc '0-9\n' | sort -n | tail -1
  ```
- 받으면 step-log 헤딩과 설계 폴더 안 파일 하나(git 은 빈 폴더를 싣지 않는다)를 바로 커밋 · push 해 선점한다.
- 잡동사니 브랜치(`v<다음 릴리스>/chore/misc`)의 자잘한 커밋은 번호를 새로 받지 않는다 — 그 브랜치가 써 온 번호를 쓰고, 새로 판 잡동사니 브랜치면 그때의 가장 큰 번호를 쓴다(`docs/tracking.md` T-69 가 정할 때까지).
- 잡동사니 브랜치에서 하던 일이 주제로 커지면 그때 자기 브랜치로 떼고 번호를 받는다.
- 이미 있는 `docs/process/S21-*` 폴더 이름은 바꾸지 않는다 — 코드 주석과 핸드오프 기록이 그 경로를 가리킨다.

## 통합 — push → CI 초록 → `--no-ff` 머지

- Read CI for the exact SHA you merge: `gh run list -w ci.yml -c <SHA> --json databaseId,event,status,conclusion`. Green means a `push` or `workflow_dispatch` run (the manual run described below) with `status` `completed` and `conclusion` `success`.
- `cancelled`, `skipped` or no run at all is not green.
- `gh run watch <id>` exits 0 on a failed run unless you pass `--exit-status`. Judge by the run's `conclusion`, not by the exit code of `watch`.
- fast-forward 하지 않는다 — 머지 커밋이 덩어리 경계를 그래프에 남긴다.
- 머지 커밋 본문에 들어간 것을 항목으로 적는다 — 브랜치 이름이 아니라 이 본문이 「그 덩어리가 무슨 작업이었나」를 남긴다.
- 머지한 브랜치는 지우지 않는다.

### CI 초록 없이 머지하는 예외 — 경로 필터가 빼는 경로만 바뀐 변경

빠지는 경로 = `.github/workflows/ci.yml` 의 `on.push.paths`(ADR-0232). 「문서만 고쳤다」는 기억으로 판정하지 않고 아래로 잰다.

- CI 실행이 하나도 없는 브랜치: `git diff --name-only master...<브랜치>` 가 전부 빠지는 경로 안이어야 한다.
- 실행이 있는 브랜치: 마지막 초록 실행의 head SHA(`gh run list --branch <브랜치> --workflow ci.yml --json headSha,conclusion`) 뒤의 변경(`git diff --name-only <headSha> <브랜치>`)이 전부 빠지는 경로 안이어야 한다.
- 마지막 실행이 빨강 · 취소이고 그 뒤가 문서뿐이면 push 해도 실행이 생기지 않는다 — `gh workflow run ci.yml --ref <브랜치>` 로 띄운다(`workflow_dispatch` 에는 경로 필터가 없다). `gh run rerun` 은 옛 SHA 를 다시 돌릴 뿐이다.
- 판정이 애매하면 같은 명령으로 CI 를 띄운다.

### 체크아웃 없는 머지

어느 워크트리도 master 를 상시 체크아웃하지 않는다 — 붙들고 있으면 다른 워크트리가 통합하지 못한다. 쥐고 있으면 `git checkout --detach` 로 놓는다.

옛 master SHA 를 먼저 잡아 둔다(③ 이 쓴다).

1. `git merge-tree --write-tree master <브랜치>` → 첫 줄이 트리 해시. 종료코드 0 만 깨끗하다 — 충돌해도 트리 해시가 찍히고 그 트리엔 `<<<<<<<` 마커가 들어 있다. 충돌이면 멈추고 임시 워크트리(`git worktree add`)에서 평범한 `git merge` 로 푼다.
2. `git commit-tree <트리> -p master -p <브랜치> -F <본문파일>` — 부모 순서는 master 먼저다(뒤집으면 `--first-parent` 가 토픽을 탄다).
3. `git update-ref -m "<사유>" refs/heads/master <새 커밋> <옛 master SHA>` — 넷째 인자(옛 SHA)를 빼면 그 사이 다른 워크트리가 옮긴 master 를 덮어써 남의 머지가 사라진다.

- master 가 체크아웃된 채로 `update-ref` 하지 않는다 — 그 워크트리가 어긋난다.
- 밀기 전 확인: 양쪽 부모가 조상인가(`git merge-base --is-ancestor`) · 첫 부모가 옛 master 인가(`git rev-list --parents -n1 master` → 세 토큰 중 둘째) · 트리가 1 의 해시와 같은가(`git rev-parse master^{tree}`) · 실물이 실렸나(`git ls-tree`).
- 이 경로는 커밋 훅(`pre-commit` · `commit-msg` · `pre-merge-commit` 등)을 건너뛴다. push 의 `pre-push` 는 그대로 탄다.
