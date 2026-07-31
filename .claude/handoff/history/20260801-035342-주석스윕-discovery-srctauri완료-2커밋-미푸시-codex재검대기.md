# 핸드오프: discovery·src-tauri 스윕 완료(2커밋·미푸시) — 다음=프론트 또는 master 병합 후 백엔드

## 한 줄 상태 · 다음 첫 액션

dashboard 전량 주석 retrofit 중. 이번 세션에 **워커 위임 방식으로 전환**해 두 구역을 완주했다 —
`discovery`(2파일 2,629줄) · `src-tauri 잔여`(13파일). **커밋 2개, 푸시 안 함**(사용자 지시: 나중에
Codex 적대 리뷰를 간단히 돌린 뒤 푸시).

**다음 첫 액션 = 아래 §잔여 표에서 구역 하나 골라 같은 파이프라인 반복.** 파이프라인은 이미
검증됐다: 워커 계약서(↓부록) → worker-senior 병렬 스폰 → 메인이 기계 증명 → Opus 적대 리뷰 →
FIX 반영 → `/qa standard` → 커밋.

---

## ★사용자 결정 (이번 세션 — 반드시 승계)★

1. **주석 작업에서 cross-family(Codex) 적대 리뷰 제외.** 사유 = 토큰 부족. 대체 = **Opus 적대 1인
   (`reviewer-deep`, model 명시) + 메인 기계 증명**. 범위 = 주석 작업 한정(다른 작업엔 원래 규약).
2. **커밋은 하되 푸시 금지.** 사용자가 나중에 Codex 적대를 간단히 돌린 뒤 푸시할 예정.
   → **다음 세션은 임의로 push 하지 말 것.**
3. **워커 팬아웃은 소규모부터 점증.** ("초반에 몇 개 해보고 늘려가지") 이번엔 2인 파일럿 →
   검증 통과 후 3인으로 확대. 5인 동시 스폰은 사용자가 중단시켰다.

## 메인 판단 (사용자 승인 없이 정한 것 — 뒤집어도 됨)

- **스윕 순서를 master 미접촉 구역 우선으로 변경.** 근거 = ↓§master 분기.
- **master 병합 안 함.** master HEAD 직전 커밋에 `★게이트 미통과·사용자 결정 대기★` 표시가 있어
  병합은 그 결정 이후 사용자 승인 사항으로 봤다.

---

## ★master 분기 — 병합 필요(이번 세션 실측)★

**양방향으로 갈라져 있다.** 워크트리라 나중에 통합해야 한다.

- `master`에만 6커밋: ADR-0122/0123 · A1 에이전트 명부 단일 입구 · 표시명 공백 정규화 ·
  step-log · **messaging 슬라이스 C(★게이트 미통과·사용자 결정 대기★)** · handoff
- `wt2`에만 8커밋: 주석 retrofit 7 + handoff 1

**충돌 위험 지도:** master 가 건드리는 중 = `crates/engram-dashboard-core/src/agent`,
`crates/engram-dashboard-messaging/src`, `crates/engram-dashboard-daemon/src`,
`crates/engram-dashboard-protocol/src`. **이 4구역 주석 스윕은 master 병합 후에 하는 게 안전하다.**
반대로 `src-tauri/`·`src/`(프론트)·`discovery`는 master 가 안 건드린다.

---

## 이번 세션 완료분

### 커밋 `0a17f5f` — discovery 구역 (워커 2인)
`src/lib.rs`(2,362 — 구획 A/B 분담 첫 실측) · `tests/stop_smoke.rs`(267).
삭제 1블록(9줄) + 정정 13건. **구획 분담 경로 실측 성공** — 2,362줄 파일을 워커 1인이 프로덕션/
테스트 두 구획으로 나눠 처리 가능했다(이전 핸드오프의 미검 항목 해소).

### 커밋 `a24f2e1` — src-tauri 잔여 (워커 3인 병렬)
`layout/` 5파일 · `commands/` 7파일 · `output_router.rs`+`cli.rs`+`lib.rs`+`output_channel.rs`+
`main.rs`. 삭제 2블록 + 정정 25건.

**가장 위험했던 수확 — 락 계약이 거꾸로 서술돼 있었다:** `output_router.rs`·`lib.rs` 가 "델타 송신은
락 해제 후 · **락 안에서 송신 금지**"라 단언했는데, 실제 호출부(`commands/layout.rs`·`popout.rs`)는
`rebuild` 와 **같은 critical section 안**에서 발화한다. 그게 F1/F2 REAL 동시성 버그 수정의 내용이고
ADR-0006 §5-1 이 근거다. **옛 주석을 따르면 고쳐진 버그가 되살아난다.** 워커 2인이 독립적으로 같은
모순을 짚어 교차 확인됐고, 리뷰어가 방향(어느 쪽이 참인지)을 코드로 확정했다.

기타 죽은 아키텍처 서술: "프론트가 WS 로 데몬에 직접 붙는다"(carrier 는 `TauriTransport` 고정,
ADR-0036) 2곳 · `AppState`(ADR-0029 제거) 2곳 · `window_bindings`·`slot_agent`·
`release_data_dir_from_exe`·`view:list-updated`(죽은 심볼/이벤트) · "ViewManager 가 락·emit 을
다룬다"(정반대가 설계 불변식).

---

## ★이번 세션의 방법론 수확 — 다음 세션이 이걸 이어야 한다★

**메인 직접 편집 → 워커 위임으로 전환했고, 성립한다.** 이전 핸드오프는 "절 판정은 지시서로 안
쪼개진다"며 메인 직접을 규정했으나, **워커 계약서에 판정 기준을 충분히 실으면 워커가 해낸다** —
2라운드 통틀어 워커 정정 38건 중 **오정정 0건**(적대 리뷰가 전건 코드 재검증).

**단, 워커가 반복해 밟는 4패턴이 있다(리뷰가 라운드마다 FIX 적출 — 전부 "안 손댄 쪽"):**

1. **"grounding 못 함" 오남용** — 접속된 두 절 중 앞절만 거짓이면 *삭제만으로* 완결된 참 문장이
   남는데도 "새 문안을 못 쓰겠다"며 stale 후보로 강등한다.
2. **반증 훅만 떼고 매끈한 거짓 잔존** — 틀린 메커니즘명을 지웠는데 남은 결론도 거짓이면, 옛
   문장보다 **더 나쁘다**(grep 할 심볼이 사라져 반증 불가).
3. **호출처 열거로 rot 재생산** — 썩은 이름 1개를 새 이름 2개로 갈아 끼운다.
4. **배너·`//!` 헤더 미검사** — 아무도 자르자고 안 해서 검사도 안 받는 최고령 주석. 2라운드 최악
   2건이 여기서 나왔다. 한 워커는 **동료 워커가 다른 파일에서 이미 거짓으로 지운 문장**을, 그
   두 줄 아래를 편집하면서 지나쳤다.

3·4번은 계약서 §2b 하드룰화 후 준수됐고, 1·2·4번은 여전히 일부 재발했다. **적대 리뷰를 빼면 안
된다** — 기계 증명(코드 토큰 0·토큰 수지)은 이 4패턴을 하나도 못 잡는다.

---

## 검증 상태

**돌린 것 (라운드마다 전부 PASS):**
- 기계 증명 2종: ① 주석 전용 diff 증명(출력 0줄 = 코드 토큰 변경 0) ② 보호 토큰 수지 1:1
  (2라운드: ADR-0006/0020/0029·T6×2·T7 정확히 대칭)
- `/qa standard`: build · 5멤버 회귀 0 fail · `cargo fmt --check` · 코어 격리 0줄 ·
  메시징 격리 0줄 · `npx tsc --noEmit` 0 · vitest 41파일/634 테스트
- Opus 적대 리뷰 2회: 1R FIX 7건 · 2R FIX 6건 — **전건 반영 후 게이트 재통과**

**재실행 명령:**
```bash
cargo build
cargo test -p engram-dashboard-core -p engram-dashboard-protocol -p engram-dashboard-discovery -p engram-dashboard-messaging -p engram-dashboard-daemon
cargo fmt --check
rg "^\s*use tauri" crates/engram-dashboard-core/src/                                    # 0줄 = PASS
rg "engram_dashboard_(core|daemon|protocol|discovery)" crates/engram-dashboard-messaging/src/  # 0줄 = PASS
npx tsc --noEmit && npm test
```

**검증 안 된 것:**
- **GUI 실측(cdp) 0회.** 주석 전용(코드 토큰 변경 0 기계 증명)이라 UI=full 승격 조건 미해당으로
  판단해 미수행. 프론트 `src/` 를 만지는 배치도 같은 논리가 성립하는지는 **미결**(그 판단을 그대로
  쓸지 사용자 확인 권장).
- **Codex(cross-family) 적대 미실행** — 사용자 결정. 두 커밋 모두 단일 family 검증 상태다.
- **`src-tauri/bindings/ViewMeta.ts`(생성물)에 폐기된 `view:list-updated` 서술이 남아 있다.**
  손으로 못 고친다(재생성되면 덮임). 재생성은 src-tauri 테스트가 필요한데 qa 바인딩이 그걸 금지
  (bare `cargo test` = WebView2 크래시). **해법 미정 — 별도 과업.**

---

## 실패한 접근 (do-not)

1. **`cargo fmt --check` 생략 금지** — 주석 삭제는 포맷 중립이 아니다.
2. **경량(Sonnet) 워커에 재작성 몰아주기 금지.** 이번 5워커 전부 `worker-senior`(Opus) 고정.
3. **`git checkout -- <file>` 은 자동 모드 분류기에 막힌다.** 원복은 다른 경로로.
4. **워커 5인 동시 스폰 → 사용자가 중단시킴.** 소규모 검증 후 점증할 것.
5. **워커에게 `cargo check`/`cargo fmt` 를 시키지 말 것** — 동시 실행 시 빌드 락 경합. 메인이
   중앙에서 한 번 돌린다(계약서에 override 로 명시돼 있음).
6. **적대 리뷰 결과를 "정정은 다 맞았으니 PASS" 로 읽지 말 것.** 2라운드 모두 정정 자체는 100%
   정확했고 판정은 FIX였다 — 결함은 항상 **안 손댄 쪽**에 있었다.
7. **`src-tauri/gen/schemas/*.json`·`protocol/bindings/*.ts` 의 `M` 표시는 블로커 아님** —
   CRLF/LF 정규화 노이즈(`git diff --numstat` 빈 출력 = 내용 동일). `git add` 로 해소.

---

## 정지 조건 (멈추고 사용자에게 물을 것)

- **`crates/engram-dashboard-messaging/src/service.rs`(11,149줄)** — 구획 분담으로도 상한 초과.
  임의 부분 스윕 금지, 에스컬레이션.
- **push 금지** — 사용자가 Codex 재검 후 직접 판단한다(↑사용자 결정 2).
- **master 병합** — master 쪽 미결(게이트 미통과 커밋) 때문에 사용자 결정 사항.
- **코드 토큰 변경이 필요한 발견** — 주석 스윕 범위 밖. 별도 `/implement` 로 (↓잔여).
- **스킬 파일 직접 수정 금지** — 결함은 각 `feedback.md` 에만(이번에 qa·review·clean-comment
  각 1행 추가). 반영은 skill-factory 소관.

---

## 잔여

**미스윕 구역(실측):**

| 구역 | 파일 | 줄 | master 충돌 위험 | 비고 |
|---|---|---|---|---|
| 프론트 `src/` | 108 | 20,616 | **없음** | 다음 후보 1순위. 상위: `protocolClient.test` 941 · `protocolClient` 837 · `wsTransport.test` 784 · `ViewLayoutRenderer.test` 717 · `AgentList` 642 |
| core | 37 | 17,879 | **있음** | master 가 `src/agent` 수정 중 |
| daemon | 28 | 28,236 | **있음** | master 가 `src/`·`src/control`·`src/bin` 수정 중 |
| messaging | 7 | 18,536 | **있음** | `service.rs` 11,149 = 에스컬레이션 |
| `daemon_client/tests.rs` | 1 | 3,153 | 없음 | 이전 세션 미스윕분 |

**프론트 착수 시 결정 필요:** qa 바인딩상 프론트 경로가 닿으면 full(cdp 실측) 자동 승격인데, 주석
전용이면 코드 토큰 변경 0이라 화면 동작에 닿을 수 없다. 이번 백엔드 배치엔 "승격 조건 미해당"으로
판단했으나 **프론트에 같은 논리를 적용할지는 사용자 확인 권장**(qa feedback 에 이미 관련 행 있음).

**코드 변경 과업(주석 스윕 밖 — 별도 `/implement`):**
- `commands/popout.rs:145-153` — 창 생성 실패 롤백 경로만 델타를 **락 밖**에서 발화한다. F1/F2
  수정이 없앴다고 선언한 그 형태다(실질 위험은 작다 — 갓 만든 빈 창이라 `to_unsubscribe` 가 거의
  항상 빈다). 선택지 = ①송신을 락 안으로 옮긴다(코드 변경) ②주석이 균일성을 주장하지 않는다.
  **이번엔 ②를 택해 주석에 예외를 명시**했다. ①은 사용자 결정.
- `protocol_state.rs:371`(+`:364`) — assert 메시지 방향 역전(`epoch 11→10` ↔ 테스트는 10→11) +
  `render_seq` 는 ADR-0046 으로 존재하지 않음.
- `daemon/src/connection_core.rs:891,893,919` — 죽은 `EmbeddedClient`/`spawn_agent` 미러 참조.
- `commands/discovery.rs:105,63`(`read_daemon_info`·`discover_daemon`) · `manager.rs:190`
  (`slot_agent`) — **운영 호출처 0**. 레거시 `WsTransport` 경로용으로 남은 것인데 "의도적 부재의
  이유" 주석이 없어 다음 세션이 죽은 코드와 구분 못 한다. 주석 추가 = 사용자 확인 후.
- `src/api/daemonControl.ts:51` — discovery 에서 고친 `CREATE_NO_WINDOW` 거짓을 복창 중(프론트
  라운드에서 처리).

---

## repo 상태

- 브랜치 `wt2`, HEAD `a24f2e1`. **`origin/wt2` 는 `e7f6306` 에 머물러 있다 — 신규 2커밋 미푸시(의도).**
- 워킹 트리 클린.
- `node_modules` 이 워크트리에 설치됨 — 프론트 게이트가 이 좌석에서 돈다.
- 다른 PC 승계 시: `git fetch && git checkout wt2` → **`npm install` 필요** → 위 재실행 명령.
  단 **미푸시 2커밋은 이 좌석에만 있다** — 다른 PC로 넘기려면 푸시가 선행돼야 한다(사용자 승인).

---

## 부록 — 워커 계약서 (scratchpad 는 휘발이라 여기 보존)

다음 세션은 이 내용으로 계약서 파일을 재생성해 워커에게 Read 시키면 된다.

**필수 선독(3종):** `~/.claude/skills/clean-comment/references/rules.md` · `style-base.md` ·
`.claude/skill-bindings/clean-comment.md`. 밀도 정합은 `retrofit-완료` 범위 밖이면 비활성 —
허용/금지 목록만으로 판정한다.

**손대는 범위(사용자 방침):** (A) 금지 목록에 **명확히** 걸리는 것 (B) **코드로 반증된** 거짓.
그 외 전부 보존. 재량 정리·문체 개선·고가치 WHY 축약 전부 범위 밖.

**정정 방법(우선순위):** ①거짓 절만 삭제 ②정본 코드 **한 곳**으로 포인터화 ③코드에서 바로 읽히는
범위 한정어 추가. 확신 없으면 편집하지 말고 stale 후보로 보고.

**두 축 필수:** ①블록 통째 판정 ②**남긴 블록 안의 절 판정**(고가치 블록일수록 그 안에 거짓이 숨는다
— 누구도 지우자고 안 하니 검사도 안 받는다).

**§2b 하드룰(위 4패턴 — 실측 기반, 반드시 포함):** ①삭제만으로 참 문장이 남는지 먼저 검사
②편집 후 남은 문장이 단독으로 참인지 재독 ③호출처 열거 금지(정본 한 곳만) ④배너·`//!` 헤더 전용
검사 패스 배정.

**하드 제약:** 주석 전용(코드 토큰 0 — 속성·`#[cfg]` 는 코드) · 보호 토큰(`ADR-NNNN`·`T<n>`·
`M<n>`·`Fix X`·`FIX-<n>`) 재작성 후에도 문장에 잔존 · 보존 범주(경계 관례 3부류·도구 지시·
**의도적 부재의 이유**·정확성 논증·`//!` 헤더).

**검증(워커):** ①주석 전용 diff 증명 ②보호 토큰 수지 — **`cargo check`/`fmt` 는 시키지 말 것**
(동시 실행 빌드 락 경합, 메인이 중앙 실행). 커밋 금지.

**기계 증명 명령(메인):**
```bash
git diff -U0 -- <경로> | rg "^[+-]" | rg -v "^(\+\+\+|---)" | rg -v "^[+-]\s*(//|///|//!)" | rg -v "^[+-]\s*$"
git diff -- <경로> | rg "^-" | rg -o "T[0-9][a-z]?|ADR-[0-9]{4}|Fix [A-C]|FIX-[0-9]|M[0-9]" | sort | uniq -c
git diff -- <경로> | rg "^\+" | rg -o "T[0-9][a-z]?|ADR-[0-9]{4}|Fix [A-C]|FIX-[0-9]|M[0-9]" | sort | uniq -c
```

---

## 참조 (읽어야 할 것만)

- `~/.claude/skills/clean-comment/references/rules.md` + `style-base.md` — 주석 기준 정본
- `.claude/skill-bindings/clean-comment.md` — `retrofit-완료` 실범위(아직 `src-tauri/src/tray`·
  `daemon_client` 2행뿐 — **이번에 완주한 discovery·src-tauri 잔여 추가는 사용자 승인 후**)
- `~/.claude/skills/{qa,review,clean-comment}/feedback.md` — 이번 세션 각 1행 추가
- 커밋 `0a17f5f`·`a24f2e1` 메시지 — 무엇을 왜 고쳤는지 전건 기록
- `CLAUDE.md` §혼동 쌍 — 창(WebView2) ≠ 슬롯 ≠ 프론트 컴포넌트
