# 프리셋 YAML 을 고칠 때 손 주석 지키기 — Rust 라이브러리 · 관행

- **상태:** light 조사 · 2026-10-18 · 수집자 1(문서형) + 메인 grounding(docs.rs `yaml-edit` 대조) · 적대 리뷰 없음 · 라이브러리는 돌려 보지 않았다
- **질문:** 데몬이 프리셋 `.yaml` 의 목록(지침 · 스킬 · 설정 · 프리셋)에 항목을 넣고 빼고 순서를 바꿀 때, serde 로 통째로 다시 쓰면 사용자가 손으로 단 주석이 사라진다(TRD T2). 주석을 지키는 Rust 라이브러리가 있나 · 관행은 지키는 쪽인가.

## 결론

- **관행은 「지킨다」 다.** 사용자 파일을 고치는 도구가 주석을 날리면 불만이 나온다 — Home Assistant UI 편집기가 손 주석을 지우는 데 대한 불만 글(원인 = YAML → JSON → YAML 왕복) · openHAB 「Main UI widget editor deletes comments」. Python 쪽 사실상 표준 ruamel.yaml 은 왕복에서 주석 · 블록 모양 · 키 순서를 지키는 것이 존재 이유다. — 가능성 높음
- **쓸 만한 후보가 있다 — `yaml-edit`**(jelmer · 0.3.2 · 2026-09-17 · Apache-2.0 · rowan 무손실 트리). 「parsing and editing YAML files while preserving formatting, comments, and whitespace」(docs.rs 대조). 확인된 편집 = Mapping `set` · `remove` · `rename_key` · Sequence `push` · `set(index)`. **목록 중간 넣기 · 빼기 · 순서 바꾸기와, 빼는 항목에 붙은 주석의 처리는 문서로 확인 못 했다** → 구현 전에 실제 프리셋 파일로 스파이크가 필요하다. — 불확실(동작) · 확실(설명문)
- **비교 후보:** `yamlpatch`(zizmor · MIT · 1.31.1 · 2026-10-15 — Add · Remove · Append · Replace · 「best-effort」 · 「not a substitute for comprehensive YAML processing libraries」) · `noyalib` `Document`(손대지 않은 바이트는 주석까지 보존 · 넣기/빼기 미확인 · 0.0.x). 쓸 수 없음: `nondestructive`(「This project is as of yet, incomplete!」).

## 출처

| 항목 | 출처 | 확신도 |
|---|---|---|
| yaml-edit | https://docs.rs/yaml-edit · https://github.com/jelmer/yaml-edit | 설명 확실 · 목록 편집 불확실 |
| yamlpatch | https://github.com/zizmorcore/zizmor/tree/main/crates/yamlpatch | 가능성 높음 |
| noyalib | https://docs.rs/crate/noyalib/latest/source/examples/lossless_edit.rs | 가능성 높음 |
| nondestructive | https://docs.rs/nondestructive | 확실 |
| Home Assistant 불만 | https://community.home-assistant.io/t/automation-editor-changes/159712 | 확실(수집자) |
| openHAB 불만 | https://community.openhab.org/t/main-ui-widget-editor-deletes-comments/114084 | 가능성 높음(제목만) |
| ruamel.yaml | https://pypi.org/project/ruamel.yaml/ | 확실(수집자) |

## 공백

- 어느 라이브러리도 돌려 보지 않았다. VS Code jsonc `modify` · kubectl · Renovate 의 주석 처리는 1차 자료를 못 찾았다.
