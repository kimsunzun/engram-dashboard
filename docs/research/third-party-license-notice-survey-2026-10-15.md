# 오픈소스 라이선스 고지 파일 — 피어 방식과 도구

- **상태:** light(수집자 2 · 적대 리뷰 없음) · T-62(`docs/tracking.md`) 의 조사 · **결정(사용자 2026-10-15) = 손으로 뽑아 커밋 + build-release manifest 에 추가 · 갱신도 손으로 · 자동화는 나중**
- **날짜:** 2026-10-15
- **방법:** 수집자 2(① 피어 앱이 실제로 어떻게 하나 ② Rust · JS 도구 비교) · 메인 grounding(Zed 스크립트 원문 · cargo-about 설정 문서 · generate-license-file README).
- **계기:** 배포판(포터블 zip)에 제3자 라이선스 고지가 없다. 의존성은 Cargo + npm 둘이다.

## 결론

1. **Rust 쪽 표준은 cargo-about 이다.** 설정(`about.toml`) + 템플릿으로 라이선스 전문을 뽑는다. `accepted` = "Priority list of all the accepted licenses for a project" — 목록 밖 라이선스는 `--fail` 로 실패시킨다. `ignore-dev-dependencies` · `ignore-build-dependencies` 로 개발용 의존을 뺀다(설정 문서 원문 — 메인 확인). **가능성 높음.**
   - **Zed** — `script/generate-licenses` 가 cargo-about(0.8.2 고정)을 `cargo about generate --fail -c script/licenses/zed-licenses.toml script/licenses/template.md.hbs` 로 돌려 `assets/licenses.md` 를 만들고, 테마 · 아이콘 라이선스를 앞에 이어 붙인다 · stderr 가 하나라도 나면 실패(`ALLOW_MISSING_LICENSES` 로 끔)(스크립트 원문 — 메인 확인). 번들 스크립트가 먼저 이걸 돌려 앱에 넣고, 앱 안에 「라이선스 열기」 동작이 있다(수집자 · **가능성 높음**). 허용 목록에 "AGPL should not be added" 주석(수집자).
   - **cargo-deny** 는 고지를 만들지 않고 **검사만** 한다(허용 목록 게이트) — GitButler 가 CI 에서 돌린다(수집자 · **가능성 높음**).
2. **npm 쪽은 generate-license-file 이 가장 맞다.** "Generates a text file containing all of the licences for your production dependencies" · `npx generate-license-file --input package.json --output third-party-licenses.txt --overwrite` · 설정으로 교체 · 제외 · 덧붙이기(README 원문 — 메인 확인). ISC · 4.2.5(2026-08-29 — 수집자). **가능성 높음.** license-checker-rseidelsohn 은 사실상 끝(후속 `@lizenz/checker` 는 0.0.x).
3. **Tauri 피어는 대부분 고지 파일을 안 싣는다(파일 이름 검색 기준).** Spacedrive(루트 `NOTICE` 는 자기 고지뿐 · About 은 자기 LICENSE 링크) · Cap · Clash Verge Rev 에서 못 찾음. 작은 Tauri 앱 vmark 는 `scripts/gen-third-party-licenses.mjs` 가 cargo-about(`src-tauri/about.toml`) + 자체 npm 스크립트로 `THIRD_PARTY_LICENSES.txt` 를 만든다(수집자 · **불확실** — 릴리즈 때만 만드는 파일은 이름 검색에 안 잡힌다).
4. **VS Code(대조)** — 루트에 `ThirdPartyNotices.txt` 를 커밋하고 앱에 싣는다. 생성은 사내 파이프라인이라 밖에서 재현 불가(수집자).
5. **Apache-2.0 의 NOTICE 파일 처리는 어느 도구 문서에도 없다** — 실제 의존성에서 NOTICE 파일이 있는 것을 따로 찾아 확인해야 한다.
6. Tauri 공식 문서에 제3자 라이선스 안내는 못 찾았다(수집자 · **가능성 높음**).

## Engram 에 옮기면 (세션 해석 — 결정은 사용자)

- cargo-about(Rust · 개발/빌드 의존 제외 · 허용 목록 + `--fail`) + generate-license-file(npm 운영 의존) → 한 파일 `THIRD_PARTY_LICENSES.txt` 로 합쳐 **저장소에 커밋**(사용자 「텍스트 파일 하나 추가」) → `scripts/build-release.ps1` manifest 에 그 파일을 더해 zip 에 싣는다(manifest 는 정확한 집합만 허용 — ADR-0100).
- 의존성이 바뀌면 같은 명령으로 다시 뽑아 커밋. CI 에서 「커밋된 파일이 최신인가」 검사는 나중.
- 생성 뒤 NOTICE 파일을 가진 의존을 따로 찾아 확인.

## 한계

- 적대 리뷰 없음. GitHub API 한도로 Zed 의 `assets/licenses.md` 커밋 여부 · GitButler · Clash Verge 의 실제 배송물은 확인 못 함. 릴리즈 때만 만드는 고지는 파일 이름 검색에 안 잡힌다.
