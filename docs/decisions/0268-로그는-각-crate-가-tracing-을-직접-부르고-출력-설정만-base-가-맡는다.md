# ADR-0268: 로그는 각 crate 가 tracing 을 직접 부르고 출력 설정만 base 가 맡는다

- 상태: 확정 (2026-10-03, 근거: 사용자 결정 2026-10-03 (「로그 감싸지 마」 — `docs/refactoring/architecture-discussion-2026-09-26.md` 결정 후보 3 번복) + 사용자 결정 2026-10-02 (가리기는 찍는 곳에) + 조사 `docs/research/low-level-crate-logging-2026-10-02.md` + 로그 현황 실측 master `a226f63` · 스팬 실측 `d5ac725`)
- 관련: ADR-0138(릴리스 로그 파일 동기 쓰기 — 「마스킹 · 레닥션 레이어」 기각은 그대로이고 결정 4 가 기대는 근거의 하나다) · ADR-0175 결정 1(logging 은 base) · ADR-0266(platform crate — `tracing` 을 직접 부른다) · ADR-0267(command · messaging 도 base 의존) · ADR-0269 결정 4(락 오염 복구의 경고 로그) · `crates/engram-dashboard-base/src/logging/mod.rs`(구독자 설치 · `EnvFilter` · `mask_secrets`) · `docs/reference/logging-conventions.md`(「보안」) · `docs/tracking-archive.md` T-1(가리기는 sink 에 배선하지 않고 호출자가 명시 호출) · 큰 절 A(로깅 시스템 설계 — 메모 「큰 절」) · 조사 `docs/research/low-level-crate-logging-2026-10-02.md` · step-log S21

## 맥락

`tracing` 을 직접 의존하는 crate 가 base 말고 여섯이다 — agent · daemon · discovery · messaging · net · 셸. 로그 매크로 호출은 약 530곳이다(daemon 193 · agent 129 · 셸 125 · net 38 · messaging 34 · discovery 9 — `tracing::` 경로 포함 대략치, 실측 master `a226f63`). `command` · `protocol` · `transport` 는 0.

중앙에서 쥔 것은 **출력 설정**이다 — 구독자 설치 · 레벨 필터 · 파일 · 형식 · 실행 중 레벨 변경을 base 의 `logging` 모듈(`crates/engram-dashboard-base/src/logging/`) 한 곳이 정한다.

비밀값 가리기는 중앙에서 걸리지 않는다 — 파일 writer 는 받은 바이트를 그대로 쓰고(`logging/mod.rs` 의 `FileSinkWriter::write`), 가리기는 찍는 쪽이 `mask_secrets` 를 골라 부른다. 그 배치는 이미 정해져 있다 — T-1 이 「sink 에 배선하지 않고 호출자가 명시 호출」로 구현했고, ADR-0138 이 「로그용 마스킹 · 레닥션 레이어 추가」를 기각했다(토큰이 이미 같은 폴더에 평문으로 있다 · 대신 「우리가 쓰는 문장에는 자격증명을 넣지 않는다」).

사용자(2026-09-26): 「로그 라이브러리를 왜 직접 쓰냐고. 나중에 컨트롤이 안 되잖아」. 이 우려에서 이 ADR 의 첫 판(2026-10-02 — base 입구로 감싼다 · 아래 「거부한 대안」 첫 항목)이 나왔고, push 전에 사용자가 뒤집었다(2026-10-03 — 「로그 감싸지 마」).

## 결정

1. **로그를 감싸지 않는다 — 각 crate 는 `tracing` facade 를 직접 부른다**(사용자 2026-10-03 — 「로그 감싸지 마」). 지금 코드의 모양 그대로다.
2. **base 는 출력 설정만 맡는다**(사용자 2026-10-03) — 구독자 설치 · 레벨 필터 · 파일 · 형식 · 실행 중 레벨 변경(`crates/engram-dashboard-base/src/logging/`).
3. **그룹별 켜고 끄기는 감싸지 않고 `tracing` 의 target 으로 한다**(사용자 질문 2026-10-03 — 모아 두면 그룹별로 켜고 끌 수 있나 — 에 대한 메인 답). 호출 자리의 모듈 경로가 기본 target 이고 base 가 설치한 `EnvFilter`(`crates/engram-dashboard-base/src/logging/mod.rs` — `RUST_LOG` 읽기 `:401` · 실행 중 레벨 변경 `:436`)가 그것으로 거른다. 모듈 경계와 다른 묶음(예: 우편)이 필요하면 호출에 `target: "<묶음>"` 을 붙인다. 카테고리 어휘는 큰 절 A(로깅 시스템 설계)가 정한다.
   - 근거(코드) — `tracing` 은 target · module_path · file · line 을 **매크로를 부른 자리마다 고정된 정적 메타데이터**(callsite)로 둔다. 호출부가 매크로를 직접 부르면 그 자리의 모듈이 그대로 target 이 된다.
4. **비밀값 가리기는 실제로 찍는 곳에 둔다**(사용자 2026-10-02 — 「base는 그걸 모르고 실제 찍는 곳에 걸어야 되는 거 아님?」 · 「매번 파싱해서 체크할 수는 없잖아 내부에서」). base 의 writer 는 메시지를 파싱하지 않는다. base 의 `mask_secrets` 는 부르는 쪽이 고르는 범용 도구로 남는다 — 그 패턴은 업체별 키 형식 일반(`sk-` · `AKIA` · `ghp_` 등 · Bearer 토큰)이라 도메인 지식이 없다(`logging/mod.rs` 의 `mask_secrets`).
5. **「`tracing` 직접 의존은 base 만」 게이트를 두지 않는다**(사용자 2026-10-03 — 결정 1 의 귀결).

## 거부한 대안

- **base 가 로그 입구를 자기 경로로 제공하고 모든 로그를 그리로 거친다(위험도 · 카테고리 · 메시지 세 인자의 매크로 · `tracing` 직접 의존은 base 만 — 게이트로 막음).** ★이 ADR 의 첫 판이다★(2026-10-02 작성 · 메모 결정 후보 3 · push 하지 않았다 — 이 재작성이 그 판을 대신한다). 기각 = 사용자 결정(2026-10-03 — 「로그 감싸지 마」). 사유:
  - **피어 관행** — 낮은 crate 가 `log` / `tracing` facade 를 직접 부른다: wezterm `portable-pty` · rust-analyzer `stdx` · `cargo-util` · zed `util` · ripgrep · alacritty · nushell. 호출부를 공용 util crate 로 거치게 하는 곳은 표본에 없었다(조사 §1 — 확실).
  - **공식 지침** — `log` · `tracing` 문서 둘 다 「라이브러리는 facade 만 링크하고, 실행 파일이 백엔드를 설치한다」(조사 §4 — 원문 대조).
  - **2026-09-26 의 통제 우려는 셋으로 갈리고 셋 다 이미 덮인다** — ① 출력 통제는 base 한 곳에 이미 있다(결정 2) ② 가리기는 찍는 곳으로 정해졌다(결정 4) ③ 백엔드를 바꾸는 것은 base 만 고친다 — `tracing` 자체가 facade 라서다.
  - **감싸면 platform 이 로그 하나 때문에 base 를 의존해야 한다** — 사용자(2026-10-03): 「로그 때문에 Base 전체를 그렇게 포함시키는게 말이 안 됨」. platform 은 워크스페이스 의존 0 이다(ADR-0266).
  - **결정 3 을 넘어 감싸기가 더하는 것은 모든 호출에 카테고리 인자를 강제하는 것 하나다** — 그 묶음은 필요한 자리에 `target:` 을 붙이는 관례가 덮는다(결정 3).
- **base writer 단계에서 모든 줄에 가리기를 자동으로 건다(선택 → 기본 — 메모 결정 후보 3 의 옛 메인 제안).** 기각 = 사용자 결정 둘(2026-10-02) — 「base는 그걸 모르고 실제 찍는 곳에 걸어야 되는 거 아님?」(무엇이 비밀인지는 찍는 쪽이 안다) · 「매번 파싱해서 체크할 수는 없잖아 내부에서」(모든 줄에 정규식을 거는 비용 — 재지는 않았다). 그리고 ADR-0138 의 기각 사유가 그대로 선다 — 토큰은 이미 같은 폴더에 평문으로 있어 로그 사본을 가리는 것은 같은 신뢰 경계 안의 중복 방어다(논리).

## 근거

- **사용자 결정 2026-10-03 · 2026-10-02** — 위 인용.
- **조사(2026-10-02 · `docs/research/low-level-crate-logging-2026-10-02.md`)** — §1 피어 관행 · §4 공식 facade 지침(둘 다 확실 등급).
- **로그 현황 실측(master `a226f63`)** — 「맥락」의 crate 수 · 호출 수(대략치).
- **writer 실측** — 파일 writer 가 받은 바이트를 가공 없이 쓴다(`FileSinkWriter::write`).
- **가리기 배치의 선례** — T-1(`docs/tracking-archive.md` — 「sink 에 배선하지 않고 호출자가 명시 호출」) · ADR-0138 「거부한 대안」 · `docs/reference/logging-conventions.md` 「보안」(「`mask_secrets` 헬퍼는 자동 적용이 아니라 호출자가 명시 호출한다」).
- **스팬 실측(2026-10-02 · `d5ac725`)** — `rg "_span!|#\[instrument|tracing::instrument|\.instrument\(|span!\(" crates src-tauri/src` → 0줄. 스팬을 만드는 곳이 없다. `tracing::span` 이 걸리는 자리는 전부 테스트 쪽 로그 관측 구독자 구현이다(아래 영향).

## 영향 / 불변식

- **전환 작업이 없다** — 첫 판이 잡았던 약 530곳의 base 입구 전환(사이드 작업)과 마지막 게이트는 사라진다.
- **남는 비용: facade(`tracing`) 자체를 다른 것으로 바꾸는 일은 여전히 호출부 전부(약 530곳)를 고친다**(메인 서술). 「거부한 대안」 첫 항목의 ③이 덮는 것은 백엔드(구독자 · 형식 · 파일) 교체까지다.
- **`tracing` 직접 의존은 지금 그대로다** — base · agent · daemon · discovery · messaging · net(선택 기능 뒤 — `optional = true`) · 셸. `command` 가 로그를 들이면(ADR-0267) 같은 꼴로 `tracing` 을 직접 의존한다.
- **platform crate(ADR-0266)도 `tracing` 을 직접 부른다** — `RmSession` 의 `Drop` 경고(`crates/engram-dashboard-agent/src/platform/file_holders.rs` 의 `impl Drop for RmSession`)는 `tracing::warn!` 그대로 옮겨 간다. platform → base 간선은 없다.
- **base 의 락 오염 복구 경고(ADR-0269 결정 4)도 `tracing::warn!` 을 직접 부른다** — base 는 이미 `tracing` 을 의존하므로 `sync` 가 base 의 `logging` 모듈을 부르지 않는다. 입주자끼리 무참조(입주 조건 ③)가 그대로 선다.
- **테스트 쪽 로그 관측은 그대로다** — 테스트가 로그를 잡으려고 `tracing` 구독자를 직접 구현한 곳(데몬 `src/log_capture.rs` — 그 머리 주석이 남은 사본 둘 `command_roster` · `control::mod` 을 함께 센다 · agent `backend/codex/transport.rs` · `backend/claude/leftover.rs` · 셸 `daemon_client/usage_interest.rs` · `settings/tests.rs` — `impl tracing::subscriber::Subscriber for` · `with_default`, `d5ac725`)을 옮길 이유가 없다.
- **ADR-0138 은 고치지 않는다** — 동기 writer · 폴더는 호출자가 넘긴다 · `LogKind` 닫힌 집합 · 마스킹 레이어 기각 전부 그대로다.
- **`docs/reference/logging-conventions.md` 의 `tracing::` 예와 필드 규약은 이 결정의 모양 그대로다** — 옮길 것이 없다. `target:` 묶음을 붙이는 실무 규칙(결정 3)은 그 문서가 진다.
- **데몬의 미사용 `tracing-subscriber` 선언(CLAUDE.md 「의존성」)은 전환과 묶이지 않는다** — 메모 §11 의 항목으로 남아 4단계(데몬 정리)에서 본다.
- **열린 것 — 큰 절 A(로깅 시스템 설계):** 카테고리 · 위험도 체계와 그 어휘(결정 3 의 `target:` 묶음 이름) · 실행 중 레벨을 바꾸는 입구.
- **코드 앵커 = `// ADR-0268`** — base `logging` 모듈 머리(출력 설정을 쥐는 자리).
