# 주석 보존 YAML 편집 — yaml-edit · yamlpatch 실측 (프리셋 파일)

| 항목 | 값 |
|---|---|
| 상태 | 실측 완료 · 편집기 추천(메인 세션 확인 대기) |
| 날짜 | 2026-10-31 |
| 방법 | 스파이크 — 세션 스크래치의 따로 된 cargo 프로젝트(저장소 밖)에서 프리셋 샘플 한 벌을 연산마다 새 사본으로 고쳐 원본과 `diff -U1` · 결과를 serde-saphyr 2.0.0 으로 다시 읽어 기대 값(원본을 읽은 값에 같은 연산을 serde_json 으로 적용한 것)과 대조 · 타입 있는 읽기(경로 문자열 \| `{path, backend}` untagged)도 함께 · 손 주석 14개의 남음/사라짐/중복 셈 · 줄 끝(CRLF/LF) 셈. rustc 1.95.0 · Windows 11 · MSVC |
| 버전 | `yaml-edit` **0.3.5**(최신 · 2026-10-22) — 대조로 TRD 가 적은 0.3.2 도 같은 표를 돌림 · `yamlpatch` **1.31.1**(최신 정식 · 2026-10-15 · 1.32.0-rc1 은 안 봄) + `yamlpath` 1.31.1 · 다시 읽기 = `serde-saphyr` 2.0.0(TRD 의 읽기 crate) |
| 질문 | TRD `S25-preset-library/trd.md` §5-1 0b — 손으로 쓴 프리셋 파일을 Engram 이 고칠 때(목록 끝 추가 · 중간 넣기 · 빼기 · 순서 바꾸기 · 맵 칸 바꾸기) 손 주석 · 모양이 남는가 |
| 산출물 | 스크래치(보존 안 함) — 결과는 이 문서가 정본 |

## 결론

- **고른다 = `yaml-edit` 0.3.5 — 조건 셋을 붙여서.** 끝 추가 · 중간 넣기 · 빼기 · 꼬리표 바꾸기(맵 칸 값 · 문자열↔맵) · `description` · 빈 목록 · CRLF · 한글 주석에서 손 주석을 잃지 않고 다시 읽은 값도 맞다. 조건:
  1. **순서 바꾸기는 그대로 쓸 API 가 없고, 자연스러운 길(뺀 노드를 다시 넣기)은 맵 항목에서 다시 못 읽는 YAML 을 낸다**(`backend:` 들여쓰기 두 겹 — 0.3.2 에서 되던 것이 0.3.5 에서 깨짐). 우회 = 뺀 노드의 글을 둘째 줄부터 들여쓰기를 걷고 다시 읽어 넣기 — 실측으로 통했다(그 항목 줄 끝 주석까지 따라감). 새 값으로 다시 만들어 넣기도 맞지만 그 항목 줄 끝 주석을 잃는다.
  2. 소스가 `[]` 인 목록에 넣으면 흐름 꼴 `[x]` 로 남는다 → 새 블록 `Sequence` 로 바꿔 끼운다(실측 ✅).
  3. 패치 판마다 동작이 바뀐다(0.3.2 → 0.3.5 사이 다섯 연산 결과가 달라짐 · 아래 대조) → `=0.3.5` 로 박고, **쓰기 전에 serde-saphyr 로 다시 읽어 기대 값과 대조**하는 문을 `format.rs` 에 둔다(어긋나면 쓰지 않는다 — 그때 할 일은 구현 판단). 이번에 잡힌 깨진 출력은 전부 이 대조가 잡는 종류였다.
- **`yamlpatch` 1.31.1 은 안 된다.** ① 중간 넣기 · 순서 바꾸기 연산이 없다(목록 통째 `Replace` = 목록 안 주석 4개 잃음) ② 문자열 → 맵이 깨진 YAML 로 거절되고, **맵 → 문자열은 말없이 틀린 값을 쓴다**(꼬리표를 떼라 했는데 `- path: …` 맵으로 남고 `backend` · 줄 끝 주석이 사라짐 · 순수 ASCII 최소 문서에서도 재현) ③ 하나 남은 항목 빼기가 그 줄 끝 주석에 한글이 있으면 **panic**(`line-index` invalid offset) · ASCII 면 `settings:` 키째 지운다 ④ CRLF 파일에 끝 추가하면 LF 가 섞인다 ⑤ 의존이 rustc **1.97** 을 요구한다(`tree-sitter-iter` ≥ 1.28 · 저장소 핀 1.95.0) — 여기서는 `--ignore-rust-version` 으로만 지었다.
- **두 crate 공통 한계 — 빼는 항목의 윗줄 주석은 남는다.** 줄 끝 주석은 함께 가지만 항목 위 주석 줄은 제자리에 남아 다음 항목의 주석처럼 보인다. 이웃 주석은 그대로다. 같은 까닭으로 `yaml-edit` 의 중간 넣기는 새 항목이 **다음 항목의 윗줄 주석 아래**로 들어간다. 받아들일지는 사용자 몫 — 아니면 `format.rs` 가 주석 줄을 따로 다뤄야 하는데 그 API 는 재지 않았다.
- TRD 0b 의 물러남(serde 로 다시 쓰고 손 주석 잃음)은 필요 없다 — 위 조건 셋이면 `yaml-edit` 로 선다.

## 연산 × 크레이트

✅ = 값 · 주석 · 모양 모두 맞음 · △ = 값은 맞고 주석 자리나 모양이 어긋남(또는 우회 필요) · ✗ = 연산 없음 · 오류 · 틀린 값 · 깨진 출력

| 연산 | yaml-edit 0.3.5 | yamlpatch 1.31.1 |
|---|---|---|
| 1 끝 추가 — 문자열(`skills`) | ✅ 마지막 항목 바로 아래(빈 줄 · 파일 끝 주석 앞) | △ 빈 줄 **뒤** · 파일 끝 주석 바로 위에 붙음(목록 안에 빈 줄) — 값은 맞음 |
| 1 끝 추가 — 맵 `{path, backend: codex}`(`instructions`) | ✅ 블록 맵 · 들여쓰기 맞음 | ✅ 같은 결과 |
| 2 중간 넣기(`instructions` [0] 뒤) | △ `insert(1, …)` — 값 맞음 · 새 항목이 다음 항목의 윗줄 주석 아래로 들어가 그 주석이 새 항목 몫처럼 보임 | ✗ 연산 없음 · 목록 통째 `Replace` = 목록 안 주석 4개 잃음 |
| 3 빼기 — 문자열(윗줄 주석 + 줄 끝 주석) | △ 항목 줄 + 줄 끝 주석 사라짐 · **윗줄 주석 남음** · 이웃 주석 그대로 | △ 같은 결과(글자까지 같음) |
| 3 빼기 — 맵 항목(윗줄 주석 + `path` 줄 끝 주석) | △ 같은 꼴 | △ 같은 결과 |
| 4 순서 바꾸기(`instructions` [1] 을 끝으로) | ✗ 그대로는 안 됨 — 뺀 노드를 `push`/`insert` = **다시 못 읽는 YAML**(`backend:` 6칸) · 문자열 노드 옮김 = 줄 끝 주석 잃음 · 자리마다 `set` = 주석이 자리에 남고 하나 잃음. **우회 ✅** 노드 글 → 둘째 줄부터 4칸 걷기 → 다시 읽어 `push` = 값 맞음 · 줄 끝 주석 따라감(윗줄 주석은 남음) | ✗ 연산 없음 · `Remove` + `Append` = 값 맞음 · 줄 끝 주석 잃음 · 목록 통째 `Replace` = 주석 4개 잃음 · 자리마다 `Replace` = 오류(아래 문자열 → 맵 결함) |
| 5 꼬리표 값 바꾸기(`backend: claude` → `codex`) | ✅ 줄 끝 주석 그대로 | ✅ 같은 결과 |
| 5 문자열 → 맵(꼬리표 붙이기) | △ 값 맞음 · 줄 끝 주석은 남되 대시 줄로 감(`-      # 커밋 끝 주석` 다음 줄부터 맵) | ✗ 오류 — 깨진 YAML 을 만들어 거절(ASCII 최소 문서에서도) |
| 5 맵 → 문자열(꼬리표 떼기) | ✅ `path` 줄 끝 주석이 새 줄에 남음 | ✗ **말없이 틀린 값** — `- path: 지침/codex 보고 형식.md` 맵으로 남고 `backend` · 줄 끝 주석 사라짐 |
| 6 `description` 바꾸기 | ✅ 줄 끝 주석 그대로 | ✅ 같은 결과 |
| 빈 목록 `settings: []` → 첫 항목 | △ `push` = `[설정/첫 설정.yaml]` 흐름 꼴 · 새 블록 `Sequence` 로 바꿔 끼우면 ✅(키 줄 주석 그대로) | △ `Append` = 오류(흐름 목록 거절) · `Replace` = 블록이 되나 키 줄 주석이 항목 줄로 옮겨감 |
| 값 없는 `settings:` → 첫 항목 | ✅ 새 블록 `Sequence` 끼우기 · 키 줄 주석 그대로 | △ `Append` = 오류 · `Replace` = 키 줄 주석이 항목 줄로 |
| 하나 남은 항목 빼기 → 빈 목록 | ✅ `settings: []`(그 항목 줄 끝 주석은 함께 사라짐) | ✗ **panic**(줄 끝 주석 한글) · ASCII 면 `settings:` 키째 지움(yamlpath 의 「빈 블록 부모를 위로 접는다」 설계) |
| CRLF 입력(1 · 2 · 3 · 4 · 5 문자열→맵 · 6) | ✅ 결과 전부 CRLF 한 가지 · CR 을 걷으면 LF 결과와 바이트 같음(4 자연 경로는 LF 와 똑같이 깨짐) | △ 끝 추가 = LF 섞임(CRLF 30 + LF만 1~2 — 원래 줄 하나도 CR 을 잃음) · 빼기 · `description` = CRLF 유지 |
| 한글 주석 · 값 | ✅ 문제 없음 | ✗ 하나 남은 항목 빼기 panic(위) · 그 밖은 문제 없음 |
| 다시 읽기(serde-saphyr 2.0.0) | 우회 안 쓴 순서 바꾸기(노드 다시 넣기)만 실패 — 나머지 전부 기대 값과 같음 | 출력이 나온 것 중 맵 → 문자열만 다름(틀린 값) |
| 손대지 않은 줄 | 바이트 그대로(따옴표 `"공용/팀 규칙.yaml"` · 정렬 공백 포함 — diff 로 확인) | 같음 |

**대표 출력(diff)** — yaml-edit 0.3.5 순서 바꾸기 자연 경로(다시 못 읽음 · `comment intercepting the multiline text`):

```diff
     backend: claude          # 꼬리표 끝 주석
+  - path: 지침/codex 보고 형식.md   # codex 에이전트에만
+      backend: codex
```

yamlpatch 맵 → 문자열(말없이 틀린 값):

```diff
   # codex 쪽 보고 형식 — 꼬리표 달린 항목
-  - path: 지침/codex 보고 형식.md   # codex 에이전트에만
-    backend: codex
+  - path: 지침/codex 보고 형식.md
```

## yaml-edit 0.3.2 대조(TRD 가 적은 판)

- `Document::from_str` 가 머리 주석 있는 파일을 거절한다(「stream-level comments … Use YamlFile::from_str()」) — 0.3.5 는 둘 다 받는다. 대조 표는 두 판 모두 `YamlFile::from_str` 로 돌렸다(0.3.5 결과는 `Document` 로 돌린 것과 같음).
- 0.3.5 보다 나쁜 것: 맵 끝 추가 = `backend:` 가 0칸으로 가 값이 바뀜(다시 읽기 다름) · 문자열 → 맵 = 다시 못 읽는 YAML · 빼기 = 줄 끝 주석이 혼자 남은 줄로 남음 · CRLF 에 LF 섞임.
- 0.3.5 보다 나은 것: 맵 노드 옮기기(뺀 노드 다시 넣기)가 됐다 → 0.3.5 에서 깨짐(회귀).
- → 0.3.3 ~ 0.3.5 가 10-21 ~ 10-22 이틀에 셋 나왔고 판 사이 동작이 크게 바뀐다. 정확한 판 고정 + 골든 시험 + 쓰기 전 다시 읽기 대조가 필요하다(결론 조건 3).

## 크레이트 사실

| | yaml-edit | yamlpatch |
|---|---|---|
| 라이선스 | Apache-2.0(crates.io 메타) · GitHub 저장소 루트에 LICENSE 파일 없음(GitHub 감지 = 없음) | MIT |
| 저장소 · 활동 | `jelmer/yaml-edit` · ★12 · fork 10 · 열린 이슈 1 · 마지막 push 2026-10-22 · 2026-08-01 뒤 커밋 ≥ 100(API 한 쪽 상한) | `zizmorcore/zizmor` 의 `crates/yamlpatch`(★6.8k · 열린 이슈 172 · 마지막 push 2026-10-30) · 2026-08-01 뒤 그 폴더 커밋 3 · zizmor 와 같은 판 번호로 나감 |
| 최근 판 | 0.3.5 10-22 · 0.3.4 · 0.3.3 10-21 · 0.3.2 09-17 · 0.3.1 08-31 · 0.3.0 08-25 · 0.2.3 07-01 | 1.31.1 · 1.31.0 10-15 · 1.30.1 09-09 · 1.30.0 08-30 · 1.29.0 08-01 · (1.32.0-rc1 10-23) |
| 내려받기 · 역의존 | 누적 24.9만 · 최근 90일 20.7만 · 역의존 21(wasmer-cli · lintian-brush · debian-lsp 등) | 누적 40.0만 · 최근 90일 20.4만 · 역의존 7(zizmor 등) |
| MSRV · Windows 빌드 | 선언 1.70 · rustc 1.95.0 + MSVC 로 지어짐(rust-version 검사에 안 걸림) | 선언 없음 · 의존 `tree-sitter-iter` 1.31.1 이 **1.97.0** 선언 → rustc 1.95.0 에서 `cargo build` 가 거절 · `--ignore-rust-version` 이면 지어짐(약 30초 · C 컴파일 = tree-sitter · cc). `tree-sitter-iter` 가 1.88 을 선언한 마지막 판 = 1.27.0 → yamlpatch 1.27.0 이 1.95 로 설 것(안 돌려 봄) |
| 의존 무게(`cargo tree -e normal,build --target all` · 자기 포함 고유) | 7 — rowan · countme · hashbrown 0.14 · rustc-hash · text-size · base64(기본 기능 `binary` — 끌 수 있음) | 45 — tree-sitter · tree-sitter-yaml(C) · cc · yaml_serde · libyaml-rs · regex · tracing · serde_json · indexmap 등 |
| 저장소 `Cargo.lock` 에 이미 있나 | rowan · countme · text-size 없음(새로 들어옴) | tree-sitter · tree-sitter-yaml · yaml_serde · line-index 없음 |

## 실측에 쓴 샘플(LF · UTF-8)

목록 순서는 지시대로 `presets` · `instructions` · `settings` · `skills`(TRD §1-2 예는 `skills` 가 `settings` 앞 — 편집 동작과 무관).

```yaml
# PR 리뷰 프리셋 — 손으로 쓴 머리 주석
# 두 번째 머리 줄 (Engram 이 고쳐도 남아야 한다)
kind: preset
version: 1
description: PR 리뷰 전용   # 한 줄 설명

presets:
  - 공용/기본 규칙.yaml      # 공용 바닥
  # 아래는 팀 공용
  - "공용/팀 규칙.yaml"

instructions:
  - 지침/리뷰 규칙.md
  # codex 쪽 보고 형식 — 꼬리표 달린 항목
  - path: 지침/codex 보고 형식.md   # codex 에이전트에만
    backend: codex
  - 지침/커밋 메시지.md      # 커밋 끝 주석
  - path: 지침/claude 전용.md
    backend: claude          # 꼬리표 끝 주석

settings:
  - 설정/claude opus.yaml    # 설정 줄 주석

skills:
  - 스킬/리뷰/review         # 리뷰 스킬
  # lint 위 주석 — 이 항목 몫
  - 스킬/리뷰/lint           # lint 끝 주석
  - 스킬/공용/commit

# 파일 끝 주석
```

변형: CRLF = 같은 글의 `\n` → `\r\n` · 빈 목록 = `settings` 두 줄을 `settings: []   # 설정 줄 주석` 로 · 값 없음 = `settings:   # 설정 줄 주석` 로.

연산 대상: 1 = `skills` 에 `스킬/공용/새 스킬` · `instructions` 에 `{path: 지침/새 codex 지침.md, backend: codex}` / 2 = `instructions` [0] 뒤에 `지침/새 지침.md` / 3 = `skills` [1](lint) · `instructions` [1](codex 맵) / 4 = `instructions` [1] 을 끝으로 / 5 = `instructions` [3] `backend` → `codex` · [2] 를 `{path, backend: claude}` 로 · [1] 을 문자열로 / 6 = `PR 리뷰 · 보안 점검`.

## 주의

- 샘플 하나 · 들여쓰기 꼴 하나(키 아래 2칸 들여 쓴 `  - `). 대시를 키와 같은 칸에 두는 꼴(0칸 `- x`) · 손으로 쓴 흐름 목록 · 탭 · BOM · 앵커 · 여러 문서는 안 돌렸다.
- 순서 바꾸기 우회(들여쓰기 걷기)는 이 샘플의 맵 항목(내용 칸 4) 하나로 LF 에서만 쟀다 — 칸을 노드에서 읽는 일반화 · CRLF 는 안 했다.
- yamlpatch 결과는 rustc 1.95 에서 `--ignore-rust-version` 으로 지은 것이다 — 선언 MSRV 1.97 에서는 돌리지 않았다. 1.27.0 등 옛 판도 안 돌렸다.
- 다시 읽기 대조의 기대 값은 원본을 serde-saphyr 로 읽은 값에 같은 연산을 serde_json 으로 적용한 것 — 의미 대조다. 모양(주석 자리 · 빈 줄 · 들여쓰기) 판정은 diff 를 눈으로 봤다.
- 타입 있는 읽기 곁 사실: `settings:`(값 없음)를 `#[serde(default)] Vec` 로 읽으면 serde-saphyr 2.0.0 은 빈 목록으로 받는다(오류 아님).
- 빼는 항목의 윗줄 주석을 함께 지우는 길(yaml-edit 의 주석 노드 API 등)은 재지 않았다.

## 스파이크 코드

세션 스크래치 `C:\Users\kimsunzun\AppData\Local\Temp\claude\…\scratchpad\yaml-spike\`(저장소 밖 · 보존 안 함) — `src/main.rs`(연산 표 · 다시 읽기 대조 · 주석 셈 · 줄 끝 셈) · `src/bin/dbg.rs`(yamlpatch 결함 좁히기 — ASCII · 한글 최소 문서). 저장소에는 코드 · 의존을 더하지 않았다.
