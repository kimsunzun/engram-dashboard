# 핸드오프: 주석 retrofit — protocol 완료 · tray/daemon_client 재스윕 완료 · 다음=절 단위 축(메인 직접 편집으로 전환)

## 한 줄 상태 · 다음 첫 액션

dashboard 전량 주석 retrofit 중. **커밋 4개 완료**(protocol 전체 + src-tauri의 tray·daemon_client, `tests.rs` 제외). 기준이 세 번 개정되며 수확이 계단식으로 올랐다(삭제 0 → 2 → 13, stale 정정 0 → 4 → 12).

**다음 첫 액션 = 절 단위(clause-level) 축 파일럿.** 대상 = `src-tauri/src/daemon_client/mod.rs` + `lifecycle.rs`. **워커에 위임하지 말고 메인이 직접 편집**(사용자 결정 — 아래 §운영 방식 전환). 끝나면 `/review doc full` → `/qa standard` → 커밋 → 즉시 핸드오프.

---

## 운영 방식 전환 (사용자 결정 2026-07-31 — 이전 세션과 다름)

1. **편집 = 메인 직접.** 절 단위 정리는 "이 문장이 이력 잔여물인가 load-bearing WHY인가"를 문장마다 판정하는 일이라 지시서로 분리되지 않는다(global-rules의 「의도 손실」 예외에 해당). 워커에 넘기면 맥락을 매번 재발견해야 하고, 이번 세션 결함 4건이 전부 그 지점에서 나왔다.
2. **컨텍스트는 잦은 핸드오프로 해결.** 실측 근거: **워커 고정 오버헤드 ≈ 68k** vs **세션 시작 오버헤드 ≈ 25~30k** → 핸드오프가 워커 스폰보다 싼 컨텍스트 리셋이다. 파일·배치 경계마다 끊고 넘긴다(절반 기다리지 않는다).
3. **리뷰어·QA 게이트는 유지·강화.** 메인이 저자 겸 오케스트레이터가 되므로 제3자 눈이 더 중요하다. `/review doc full`(cut-advocate blind = codex + load-bearing 수호 doc-aware) 유지.
4. **워커는 대량 기계 작업만** — 전 파일 심볼 스캔, 다수 파일 삭제 패스 등.
5. **워커를 쓸 때는 전량 Opus(`worker-senior`) 고정** — 사용자 결정. 전역 사전의 『코더(단순)』=경량 바인딩을 이 작업에서 덮는다. 근거 = 경량 티어가 재작성에서 창/뷰 어휘 혼동·거짓 단정·의도 날조를 만들었고, 삭제 판정은 16건 전건 통과했다(삭제는 티어 내성 있음, 재작성은 없음).

---

## 완료 범위 (재스윕 불필요)

- `crates/engram-dashboard-protocol/src`, `.../tests` — 커밋 `b297848`(거짓 정정 4건) + `6f783c6`(parrot 13건) + `4cf524d`(rustfmt 보정)
- `src-tauri/src/tray` (3파일), `src-tauri/src/daemon_client` (**`tests.rs` 제외** 5파일) — 커밋 `5648497`(stale 12건 + 삭제 3건)
- `crates/engram-dashboard-protocol/bindings` = ts-rs 생성물 → 스윕 대상 아님(주석 삭제 시 `cargo test -p engram-dashboard-protocol`이 자동 재생성하므로 커밋에 포함할 것)

**`retrofit-완료`는 의도적으로 미기록**(사용자 결정). 바인딩이 *선언*인데 진도 상태를 섞는 문제라 자리 결정을 스킬 팩토리에 넘겼다. 따라서 rules.md의 **「밀도 정합」은 전 구역 비활성** — 스윕 시 허용/금지 목록만 적용한다.

---

## 확정 기준 (다음 세션이 되돌리지 말 것)

- **parrot 확정 = 식별자 번역 doc.** "공개 API rustdoc이라서"는 보존 사유가 **아니다** — 계약 정보(단위·경계값·소유권·에러 조건·대응 이벤트·형제 구분) 한 조각도 없으면 소음. (style-base에 팩토리가 이미 명확화 반영)
- **죽은 심볼 스캔 의무.** 주석이 이름 붙인 심볼·명령·타입·이벤트명을 `rg`로 실재 확인한다. 눈에 띌 때까지 기다리지 않는다. 이번 세션 최대 수확이 이 유형이었다.
- **경계 관례 3부류 = 보존 확정**(사용자 결정, 재논쟁 종료): 리뷰 추적 태그(`(FIX 3)`·`M1`·`(phase4 1단계)`) · `PROTOCOL_VERSION` v2/v3 breaking 근거 · 테스트 구획 라벨·바이트 폭 tally. **blind 리뷰어가 매번 다시 삭제 제안하는데, 사용자에게 에스컬레이션하지 말고 SEALED 기계 적용으로 기각한다**(사용자 지적: 애매한 건 보존이 이미 규칙이다).
- **의도적 부재의 이유 = 고가치.** `lifecycle.rs`의 loom 유보 블록처럼 "왜 이 seam·기능이 없나"는 보존(style-base 명문).
- **재작성 = 실코드 그라운딩만.** 주석-대-주석 대조 금지(guardian도 한 번 이 실수를 했다 — `connection_core.rs` 주석을 근거로 stale 판정). 확신 없으면 `stale 후보`로 강등해 보고.

---

## 다음 작업의 실체 — 절 단위 축 (왜 필요한가)

**부피의 진원지 = 긴 주석 블록.** 실측: repo 전체 5줄 이상 주석 블록 **1,317개 / 13,390줄**(전체 ~103k줄의 13%). 스윕 완료 파일에서도 거의 그대로 남았다 — `connection.rs` 24블록/209줄, `mod.rs` 20블록/207줄, `lifecycle.rs` 10블록/134줄.

**그러나 압축은 답이 아니다.** `mod.rs:1-32`(최대 블록)를 뜯어보면 16~32행이 이 코드베이스 최고가치 주석이다(단일 task가 stream 단독 소유하는 이유 · atomic으로는 TOCTOU를 못 묶어 `Mutex<Lifecycle>`로 원자화한 경위 · Codex 적출 이력). 압축하면 손실이다.

**진짜 갭:** 현재 워커·스윕은 블록을 **통째로 남길까/지울까**로만 판정한다. 그래서 **남겨야 할 블록 안에 든 이력·로드맵·거짓 절**은 손대지 않는다. 실물 = `mod.rs:29-31`의 `"T2 는 씨앗까지만 … 완전한 동시-시도 abort·백오프 재연결은 T4"` — **백오프 재연결은 이미 구현됐다**(`connection.rs:633` 지수 백오프, `:63` MAX 5회 — guardian 검증). 고가치 블록 속의 거짓이다.

즉 새 기준이 아니라 **기존 금지 목록(journal/changelog)을 절 단위로 적용**하는 축이다. 대상 유형: 완료된 마일스톤 서술(T2/T3/T7c 등) · ADR 본문 재서술(ADR이 정본, 코드엔 앵커 한 줄이 프로젝트 규약) · 미래형으로 남은 완료 사항.

---

## 검증 상태

**돌린 것 (전부 PASS):**
- `/review doc full` 2라운드 — cut-advocate(codex, blind) + load-bearing 수호(doc-aware). 두 배치 모두 FIX → 전건 정정 → 재검증. 마지막 판정 = FIX(low, 비차단 — 아래 잔여)
- `/review doc light` 1회(parrot 패스) — PASS, 13건 전건 안전 판정
- `/qa standard` — 빌드 34s · 5멤버 19스위트 0 fail · `cargo fmt --check` · 코어 격리 0줄 · tsc rc=0 · vitest 634/634

**재실행 명령:**
```bash
cargo build
cargo test -p engram-dashboard-core -p engram-dashboard-protocol -p engram-dashboard-discovery -p engram-dashboard-messaging -p engram-dashboard-daemon
cargo fmt --check
rg "^\s*use tauri" crates/engram-dashboard-core/src/   # 0줄 = PASS
npx tsc --noEmit && npm test
```

**검증 안 된 것:**
- **GUI 실측 0회** — 주석 변경이라 동작 영향이 없다고 판단해 qa full로 올리지 않았다. 프론트 파일을 만지는 배치(B5~B7)는 qa가 full로 자동 승격되므로 cdp 실측이 필요하다.
- **`daemon_client/tests.rs`(3,153줄) 미스윕** — 워커 상한 초과로 이번 배치에서 제외. 이 파일 때문에 `daemon_client` 디렉터리는 부분 커버다.
- **구획 분담(같은 파일을 줄 범위로 쪼개 순차 워커) 경로 0회 실측** — 스킬에 신설됐으나 아직 안 써봤다. `discovery/src/lib.rs`(2,362줄, 절단선 = `#[cfg(test)]` 1177행)가 첫 대상 예정이었다.

---

## 실패한 접근 (do-not)

1. **qa를 좁혀 돌리며 `cargo fmt --check` 생략 → 드리프트가 커밋됐다**(`6f783c6`, `4cf524d`로 보정). **주석 삭제는 포맷 중립이 아니다** — doc 주석을 지우면 rustfmt가 struct variant의 한 줄/여러 줄 판정을 바꾼다. 주석만 만지는 스윕에서도 fmt 검사는 필수.
2. **경량(Sonnet) 워커에 재작성 11건을 몰아주기** → 원본에 없던 오류 3부류 생성(창/뷰 어휘 혼동 · "반환값 쓰는 호출자 없음" 거짓 · "향후 재사용을 위해 남겨둠" 의도 날조). 재작성은 상급 티어 + 전건 검사.
3. **blind 리뷰어 제안을 그대로 수용하지 말 것** — blind는 프로젝트 규약·사용자 결정을 모른다(설계상). 경계 관례 3부류 삭제를 매번 다시 제안한다.
4. **`git checkout -- <file>` 은 자동 모드 분류기에 막힌다.** 원복이 필요하면 사용자에게 권한을 요청하거나 다른 경로를 쓸 것.
5. **메인이 "군더더기가 거의 없다"로 브리핑한 것은 오류였다** — 실상은 정책·오적용으로 안 지운 것이었고 사용자 이의제기로 발각됐다. 수확 보고 시 "규칙상 대상이 아니어서 남긴 것"과 "정말 좋은 주석"을 구분해 말할 것.

---

## 정지 조건 (멈추고 사용자에게 물을 것)

- **`crates/engram-dashboard-messaging/src/service.rs`(11,149줄 = impl 4,182 + test 6,967)** — 구획 분담으로도 상한을 넘는다. 임의 부분 스윕 금지, 에스컬레이션.
- **`retrofit-완료` 기록** — 현재 방침은 미기록. 기록하려면 사용자 승인.
- **스킬 파일(`~/.claude/skills/clean-comment/`) 직접 수정 금지** — 결함은 `feedback.md`에만 누적, 반영은 스킬 팩토리(`I:\Engram\agents\skill-factory`) 소관. 이번 세션에 팩토리가 이미 3건 반영했다.
- **코드 토큰 변경이 필요한 발견** — 주석 스윕 범위 밖. 별도 과업으로 넘긴다(아래 잔여 참조).

---

## 잔여 (비차단 — 다음 세션 처리 대상)

**주석 축:**
- `daemon_client/tests.rs` 3,153줄 미스윕
- `mod.rs` — T2/T4 마일스톤 어휘 2곳이 용어집 삭제 후 고아 상태(`connection.rs:1`도 "S14 모듈① T2" 유지). 절 단위 축 첫 대상.
- `mod.rs:29-31` — 백오프 재연결이 이미 구현됐다는 사실로 문구를 좁힐 것. "두 소켓 동시 open" 절반은 미검증이라 그대로 둘 것.
- `protocol_state.rs:6` — "창 N개로는 깨끗한 청크만 fan-out"이 이제 epoch-only 의미(모호, 거짓 아님)

**코드 변경 과업(주석 스윕 밖):**
- `protocol_state.rs:371` assert 메시지 `"epoch 11→10 변경 — 창 render_seq 리셋 필요"` → **방향 역전**(테스트는 10→11) + `render_seq`는 ADR-0046으로 존재하지 않음. 제안 문안 `"epoch 10→11 변경 = changed"`. 같은 파일 `:364` `"같은 epoch 재확인 — 창 리셋 불필요"`도 동종. guardian은 두 건을 한 편집으로 하라고 권고.
- `crates/engram-dashboard-daemon/src/connection_core.rs:891,893,919` — 죽은 `EmbeddedClient`/`spawn_agent` 미러 참조. 이번 스윕 범위 밖(daemon crate)이었다.

**미스윕 구역(실측, bindings 제외):**

| 구역 | 파일 | 줄 | 비고 |
|---|---|---|---|
| discovery | 2 | 2,629 | `lib.rs` 2,362 = 구획 분담 첫 실측 대상(절단선 1177행) |
| messaging | 7 | 18,536 | `service.rs` 11,149 = 에스컬레이션 |
| core | 37 | 17,879 | main churn 구역 |
| daemon | 28 | 28,236 | main churn 구역 |
| src-tauri 잔여 | 17 | ~11,200 | tray·daemon_client 제외분 |
| 프론트 `src/` | 108 | 20,616 | qa full 자동 승격 · 주석 비율 상위(`api/transport.ts` 74% · `i18n/index.ts` 58% · `api/agentClient.ts` 57%) |

**배치 순서 후보(미확정):** 원 지시서는 모듈 순서였으나, 주석 비율 순이 수확 대비 비용에 유리하다는 데이터가 나왔다(상위권 3개가 프론트 = qa full 비용 유의).

---

## repo 상태

- 브랜치 `wt2`, HEAD `5648497`. **커밋 4개 미푸시**(`b297848` `6f783c6` `4cf524d` `5648497`)
- **미커밋 dirty 2건 = 빌드 산출물**: `src-tauri/gen/schemas/desktop-schema.json`·`windows-schema.json`(`cargo build`가 갱신). 주석 작업과 무관 — 다음 세션이 커밋하거나 버릴지 정할 것. **clean-comment flow §0이 클린 트리를 요구하므로 다음 스윕 전에 처리 필요.**
- **wt2 워크트리에 `npm install` 완료** — 프론트 게이트(`npx tsc --noEmit`·`npm test`) 실행 가능(이전엔 `node_modules` 없어 불가였다)
- untracked 핸드오프 history 1건은 타 에이전트 것 — 건드리지 말 것

## 참조 (읽어야 할 것만)

- `~/.claude/skills/clean-comment/feedback.md` — 이번 세션에 6행 추가(🔴 4건). 특히 **`retrofit-완료`가 품질 보증으로 오독되는 구조** 행과 **fmt 비중립성** 행
- `docs/decisions/0046-*.md` 결정 3 — "진도 상태의 유일한 거처 = 웹뷰 **뷰(slot)** 단위". 창/뷰 어휘의 정본
- `CLAUDE.md` §혼동 쌍 — 창(WebView2) ≠ 슬롯 ≠ 프론트 컴포넌트. 이번에 워커가 위반했다
- 커밋 메시지 4개 — 무엇을 왜 지웠/고쳤는지 전건 기록됨
