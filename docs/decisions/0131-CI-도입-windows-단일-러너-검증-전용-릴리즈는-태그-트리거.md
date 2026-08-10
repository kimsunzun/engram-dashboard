# ADR-0131: CI 도입 — windows 단일 러너 · 검증 전용 · 릴리즈는 태그 트리거

- 상태: 확정 (2026-08-09~11, 근거: 사용자 결정 + 적대 리뷰 3라운드 + 실행 실측)
- 관련: ADR-0003(코어 격리) · ADR-0110(메시징 커널 격리) · ADR-0129(네트워크 행 격리 게이트) · ADR-0100(릴리즈 co-location) · ADR-0012(모듈 격리 하네스) · step-log S18.26

## 맥락

이 프로젝트에는 CI가 **전혀 없었다**. 검증 명령은 CLAUDE.md 「빌드·검증 명령」에 다 정의돼 있었지만 전부 사람이 기억해서 치는 것이었다.

특히 **격리 게이트가 취약했다**. 코어의 tauri import 금지(ADR-0003), 메시징 커널 무의존(ADR-0110), 네트워크 행 의존 상한(ADR-0129) — 셋 다 컴파일러가 강제하지 않는다. 문법은 멀쩡하고 실행도 된다. 사람이 안 세면 조용히 무너지고, 실제로 discovery에 async 런타임이 조용히 반입된 사건이 있었다(2026-08-05, net `default = []` 결정의 계기).

여러 PC에서 같은 브랜치에 push하는 운용이라 "내 PC에선 되는데" 부류도 걸러낼 표면이 필요했다.

## 결정

**GitHub Actions로 검증 게이트 전체 + 태그 트리거 릴리즈를 자동화한다.** 실물 = `.github/workflows/ci.yml` + `rust-toolchain.toml`.

### 1. 러너 = `windows-latest` 단독

리눅스 매트릭스를 두지 않는다. Job Object 코드가 `#[cfg(windows)]`이고 PTY 픽스처가 `cmd.exe`/`powershell.exe`라 **리눅스에서 이 워크스페이스가 서는지 자체가 미실측**이다. 릴리즈 발행 잡만 예외로 `ubuntu-latest`(쓰기 토큰 쥔 잡의 표면 축소 — 아래 4).

### 2. 범위 = 검증 전용, 릴리즈는 태그에서만

매 push에 도는 것은 CLAUDE.md 「빌드·검증 명령」과 격리 게이트뿐이다. `tauri build`는 태그에서만 돈다 — 무겁고(실측 634s) push마다 필요하지 않다.

### 3. 실 claude 의존 테스트 6건은 CI 레인에서 제외

러너에 claude가 없다. 테스트의 자기 skip 가드(`spawn_json_agent` → `skip_no_claude`)는 **러너에서 작동하지 않는다** — 가드가 ① spawn 호출 성공 ② 5초 내 목록 등장만 보는데, claude 부재 시에도 프로세스가 일단 떠서 목록에 잡혔다가 죽는다. 어느 건이 터지는지가 러너 타이밍에 따라 달라져(실측: 첫 실행 `c2_live_…`, 두 번째 `mcp_send_message_…`) 개별이 아니라 축 단위로 뺐다.

**이 축의 CI 커버리지는 0이고 로컬에서만 검증된다.** 테스트 소스는 고치지 않았다.

### 4. 릴리즈는 빌드/발행 2잡 분리 — 쓰기 토큰 격리

- `release-build` (`contents: read`) — `npm ci`·cargo·`tauri build`. **서드파티 코드가 도는 곳. 토큰 없음.**
- `release-publish` (`contents: write`) — 아티팩트만 받아 발행. 3스텝, npm·cargo·패키징 스크립트 없음.

한 잡에 두면 손상된 빌드 의존성이 `GITHUB_PATH`로 가짜 `gh`를 심어 쓰기 토큰을 탈취하고 성공한 척 끝낼 수 있다(cross-family 리뷰 BLOCK 적출). 두 잡은 다른 러너라 그 경로가 끊긴다.

### 5. 패키징 로직은 재구현하지 않는다

`scripts/build-release.ps1`을 부른다. ADR-0100 co-location 불변식과 "무엇을 배송하는가"의 단일 출처는 그 스크립트의 manifest다. 기대 파일 목록을 워크플로에 적으면 출처가 둘이 되어 반드시 어긋난다.

### 6. 버전은 단언만, 고쳐 쓰지 않는다

태그와 제품 버전 4곳(`tauri.conf.json`·`src-tauri/Cargo.toml`·daemon `Cargo.toml`·`package.json`)+`package-lock.json`을 대조해 다르면 **빌드 전에** 멈춘다. CI가 버전을 자동으로 맞추지 않는다 — 저장소 내용과 배송물의 대응이 깨지면 "이 버전이 어느 커밋이었나"를 못 따라간다.

## 거부한 대안

- **`ubuntu-latest` 매트릭스 추가** — 빠르고 싸지만 리눅스 지원이 미실측이라 첫 실행이 곧 이식 작업이 된다. Windows 전용 코드 경로는 어차피 리눅스에서 검증되지 않는다.
- **self-hosted 러너(개발 PC)** — 무료·빠르고 GUI 실측까지 가능하지만 **public repo에 self-hosted는 보안상 금기**(fork PR이 러너에서 코드 실행). public 무료 러너로 충분하다.
- **`ENGRAM_TEST_REQUIRE_CLAUDE=1` 레인 신설** — 테스트 소스가 이미 제공하는 knob이지만, 러너에 claude가 없는 것이 확실하므로 설정하면 CI가 설계상 빨간불이 된다.
- **claude 의존 테스트에 skip 플래그·`continue-on-error` 주입** — 실패를 감춘다. 이름 제외 + 미커버 명시를 택했다.
- **릴리즈 단일 잡** — 간단하지만 위 4의 토큰 탈취 경로가 열린다.
- **CI가 버전을 자동 bump** — 손이 덜 가지만 저장소 코드와 배포물이 갈린다.
- **`actions/attest-build-provenance` 추가** — "이 zip이 우리 CI에서 나왔나"는 증명하지만 **빌드 의존성이 깨끗했나는 증명하지 않는다**. 트로이 목마가 든 산출물도 유효한 attestation을 받는다. 값이 나오는 배치(빌드 잡)는 위 4가 격리한 바로 그 잡에 `id-token: write`+`attestations: write`를 되돌려 놓는다. 위협이 "릴리즈 페이지 에셋 치환"이라면 발행 잡 배치가 옳고 — 그건 별건이다.
- **branch protection(초록 아니면 머지 금지) 동시 도입** — CI 안정성을 관측하기 전에 걸면 CI 자체를 고칠 때도 PR을 타야 한다. 며칠 관측 후 재론(미결).

## 영향

- **호출자와 정본이 대부분 갈려 있다.** CLAUDE.md·net lib.rs 헤더에 정의된 게이트는 워크플로가 **부르기만** 한다 — 그쪽을 고치면 워크플로도 같이 고쳐야 한다. **예외 2건은 워크플로가 유일 출처다**(ts-rs 바인딩 sync · discovery async 반입 — 로컬 문서에 대응 명령이 없다). 그 둘은 로컬 fallback에서 조용히 빠지므로 `/qa` 바인딩이 명시적으로 포인터를 든다.
- **로컬/CI 분담이 바뀌었다** — 로컬은 quick + GUI 실측, 전체 회귀는 CI. 정본 = `.claude/skill-bindings/qa.md` 「CI와의 분담」.
- **`--locked`가 전 cargo 호출에 붙었다.** 의존성을 바꾸고 `Cargo.lock`을 커밋하지 않으면 CI가 빨간불이다(의도된 마찰).
- **이 워크플로가 막지 못하는 것** — 워크플로 파일 자체에도 적혀 있다. 요지: 잡 분리는 **토큰을 지키지 산출물을 지키지 않는다**. 손상된 빌드 의존성은 여전히 트로이 목마를 zip에 넣을 수 있고 발행 잡은 그걸 성실히 발행한다. 재현 빌드·서명·의존성 감사는 범위 밖이다.
- **미결 3건:** ① 실 claude 의존 6건 처리(그대로 / 가짜 CLI 하네스 / skip을 실패로 승격) ② branch protection ③ GUI 실측의 CI 편입(등록된 시나리오가 없어 자동화 대상이 없다 — 시나리오 작성이 본 작업).

## 근거 — 적대 리뷰가 적출한 것 (요약)

3라운드, doc-aware(주도) + cross-family(비주도) 병렬. 판정 FIX → BLOCK → FIX/BLOCK, 총 15건 반영. 이 결정 문서에 남길 만한 셋:

1. **ts-rs 바인딩 게이트가 vacuous였다** — `cargo test -p engram-dashboard-protocol`은 바인딩을 **재생성만** 하고 `tests/ts_export.rs`에 단언이 0개라 wire 계약↔TS drift가 초록으로 통과했다. `git diff --exit-code`를 뒤에 붙여 닫았다.
2. **비-버전 태그로 공개 릴리즈가 나갈 수 있었다** — `tags: ['v*']` 글롭은 push에만 걸리고 `workflow_dispatch`는 임의 ref를 겨눌 수 있다. job `if`에 이름 가드를 직접 넣었다.
3. **핀해두고 확인은 안 하는 비대칭** — rustc·ripgrep을 특정 버전에 핀했지만 그게 실제로 도착했는지 단언하지 않았다. 핀이 무력화돼도(파일 삭제·`RUSTUP_TOOLCHAIN` 개입·러너에 다른 rg) 초록으로 남는다. 둘 다 런타임 단언을 붙였고, rustc 기대값은 `rust-toolchain.toml`에서 읽어 **두 번째 정본을 만들지 않는다.** (action SHA는 GitHub이 강제하므로 별도 단언이 없다 — 핀 형식만 지키면 된다.)
