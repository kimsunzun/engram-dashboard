# claude 설정 스키마 — 어디서 얻나 · 우리가 직접 검사해도 되나

- **상태:** light 조사 + 실측 · 2026-10-18 · 수집자 1(소스·바이너리 판정) + 메인 grounding(공식 문서 문장 · 추출 파일 대조) · 적대 리뷰 없음
- **질문:** `claude --settings <json> doctor`(약 2초)를 돌리는 이유는 claude 안의 설정 스키마로 검사받기 위해서다. 그 스키마를 직접 얻거나 공개 스키마로 Engram 이 즉시 검사(문법 · 키가 있나 · 값이 맞나)할 수 있나.

## 결론

- **claude 내부 스키마(zod)를 JSON 스키마로 꺼낼 수는 있다 — 단 공개된 길이 아니다.** 번들된 `update-config` 스킬이 실행 중에 내부 zod 스키마를 JSON 스키마로 바꿔 프롬프트에 싣는다. 가짜 로컬 API 로 모델이 그 스킬을 부르게 해 잡아냈다(모델 호출 없음 · 약 5초 · claude 2.1.284 → draft 2020-12 · 261 KB · 맨 위 키 176개). 스키마를 찍는 명령 · 플래그 · 패키지 안 스키마 파일은 없다. 버전마다 깨질 수 있다. — 확실(실측 · 추출 파일 확인)
- **SchemaStore 스키마는 그것과 다른, 커뮤니티가 손으로 관리하는 사본이고 늦게 따라간다.** 문서와 변경 기록을 보고 손으로 맞추며(PR #6131 「reconciled against the full settings.md and env-vars.md reference tables」), 마지막 버전 맞춤 = v2.1.220(2026-07-27). 2.1.284 내부 스키마와 비교하면 48 키 없음 · 14 키 남음 · `managedMcpServers` 타입 다름. — 확실(실측 비교 · 커밋 기록)
- **공식 문서는 SchemaStore 를 가리킨다** — 「The `$schema` line points to the published JSON schema for Claude Code settings, which gives you autocomplete and inl[ine validation]」(https://code.claude.com/docs/en/settings · 원문 대조). 편집기 자동 완성 · 검사용으로 권한다. 공식 · 자동 생성이라는 문장은 없다. — 확실
- **버전별 claude 설정 스키마를 Anthropic 이 내놓는 곳은 못 찾았다** — 가능성 높음
- **codex 는 릴리스마다 `config-schema.json` 을 함께 낸다**(코드에서 생성 · 그 버전과 정확히 맞음). — 확실(자산 존재) · 가능성 높음(생성 방식)

## 함의

- 내부 스키마를 쓰면 claude 와 거의 같게 즉시 검사하지만, 꺼내는 길이 내부 구현(스킬 이름 · 프롬프트 머리글)에 기대 언제든 깨진다.
- SchemaStore 를 쓰면 가볍고 공식 문서가 권하는 길이지만, 새 키를 모르는 키로 볼 수 있고(48개) 타입이 어긋나는 키가 있다 → 「모르는 키」 를 오류로 잡으면 오경보가 난다.
- 둘 다 맨 위에서 모르는 키를 허용한다 — 「키가 있나」 는 우리가 `properties` 와 따로 비교해야 한다.

## 공백

- 추출한 내부 스키마로 Rust `jsonschema` 검사를 돌려 보지 않았다. zod 의 `.catch` · refine 처럼 JSON 스키마로 안 옮겨지는 규칙의 범위 · 스킬의 후처리가 지우는 것은 미확인.
- 추출 실측 파일(세션 스크래치 · 저장소 밖): `internal-settings-schema.json` · `cap2.js`.
