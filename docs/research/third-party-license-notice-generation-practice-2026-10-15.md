# 서드파티 라이선스 고지 — 언제 만들고 어떻게 검증하나

- **상태:** medium(수집자 3 · 메인 grounding · codex 적대 리뷰 BLOCK → 지적 반영 · 아래 「적대 리뷰」). **결정(사용자 2026-10-15) = 릴리즈 빌드 때 생성 · 커밋 안 함**(옛 결정 「손으로 뽑아 커밋」을 바꿨다) — ADR-0294. 도구 선택 조사(`third-party-license-notice-survey-2026-10-15.md` — `v0.3.3/chore/misc` 브랜치)의 후속. **계기:** 사용자가 「손으로 뽑아 커밋」을 「릴리즈 뽑을 때 생성」으로 바꾸면서(2026-10-15) 「매번 뽑을 때 잘못 뽑히는 위험은 없나 · 다른 곳은 어떻게 하나」를 물었다.
- **날짜:** 2026-10-15
- **확신도 범례:** 확실 = 원문 직접 확인 + 독립 근거 · 가능성 높음 = 원문 확인(단일) 또는 수집자 보고를 메인이 부분 확인 · 불확실 = 수집자 보고만

## 결론

1. **본 Rust 데스크톱 표본에선 「번들(릴리즈) 때 생성 · 커밋 안 함」이 많았다 — 표본이 작아 주류 판단은 하지 않는다.** (가능성 높음 — 표본 한정)
   - Zed — 번들 스크립트가 먼저 생성한다: `script/bundle-mac:88-89` 「Generate the licenses first, so they can be baked into the binaries」 · 생성물은 `.gitignore:27` `/assets/*licenses.*` (메인 확인).
   - Warp · ALVR · Graphite 도 패키징 단계에서 cargo-about 를 돌린다(수집자 보고 · 메인 미확인 · 리비전 고정 출처 없음 — 불확실).
   - 반례: advancedfx(Rust 섞인 데스크톱)는 `misc/THIRDPARTY.yml` 을 커밋하고 Rust 의존이 바뀌면 검사한다(적대 리뷰가 든 반례 · 메인 미확인).
   - 다른 갈래: **커밋 + CI 최신 검사** = Signal Desktop(`.github/workflows/ci.yml:95-99` — 다시 생성 후 `git diff --exit-code` · 메인 확인) · libsignal(`--check` 모드). **커밋 + 손/사내 봇** = VS Code · PowerToys · Windows Terminal(수집자 보고).
2. **릴리즈에서 처음 터지지 않게, 같은 생성을 PR 에서도 돌리고 결과는 버린다.** (확실)
   - Zed `run_tests.yml` 의 `check_licenses` 잡(L769-792 · `main` 2026-10-15 기준)이 `script/generate-licenses` 를 돌린다 · `pull_request` 트리거 · 경로 필터 = `Cargo.lock` · `script/*licenses*` · `LICENSE*`(L117) (메인 확인).
   - ALVR `rust.yml` 도 `pull_request` 에 `check-licenses` 잡(L131-141) (메인 확인).
   - Warp 는 생성 대신 `cargo deny check licenses` 만 PR 에서 돌린다(수집자 보고).
3. **도구 버전을 고정한다.** (확실) Zed `script/generate-licenses:5` `CARGO_ABOUT_VERSION="0.8.2"` — 다르면 그 버전을 설치한다(L30-32) · `--fail` + stderr 에 무엇이든 찍히면 실패(`fail_on_stderr` · `ALLOW_MISSING_LICENSES` 로 끈다) (메인 확인). libsignal 은 버전 파일 `acknowledgments/cargo-about-version` 을 두고 다르면 거부한다(수집자 보고).
4. **cargo-about 0.9.2 의 「라이선스 파일 없는 crate」 대체는 `--fail` 로 안 잡힌다 — 표지는 `source_path` 가 비는 것 하나다(JSON 이든 템플릿이든).** (확실)
   - 0.9.2 `src/generate.rs:141-155` — 파일을 못 찾으면 `log::debug!` 만 남기고 SPDX 표준 본문(`id.text()`)을 `source_path: None` 으로 넣는다(주석은 「emit a warning」이라 하지만 실제는 debug) (메인 확인). Zed 는 0.8.2 를 쓰므로 이 0.9.2 소스로 Zed 의 stderr 검사가 이걸 잡는지는 말할 수 없다(불확실 — 0.8.2 소스 미확인).
   - 그 표준 본문의 MIT 는 `Copyright (c) <year> <copyright holders>` 자리표시를 그대로 싣는다(우리 생성물에서 관측).
   - libsignal 은 이걸 **표시하고 사람이 고친다** — 템플릿이 `{{#unless source_path}} (synthesized){{/unless}}` 를 붙이고(`acknowledgments.html.hbs:58` · 메인 확인), README 가 「새 synthesized 항목이 생기면 clarify 를 더하라」고 한다(수집자 보고). CI 가 자동으로 거부하지는 않는다.
5. **cargo-about 안에서 고치는 수단 = `clarify`(`files`/`git` 에 `path` + 체크섬).** (가능성 높음 — 수집자가 0.9.2 소스 인용 · 메인 미확인) `clarify.files`(로컬 파일 + 체크섬) 또는 `clarify.git`(Cargo.toml `repository` + `.cargo_vcs_info.json` 커밋으로 원본 저장소에서 받아 옴 · 네트워크 필요 · `repository` 없으면 오류). 내장 `workarounds` 목록(bitvec · chrono · clap · ring · rustls 등)에 `windows-*` · `tauri-*` 는 없다.
6. **generate-license-file 4.2.5 는 라이선스를 못 정한 패키지를 경고만 남기고 뺀다 — 그 패키지 하나 때문에 실패하지 않는다**(그 `catch` 밖의 오류는 여전히 실패한다). (확실) `resolveNpmDependencies.ts:55-88` — `catch` 에서 `logger.warn` 후 다시 던지지 않는다 (메인 확인). 실패로 바꾸는 옵션은 문서에 없다(수집자 보고).
7. **자리표시 저작권이 실린 고지는 큰 회사 저장소에도 있다** — microsoft/CsWin32 `NOTICE.txt` 등(수집자 보고 · 메인 미확인). 법적 판단은 하지 않는다.
8. **다른 도구 갈래:** cargo-bundle-licenses 는 못 찾은 본문을 「NOT FOUND」로 남겨 사람이 채운 커밋 파일을 다음 실행이 보존하고 `--previous --check-previous` 로 드리프트를 검사한다(적대 리뷰 제시 · 메인 미확인). cargo-deny 의 `license-files` clarify 는 파일을 검사할 뿐 고지를 만들지 않는다.
9. **찾지 못한 것:** 생성물을 의존성 그래프(`cargo metadata` · npm 트리)와 대조해 누락을 잡는 곳 · 자리표시를 grep 으로 거부하는 곳은 표본에서 못 찾았다. 관측된 장치는 「버전 고정 + `--fail` + PR 에서 같은 생성 + (커밋형이면) diff 검사 + synthesized 항목 사람 검토」다.

## Engram 에 옮기면 (세션 해석 — 결정은 사용자)

- 사용자가 고른 「릴리즈 때 생성」은 Zed 형과 같다(표본이 작아 주류 판단은 하지 않는다). 짝으로 **평소 CI 에서 같은 생성을 돌리는 잡**을 두면 태그 뒤에 처음 터지는 일을 막는다 — 우리는 태그 재시도가 안 되므로(CLAUDE.md 「검증」) 이 짝이 피어보다 더 필요하다.
- 우리 생성 스크립트가 이미 하는 것(허용 목록 · 판별 실패 · 우리 crate 혼입 · npm 의 조용한 누락 감지)은 피어보다 엄격한 쪽이다. 피어에 없는 것을 더하려면(누락 대조 · 자리표시 거부) 근거는 피어가 아니라 우리 요구다.
- **자리표시 갈래(약 50 crate + npm 4)** 는 선택지가 셋이다 — ① libsignal 처럼 `source_path == null` 항목에 「synthesized」 표시만 하고 둔다 ② crate 마다 `clarify` 를 더한다(`clarify.git` 은 릴리즈 빌드가 네트워크에서 원본을 받아 온다) ③ 자리표시를 실패로 만들어 ②를 강제한다. → 채택: ③ + 상류 저장소 교체 파일 — 남은 synthesized 는 siphasher 하나(ADR-0294 결정 5).

## 적대 리뷰

codex(effort high · web_search) 판정 **BLOCK** — 지적 6건을 전부 반영했다(위 본문에 녹임):
- 「주류」 = 작은 표본으로 우세를 말한 과장 → 표본 한정 표현으로 낮추고 반례(advancedfx)를 넣었다.
- Zed(0.8.2)에 0.9.2 소스 동작을 적용한 추론 → 불확실로 낮췄다.
- 「`source_path` 로만」 표현이 libsignal 템플릿과 어긋나 보임 → 표지가 하나라는 뜻으로 고쳐 적었다.
- 놓친 도구 갈래(cargo-bundle-licenses · clarify `path`) → 결론 8 에 더했다.
- 리비전 고정 없는 수집자 보고 → 불확실로 표시했다.
- cargo-about 0.9.2 `generate.rs:146-190` 의 `LicenseRef-*` 분기 결함을 codex 가 태그 소스로 확인했다고 한다(메인 미확인) — 우리는 `LicenseRef-*` clarify 를 쓰지 않아 지금은 무관하다. 쓰게 되면 먼저 확인한다.

BLOCK 사유는 전부 문서 표현 · 누락이었고, 「릴리즈 때 생성 + 평소 CI 에서 같은 생성」이라는 결정 근거를 뒤집는 반증은 없었다(advancedfx 는 본문의 「커밋 + 검사」 갈래 사례다).

## 한계

- 수집자 3 중 1 명(안전장치 갈래 첫 수집)은 얕았다(검색 10회 · 요약형 fetch) — 보강 수집으로 메웠다.
- Alacritty · WezTerm · Lapce · RustDesk · Spacedrive · Deno 는 확인 못 함(GitHub 코드 검색 한도). 패턴을 고른 **이유**를 적어 둔 곳은 Zed 의 번들 스크립트 주석 하나뿐이다.
- `LicenseRef-*` 분기 결함은 codex 만 확인했다(메인 미확인).
