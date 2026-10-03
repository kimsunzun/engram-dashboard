# ADR-0267: command 와 messaging 은 base 가 필요해질 때만 의존한다 — 지금은 워크스페이스 의존 0 을 유지한다

- 상태: 확정 (2026-10-03 재작성 · 커밋했으나 push 하지 않은 첫 판 `ebfdafc`(2026-10-02 · 「command 와 messaging 도 base 를 의존한다」 — 의존을 연다 · 아래 「거부한 대안」 첫 항목)을 대신한다, 근거: 사용자 결정 2026-10-03 (「필요할 때만 연결하면 되지」 — `docs/refactoring/architecture-discussion-2026-09-26.md` 결정 후보 2 철회) + 도우미 몫 재측정 2026-10-03 + 로그 현황 실측 master `a226f63`)
- 관련: ADR-0155(결정 2의 command 워크스페이스 의존 0 — 그대로) · ADR-0110(결정 2의 messaging 워크스페이스 무의존과 영향의 import 0 게이트 — 그대로) · ADR-0175(영향의 의존 그래프 — command 와 messaging 은 잎으로 남는다) · ADR-0151(command 의 존재 이유 = 독립적으로 쓸 수 있고 순환을 막는다 — 그대로) · ADR-0268(로그는 각 crate 가 `tracing` 을 직접 부른다) · ADR-0269(base 범용 도우미) · `crates/engram-dashboard-base/src/lib.rs`(입주 조건 · 게이트) · `.github/workflows/ci.yml`(messaging · command 의존 상한 · messaging 격리 정규식) · CLAUDE.md 「빌드·검증 명령」 · step-log S21

## 맥락

`command`(ADR-0155 결정 2)와 `messaging`(ADR-0110 결정 2)은 「워크스페이스 의존 0」을 성질로 갖고, CI 의존 상한 게이트가 그것을 지킨다(기대값 = 자기 자신 1줄 · CLAUDE.md 「빌드·검증 명령」 · `ci.yml`). messaging 은 소스 격리 정규식(`rg "engram_dashboard_(agent|base|daemon|protocol|discovery|command)" crates/engram-dashboard-messaging/src/` → 0줄)도 갖고, 그 금지 목록에 `base` 가 들어 있다.

이 ADR 의 첫 판(2026-10-02 · `ebfdafc`)은 두 crate 가 base 를 의존하도록 그 성질을 열었다. 사유는 로그였다 — 사용자(2026-09-26): 「커맨드·메시지는 로깅 안 함? 당연히 해야지」. 그 사유는 ADR-0268 재작성(2026-10-03 — 로그는 감싸지 않고 각 crate 가 `tracing` 을 직접 부른다)으로 사라졌다. 로그를 **찍는** 데는 base 가 필요 없다 — `messaging` 은 이미 `tracing` 을 직접 의존해 약 34곳에서 찍고(`service.rs` 등), 찍힌 로그는 그 프로세스(데몬)가 base 로 설치한 구독자를 타고 파일에 간다. `command` 는 로그가 0줄이다(`tracing` 의존도 없음 — 실측 master `a226f63`).

그 뒤 사유를 base 의 범용 도우미(ADR-0269)로 바꿔 세웠으나(메인 판단 2026-10-03 · `56cb265`), 두 crate 가 지금 받을 몫은 작다(재측정 2026-10-03):

- **락 오염 복구** — command 의 운영 코드엔 0곳이다. 하네스 모듈 `src/testing.rs`(`test-support` 기능 플래그 뒤)에만 `PoisonError::into_inner` 3곳이 있다. messaging 은 0.
- **XML 이스케이프** — 소비자가 messaging 하나다(`envelope.rs` 의 `escape_xml_attr` · `escape_xml_text`).
- **로그** — base 가 필요 없다(위 · ADR-0268).

사용자(2026-10-03): 「필요할 때만 연결하면 되지」.

## 결정

1. **`command` 와 `messaging` 은 지금 base 를 의존하지 않는다 — 워크스페이스 의존 0 을 유지한다**(사용자 2026-10-03). 두 crate 의 의존 상한 게이트(기대값 = 자기 자신 1줄)와 messaging 소스 격리 정규식(금지 목록에 `base` 포함)은 그대로다(CLAUDE.md 「빌드·검증 명령」 · `ci.yml`).
2. **base 도우미가 실제로 필요해지면 그때 연결한다**(사용자 2026-10-03). 연결하는 변경이 그 필요(어느 도우미를 어디서 쓰는가)를 근거로 적고, 이 ADR 을 개정한다. 그때 같은 변경에서 바꿀 게이트는 아래 「영향」에 있다.
3. **로그는 이 결정과 무관하다** — `command` 가 로그를 들이면 `tracing` 만 직접 의존한다(ADR-0268). 워크스페이스 의존은 늘지 않는다.

## 거부한 대안

- **지금 base 의존을 연다(두 crate 의 의존 상한을 「자기 자신 + base」로).** ★이 ADR 의 첫 판이다★(2026-10-02 작성 · 메모 결정 후보 2 · push 하지 않았다 — 이 재작성이 그 판을 대신한다). 기각 = 사용자 결정(2026-10-03 — 「필요할 때만 연결하면 되지」). 사유:
  - **첫 판의 사유(로그)는 사라졌다** — 로그는 base 없이 `tracing` 으로 찍힌다(ADR-0268 재작성).
  - **바꿔 세운 사유(범용 도우미)는 지금 받을 몫이 작다** — command 운영 코드의 락 오염 복구 0곳(하네스 3곳뿐) · messaging 0곳 · XML 이스케이프 소비자 하나(위 「맥락」 · 재측정 2026-10-03).

## 근거

- **사용자 결정 2026-10-03** — 위 인용.
- **재측정 2026-10-03** — 위 「맥락」의 세 항목(락 오염 복구 · XML 이스케이프 · 로그). 로그 현황(messaging 약 34곳 · command 0줄)은 master `a226f63` 실측이다.
- **필요해졌을 때 연결하는 모양 자체는 문제가 없다** — 결정의 근거가 아니라, 나중에 연결을 막을 까닭이 없다는 맥락이다.
  - 공용 기반 crate 를 대부분의 crate 가 의존하는 것은 큰 워크스페이스의 흔한 형이다 — rust-analyzer `hir` · `vfs` → `stdx`, zed `editor` · `rope` → `util`, Materialize `adapter` · `persist` → `mz-ore`, TiKV `raftstore` → `tikv_util`, Firecracker `vmm` → `utils`, nushell `nu-protocol` → `nu-utils`(각 `Cargo.toml` 확인 2026-10-03).
  - base 는 도메인 지식 0 · 워크스페이스 의존 0 을 자기 게이트(의존 상한 · `use tauri` 0줄 · 입주자 상호 무참조)가 지킨다(`crates/engram-dashboard-base/src/lib.rs` 헤더 · CLAUDE.md 「빌드·검증 명령」). 그래서 두 crate 가 base 에 기대도 base 를 거쳐 다른 워크스페이스 crate 가 딸려 오지 않는다.

## 영향 / 불변식

- **불변식(그대로): `command` · `messaging` 의 워크스페이스 의존 = 0.** 게이트를 바꾸지 않는다 — 의존 상한 둘(→ 정확히 1줄 = 자기 자신)과 messaging 소스 격리 정규식(→ 0줄)이 지킨다(`ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`).
- **ADR-0155 · ADR-0110 · ADR-0175 · ADR-0151 을 개정하지 않는다** — 첫 판이 그 넷에 박았던 부분 폐기 도장과 개정 링크는 걷었다(2026-10-03). command 와 messaging 은 ADR-0175 영향의 의존 그래프에서 잎으로 남는다.
- **연결할 때 같은 변경에서 바꿀 것**(첫 판이 적어 둔 목록 — 그때 이 ADR 을 개정한다):
  - 의존 상한 둘의 기대값 → 정확히 2줄(자기 자신 + base). 자리 = `ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`.
  - ★**messaging 소스 격리 정규식의 금지 목록에서 `base` 를 뺀다**★ — 남겨 두면 base 를 쓰기 시작하는 순간 빨개진다(`ci.yml` · CLAUDE.md · `qa.md` 세 자리).
  - CLAUDE.md 「백엔드 모듈 맵」 command · messaging 항목(「워크스페이스 crate 의존 0」 · 「워크스페이스 crate 무의존(컴파일러 강제 벽)」).
- **ADR-0269 의 도우미 흡수에서 두 crate 몫은 빠진다** — messaging 의 XML 이스케이프와 command 하네스의 락 오염 복구 3곳은 작업 순서 1-1 에서 옮기지 않고 제자리에 남는다(ADR-0269 영향).
- **코드 앵커는 두지 않는다** — 바뀌는 코드가 없다.
