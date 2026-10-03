# ADR-0267: command 와 messaging 도 base 를 의존한다

- 상태: 확정 (2026-10-02, 근거: 사용자 결정 2026-09-26 (`docs/refactoring/architecture-discussion-2026-09-26.md` 결정 후보 2) + 로그 현황 실측 master `a226f63` + 사유를 로그에서 base 범용 도우미로 바꿔 세운 것은 메인 판단 2026-10-03 — 사용자는 피어 증거를 함께 보고 base 를 유지했으나 이 ADR 을 따로 판정하지는 않았다)
- 관련: Amends ADR-0155 (결정 2의 워크스페이스 의존 0) · Amends ADR-0110 (결정 2의 워크스페이스 무의존과 영향의 import 0 게이트) · Amends ADR-0175 (영향의 의존 그래프 중 command 와 messaging 이 잎이라는 줄) · ADR-0151(command 의 존재 이유 = 독립적으로 쓸 수 있고 순환을 막는다 — 그대로) · ADR-0268(로그는 각 crate 가 `tracing` 을 직접 부른다 — 2026-10-03 재작성) · ADR-0269(base 범용 도우미) · `crates/engram-dashboard-base/src/lib.rs`(입주 조건 · 게이트) · `.github/workflows/ci.yml`(messaging · command 의존 상한 · messaging 격리 정규식) · step-log S21

## 맥락

`command`(ADR-0155 결정 2)와 `messaging`(ADR-0110 결정 2)은 「워크스페이스 의존 0」을 성질로 갖고, CI 의존 상한 게이트가 그것을 지킨다(기대값 = 자기 자신 1줄). base 는 설계상 어느 crate 든 쓸 수 있는 잎인데, 이 두 crate 만 자기 게이트에 막혀 base 를 못 쓴다.

로그 현황(실측 master `a226f63`): `messaging` 은 이미 로그를 남긴다 — `tracing` 을 직접 의존해 약 34곳(`service.rs` 등). 로그를 **찍는** 데는 base 가 필요 없고, 찍힌 로그는 그 프로세스(데몬)가 base 로 설치한 구독자를 타고 파일에 간다. `command` 는 로그가 0줄이다(`tracing` 의존도 없음).

사용자(2026-09-26): 「커맨드·메시지는 로깅 안 함? 당연히 해야지」.

★**그래서 base 의존의 사유는 로그가 아니다**★(메인 판단 2026-10-03). 로그는 각 crate 가 `tracing` 을 직접 부른다(ADR-0268 재작성). 두 crate 가 base 를 쓸 몫은 base 의 범용 도우미다(ADR-0269) — messaging 의 XML 이스케이프(`envelope.rs` 의 `escape_xml_attr` · `escape_xml_text` → base `text`), 락 오염 복구(→ base `sync` — command 는 운영 코드엔 없고 하네스 모듈 `testing.rs`(`test-support` 플래그 뒤)에만 `PoisonError::into_inner` 3곳이 있다 · messaging 은 0 — 재측정 2026-10-03), 시간(→ base `time`) 등.

## 결정

1. **`command` 와 `messaging` 은 base 를 의존할 수 있다**(사용자 2026-09-26). 두 crate 의 워크스페이스 의존 상한은 「자기 자신」에서 「자기 자신 + base」로 바뀐다. base 밖의 워크스페이스 crate 는 여전히 0 이다.
2. **얻는 것 = base 의 범용 도우미**(결정 후보 4 · ADR-0269 — 메인 판단 2026-10-03 으로 이것이 이 결정의 사유다) — messaging 의 XML 이스케이프 · 락 오염 복구 `sync` · 시간 `time` 등을 두 crate 도 쓴다. ★로그는 base 를 거치지 않는다★ — 결정 후보 3 이 번복돼(ADR-0268 재작성, 사용자 2026-10-03 「로그 감싸지 마」) 로그는 각 crate 가 `tracing` 을 직접 부른다. `command` 의 로그 추가는 리팩터링 때 하고, 그때 `command` 가 `tracing` 을 직접 의존한다.

## 거부한 대안

- **「워크스페이스 의존 0」을 그대로 지킨다(ADR-0155 결정 2 · ADR-0110 결정 2).** 기각 = 사용자 결정(「당연히 해야지」). 그 성질 아래서는 두 crate 가 범용 도우미를 못 쓴다. ★옛 사유의 앞절 「로그를 base 경유로 찍을 수 없다(결정 후보 3 과 충돌)」은 ADR-0268 재작성(2026-10-03)으로 힘을 잃었다★ — 로그는 base 없이 `tracing` 으로 찍힌다(위 「맥락」이 이미 적은 대로다).

## 근거

- **사용자 결정 2026-09-26** — 위 인용.
- **안전성 — 순환·도메인 유입 경로가 생기지 않는다.** base 는 도메인 지식 0 · 워크스페이스 의존 0 을 자기 게이트(의존 상한 · `use tauri` 0줄 · 입주자 상호 무참조)가 지킨다(`crates/engram-dashboard-base/src/lib.rs` 헤더 · CLAUDE.md 「빌드·검증 명령」). 그래서 두 crate 가 base 에 기대도 base 를 거쳐 다른 워크스페이스 crate 가 딸려 오지 않는다.
- **사유 = base 범용 도우미**(메인 판단 2026-10-03) — 위 「맥락」 끝 문단 · ADR-0269. 로그는 사유가 아니다(ADR-0268).
- **공용 기반 crate 를 대부분의 crate 가 의존하는 것은 큰 워크스페이스의 흔한 형이다** — rust-analyzer `hir` · `vfs` → `stdx`, zed `editor` · `rope` → `util`, Materialize `adapter` · `persist` → `mz-ore`, TiKV `raftstore` → `tikv_util`, Firecracker `vmm` → `utils`, nushell `nu-protocol` → `nu-utils`(각 `Cargo.toml` 확인 2026-10-03).
- **로그 현황 실측** — 위 「맥락」(messaging 약 34곳 · command 0줄). 결정을 받치는 근거가 아니라 「로그 때문에 base 가 필요하다」가 아님을 보이는 사실이다.

## 영향 / 불변식

- **불변식(바뀐 꼴): `command` · `messaging` 의 워크스페이스 의존 = base 하나뿐.** ADR-0110 이 말한 「완전 상호무지」의 대상(agent · daemon · protocol · discovery 등 도메인 crate)은 그대로 0 이다.
- **바꿀 게이트(같은 변경에서):**
  - 의존 상한 둘의 기대값 — `cargo tree -p engram-dashboard-messaging …` · `cargo tree -p engram-dashboard-command …` → 정확히 2줄(자기 자신 + base). 자리 = `ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`.
  - ★**messaging 소스 격리 정규식이 `base` 를 금지 목록에 갖고 있다**★ — `rg "engram_dashboard_(agent|base|daemon|protocol|discovery|command)" crates/engram-dashboard-messaging/src/`(→ 0줄 · `ci.yml` · CLAUDE.md · `qa.md` 세 자리). base 를 쓰기 시작하면 이 게이트가 빨개진다. 메모(결정 후보 2)는 의존 상한 게이트만 적었다 — 이 정규식에서 `base` 를 빼는 것도 같은 변경이다.
- **ADR-0175 영향의 의존 그래프(「`base` · `command` · `messaging` = 잎」)에서 잎은 base 하나가 된다.**
- **ADR-0151 의 정당화는 그대로다** — command 의 존재 이유는 「독립적으로 쓸 수 있고 순환을 막는다」이고, base 는 그 두 성질을 깨지 않는다(위 안전성). 단 그 ADR 근거의 한 문장(「아무것도 의존하지 않는 `command`는 … 공통 어휘 crate 자리에 앉는다」)은 사실로서 낡는다.
- **CLAUDE.md 「백엔드 모듈 맵」 command · messaging 항목을 고친다** — 「워크스페이스 crate 의존 0」 · 「워크스페이스 crate 무의존(컴파일러 강제 벽)」.
- **코드 앵커 = `// ADR-0267`** — 두 crate 의 `Cargo.toml` base 의존 줄.
