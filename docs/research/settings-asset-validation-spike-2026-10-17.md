# ★ 설정 에셋 내용 확인 — claude · codex 실측 (스키마 · 실행 때 반응)

| 항목 | 값 |
|---|---|
| 상태 | 실측 완료 · 결정 대기(사용자) |
| 날짜 | 2026-10-17 |
| 방법 | 스파이크 — 워커 1명이 실제 명령을 순서대로 실행(claude 2.1.284 · codex-cli 0.156.1 · Windows) · 스키마 내려받기 · ajv 8.20 로 대조 · 메인이 산출물 파일로 스팟 확인(`Invalid settings` 출력 · codex 경고 · `--strict-config` 오류 문구). 비용 ≈ claude $0.33 + codex 한 번 21k 토큰 · 전역 설정 파일 변경 없음 |
| 질문 | 설정 에셋의 원문 칸(claude = `--settings` 로 넘길 객체 · codex = `-c 키=값` 목록) 내용을 실행 전에 확인할 수 있나 — 공개 스키마를 처음에 내려받아 쓰기 · 안 되면 실행 중 에러 · 문제는 목록에 아이콘으로 |
| 산출물 | 세션 스크래치(지워질 수 있음) — 명령 · 출력 원문은 이 문서에 옮긴 것이 정본 |

## 결론

- **claude — ★실행 때는 조용히 실패한다★.** 타입이 틀린 키가 하나라도 있으면 `--settings` 객체 **전체를 말없이 버리고** exit 0 으로 돈다(`{"cleanupPeriodDays":"ten","model":"haiku"}` → opus 로 실행 · 대조 `{"model":"haiku"}` → haiku · stderr · `--debug-file` 에도 없음). 모르는 키는 그 키만 무시. 그러니 **실행 전 확인이 꼭 필요하다.**
- **claude — 모델 호출 없는 확인 = `claude --settings '<json>' doctor`**(약 2초) — `Invalid settings` / `- …\claude-settings-<hash>.json › cleanupPeriodDays: Expected number, but received undefined` 를 찍는다. ★exit code 는 틀려도 0★ — 출력 글을 읽어야 한다(문구는 문서화된 계약이 아님). 모르는 키는 안 잡는다.
- **claude 공개 스키마 = SchemaStore** `https://json.schemastore.org/claude-code-settings.json`(→ 301 → 200 · 231,533 B · draft-07 · 최상위 속성 142 · 최상위 `additionalProperties: true`). Anthropic 설정 문서가 이 스키마를 가리키며 「최신 CLI 보다 늦을 수 있다」고 적는다. 타입 오류는 잡고(`cleanupPeriodDays must be integer` · `permissions/allow must be array`) 모르는 키는 못 잡는다. ajv strict 모드는 컴파일 실패(`unknown keyword: "allowTrailingCommas"`) → 비엄격으로.
- **codex — 판(버전)에 맞는 스키마가 공개돼 있고 엄격하다** — `https://github.com/openai/codex/releases/download/rust-v0.156.1/config-schema.json`(200 · 220,935 B · 최신 = `https://developers.openai.com/codex/config-schema.json` · 원본 `codex-rs/core/config.schema.json` · draft-07 · `additionalProperties:false`). 모르는 키 · 열거 값 오류 · 정수 칸 타입 오류는 잡고, 자유 문자열 칸(`model` · `model_reasoning_effort`)은 통과한다.
- **codex 실행 때:** 모르는 키 = 경고 후 계속(`Codex is ignoring 1 unrecognized configuration setting … notARealKey is ignored`) · `--strict-config` 를 주면 모델 호출 전 exit 1(`unknown configuration field \`notARealKey\` in -c/--config override`) · 열거 값 오류(`approval_policy=bogus`) = 모델 호출 전 exit 1 · `model_reasoning_effort=hihg` · `model=[unclosed` = 로컬 통과 → **API 에서** exit 1(허용 값 목록이 오류에 나옴). TOML 이 아닌 값은 글자 그대로 문자열이 된다(`--help` 에 적힌 동작).
- **codex 모델 호출 없는 확인:** `codex doctor --json -c <kv>`(약 2초 · 모르는 키 = warning · 열거 오류 = fail 이지만 세부는 「invalid data」 · `hihg` = ok) · `codex debug models`(1초 미만 · 모델 이름과 `supported_reasoning_levels` 목록).
- **yaml 문법 = serde-saphyr 2.0.0**(MIT OR Apache-2.0 · MSRV 1.89) — `Error::location() -> Option<Location>` · `line()/column()` 1부터 · 문자 기준 → 「N째 줄」 가능. 위치가 `Option` 이라 줄이 없는 오류도 있다. (docs.rs — 컴파일은 안 해 봄)
- **Rust 스키마 검사 후보:** `jsonschema` 0.59.0(MIT · MSRV 1.85 · 기본 기능이 reqwest/rustls 를 끌어오므로 `resolve-http` 끄기) · `boon` 0.6.1(MIT OR Apache-2.0 · 마지막 릴리스 2025-01) · `valico` 4.0.0(2023-05 이후 정체).

## 실측 표

| 경우 | 명령 | 결과 |
|---|---|---|
| claude 깨진 JSON ① `{"env": {"X": "1"` | `claude -p "reply with the word ok" --max-turns 1 --no-session-persistence --output-format json --settings <x>` | exit 1 · 모델 호출 전 · `Error: Settings file not found: <cwd>\{"env": {"X": "1"` — 글을 파일 경로로 읽었다 |
| claude 깨진 JSON ② `{"env": {"X": "1"}, oops}` | 같음 | exit 1 · `Error: Invalid JSON provided to --settings` |
| claude 모르는 키 `{"notARealKey":true}` | 같음 | exit 0 · 그 키만 조용히 무시(다른 키는 적용 — `+"model":"haiku"` → haiku) |
| claude 타입 오류 `{"cleanupPeriodDays":"ten"}` | 같음 | exit 0 · **객체 전체를 조용히 버림**(대조 실행으로 확인) |
| claude 없는 모델 `{"model":"no-such-model"}` | 같음 | API 까지 감 · exit 1 · `[claude-code:unrecognized_model]` · 404 · 비용 0 |
| codex 모르는 키 `-c notARealKey=1` | `codex exec --skip-git-repo-check --ephemeral -s read-only -c <kv> "reply with the word ok"` | exit 0 · 경고 후 실행 · `--strict-config` = exit 1(모델 호출 전) |
| codex 틀린 값 `-c model_reasoning_effort=hihg` | 같음 | 로컬 통과 · API 에서 exit 1 — 「Invalid value: 'hihg'. Supported values are: 'none', 'minimal', 'low', 'medium', 'high', 'xhigh', and 'max'.」 |
| codex TOML 아닌 값 `-c 'model=[unclosed'` | 같음 | 문자열로 받아 API 에서 exit 1 · 정수 칸이면 로컬에서 `expected i64` |
| codex 열거 오류 `-c approval_policy=bogus` | 같음 | 모델 호출 전 exit 1 · `unknown variant \`bogus\`` |

## 실측자 제안 흐름(결정 아님)

1. 저장 · 훑기 때(데몬 · 오프라인): serde-saphyr 로 문법 → 「N째 줄」 · 내려받아 둔 스키마로 내용 검사(codex = `codex --version` 에 맞는 릴리스 스키마 · claude = SchemaStore + 「스키마에 없는 키」 따로 검사) — 스키마가 CLI 보다 늦을 수 있어 **경고** 아이콘.
2. 실행 직전(모델 호출 없음): `claude --settings '<json>' doctor` 출력의 `Invalid settings` 줄 → **오류** 아이콘 · codex 는 `codex debug models` 로 모델 · 강도 값 확인 · `--strict-config`(모르는 키를 오류로)는 사용자 결정.
3. 실행 중: exit code + stderr 를 목록 아이콘으로 — claude `Invalid JSON` · 모델 404 · codex `Error loading config.toml` · API 400.

## 못 잰 것

- claude 대화형 TUI · stream-json 모드(`-p` 만 돌림 — TUI 는 설정 대화상자를 띄울 수도 있다).
- doctor 출력 문구가 판이 바뀌어도 그대로인지(문서화 안 됨).
- Rust `jsonschema` · serde-saphyr 를 실제로 컴파일해 돌리지 않음(ajv 가 대신) · schemars 의 `uint*` 형식 처리 미확인.
- `codex debug models` 가 네트워크를 타는지.
