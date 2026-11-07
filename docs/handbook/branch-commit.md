# Branch and Commit Procedure

The rules are in `../../CLAUDE.md` 「브랜치·커밋」. This document is the procedure you open when you start a topic (getting `S<n>`) and when you integrate.

## Starting a topic — getting `S<n>`

- When you cut your own topic branch, you get a number. Use that number together in the commit prefix · the design folder `docs/process/S<n>-<슬러그>/` · the step-log heading `## S<n> — …`.
- How to pick = after `git fetch`, scan all refs, take the largest integer n found, and use n + 1. Looking only at the current branch gives the same number in different worktrees. The step-log is not in time order, so do not continue from its last heading. n is the integer part (`S20.19` = 20).
  ```bash
  git log --all --format=%s | grep -oE '^S[0-9]+' | tr -d S | sort -n | tail -1
  git log --all --name-only --format= -- docs/process | grep -oE 'docs/process/S[0-9]+' | tr -dc '0-9\n' | sort -n | tail -1
  for r in $(git for-each-ref --format='%(refname)' refs/heads refs/remotes); do MSYS_NO_PATHCONV=1 git show "$r:docs/process/step-log.md" 2>/dev/null | grep -oE '^## S[0-9]+'; done | tr -dc '0-9\n' | sort -n | tail -1
  ```
- Once you have it, commit and push the step-log heading and one file inside the design folder (git does not carry empty folders) right away to claim it.
- Small commits on a miscellany branch (`v<다음 릴리스>/chore/misc`) do not get a new number — use the number that branch has been using, and for a newly cut miscellany branch use the largest number at that time (until `docs/tracking.md` T-69 decides).
- When work that was going on in a miscellany branch grows into a topic, split it off into its own branch at that point and get a number.
- Do not rename existing `docs/process/S21-*` folders — code comments and handoff records point to those paths.

## Integration — push → CI green → `--no-ff` merge

- Read CI for the exact SHA you merge: `gh run list -w ci.yml -c <SHA> --json databaseId,event,status,conclusion`. Green means a `push` or `workflow_dispatch` run (the manual run described below) with `status` `completed` and `conclusion` `success`.
- `cancelled`, `skipped` or no run at all is not green.
- `gh run watch <id>` exits 0 on a failed run unless you pass `--exit-status`. Judge by the run's `conclusion`, not by the exit code of `watch`.
- Do not fast-forward — the merge commit leaves the chunk boundary in the graph.
- List as items what went into the merge commit body — it is this body, not the branch name, that records 「그 덩어리가 무슨 작업이었나」 (what work that chunk was).
- Do not delete merged branches.

### Exception: merging without CI green — a change touching only paths the path filter excludes

The excluded paths = `on.push.paths` in `.github/workflows/ci.yml` (ADR-0232). Do not decide 「문서만 고쳤다」 ("I only changed docs") from memory; measure it as below.

- A branch with no CI run at all: `git diff --name-only master...<브랜치>` must be entirely inside the excluded paths.
- A branch with runs: the changes after the head SHA of the last green run (`gh run list --branch <브랜치> --workflow ci.yml --json headSha,conclusion`) (`git diff --name-only <headSha> <브랜치>`) must be entirely inside the excluded paths.
- If the last run is red or cancelled and only docs came after it, pushing does not create a run — start one with `gh workflow run ci.yml --ref <브랜치>` (`workflow_dispatch` has no path filter). `gh run rerun` only reruns the old SHA.
- If the decision is ambiguous, start CI with the same command.

### Merging without a checkout

No worktree keeps master checked out permanently — holding it keeps other worktrees from integrating. If you hold it, let go with `git checkout --detach`.

Capture the old master SHA first (step 3 uses it).

1. `git merge-tree --write-tree master <브랜치>` → the first line is the tree hash. Only exit code 0 is clean — even on a conflict a tree hash is printed, and that tree contains `<<<<<<<` markers. On a conflict, stop and resolve it with a plain `git merge` in a temporary worktree (`git worktree add`).
2. `git commit-tree <트리> -p master -p <브랜치> -F <본문파일>` — the parent order is master first (reversed, `--first-parent` rides the topic).
3. `git update-ref -m "<사유>" refs/heads/master <새 커밋> <옛 master SHA>` — if you leave out the fourth argument (the old SHA), you overwrite a master another worktree moved in the meantime, and someone else's merge disappears.

- Do not `update-ref` while master is checked out — that worktree goes out of sync.
- Check before pushing: are both parents ancestors (`git merge-base --is-ancestor`) · is the first parent the old master (`git rev-list --parents -n1 master` → the second of three tokens) · is the tree the same as the hash from step 1 (`git rev-parse master^{tree}`) · did the actual files get carried (`git ls-tree`).
- This path skips the commit hooks (`pre-commit` · `commit-msg` · `pre-merge-commit`, etc.). The push `pre-push` hook still runs.
