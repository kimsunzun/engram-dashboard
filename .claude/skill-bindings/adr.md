# adr 바인딩 — engram-dashboard

## 서기 스크립트 실명령

저장소 루트에서 실행한다. 모든 호출에 `--anchor-roots crates,src,src-tauri,scripts`를 붙인다 — 빠진 `lint`는 코드 앵커 검사를 건너뛴다. 폴더·인덱스 파일·상태 어휘·템플릿은 스크립트 기본값이 dashboard 실값이라 플래그를 주지 않는다.

```bash
node <skill>/scripts/adr.mjs new --title "<한 줄 제목>" [--status 확정|제안] --anchor-roots crates,src,src-tauri,scripts
node <skill>/scripts/adr.mjs supersede --old <N> --mode full --title "<새 제목>" --anchor-roots crates,src,src-tauri,scripts
node <skill>/scripts/adr.mjs supersede --old <N> --mode partial --clause "<바뀐 조항>" --title "<새 제목>" --anchor-roots crates,src,src-tauri,scripts
node <skill>/scripts/adr.mjs supersede --old <N> --clause "<조항>" --old <M> --clause "<조항>" --mode partial --title "<새 제목>" --anchor-roots crates,src,src-tauri,scripts
node <skill>/scripts/adr.mjs link --old <N> --new <M> --mode full --anchor-roots crates,src,src-tauri,scripts
node <skill>/scripts/adr.mjs link --old <N> --new <M> --mode partial --clause "<바뀐 조항>" --anchor-roots crates,src,src-tauri,scripts
node <skill>/scripts/adr.mjs link --old <N> --new <M> --mode partial --clause "<새 조항 전문>" --rewrite-clause --anchor-roots crates,src,src-tauri,scripts
node <skill>/scripts/adr.mjs lint --anchor-roots crates,src,src-tauri,scripts
node <skill>/scripts/adr.mjs index --check --anchor-roots crates,src,src-tauri,scripts   # 읽기 전용 — 쓰지 않고 diff만
node <skill>/scripts/adr.mjs index --write --anchor-roots crates,src,src-tauri,scripts   # 인덱스를 본문에서 다시 만들어 쓴다(커밋 안 함)
```

- 실데이터 밖 시험은 `--dir <임시 폴더> --no-git`으로 돌린다.

## ADR 폴더 · 파일명

- 폴더 = `docs/decisions/` · 파일명 = `NNNN-<슬러그>.md`(4자리 번호, 슬러그는 스크립트가 만든다). 예: `docs/decisions/0012-테스트-격리-하네스-tdd.md`.

## 스캐폴드 템플릿

- 스킬 표준 템플릿(`adr.template.md`)을 쓴다 — `--template`을 주지 않는다. 섹션 구조(맥락 · 결정 · 거부한 대안 · 근거 · 영향 / 불변식)가 `docs/decisions/README.md` 「템플릿」과 같다.

## 인덱스 위치 · 형식

- `docs/decisions/README.md`의 `## 인덱스` 표(`| # | 제목 | 상태 |`). 스크립트가 본문 H1·상태줄에서 다시 만든다 — 손으로 고치지 않는다.

## 상태 어휘

- `확정` · `제안` · `폐기` · `거부` — 범례 = `docs/decisions/README.md` 「상태 범례」.

## 앵커 명령

- 코드 앵커 루트 = `crates,src,src-tauri,scripts`(위 실명령의 `--anchor-roots`). 앵커 형식의 정본 = `.claude/skill-bindings/code-conventions.md` §주석.

## 채번 push

- push하지 않는다(`--push` 없음). 이 저장소의 워크트리들은 `.git` 하나를 같이 써서, 스크립트의 커밋은 다른 워크트리의 채번 스캔(`git log --all`)에 바로 보인다.
