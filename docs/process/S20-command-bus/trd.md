# TRD — 통합 command 버스 (S20)

> 상태: **개정 2판(2026-08-13).** 이 판은 **ADR-0134**(S20 구조 자체)와 **ADR-0135**(발견 경로 3건 — 등록 패킷에 모양 동봉 · 값 읽기 v1 포함 · tombstone 만료 없음)를 반영해, 그 셋에 걸려 있던 미확정·미결 항목을 닫는다.
> 이력 — **개정 1판(2026-08-13).** 구판(2026-08-12)은 **선언을 `engram-dashboard-protocol` 한 곳에 모으는 안** 위에 세워졌고, 같은 세션 후반 대화에서 그 안이 뒤집혔다. 1판의 정본은 세션 인계 `.claude/handoff/latest.md` 「확정된 구조」·「세션 중 뒤집힌 판단 3건」이며, 두 절과 어긋나는 구판 문장은 전부 1판에서 개정했다.
> 부분 정정 — **Step 1 실측(2026-08-14).** 구현이 끝난 칸을 실측값으로 고치고(고친 자리마다 `★개정(Step 1 실측 2026-08-14)★`), §4에 ⑨(릴리즈 패닉 정책 — 사용자 결정)를 더했다. **판을 올리지 않는다** — 설계가 뒤집힌 것이 아니라 회계와 서술이 틀렸던 자리라, 같은 표기 규약(구판 축약 보존)으로 그 칸에만 박았다.
> **읽는 법:** 뒤집힌 자리는 `★개정★`으로 표시하고 **구판 문장을 줄여서 남긴다.** 결과만 남기면 다음 세션이 같은 안을 다시 꺼낸다.
> 앵커: **ADR-0134(S20 구조) · ADR-0135(발견 경로 3건)** · ADR-0022(방향) · ADR-0055(프론트 레지스트리 골격) · ADR-0064(메뉴 = command id 참조) · ADR-0081(릴레이 형태 — 이 문서는 **확장**이지 번복이 아니다) · ADR-0132 결정 7(후속 순서) · ADR-0129(net 격리) · ADR-0035/0057(레이아웃 권위) · ADR-0003(코어 격리) · ADR-0012(테스트 격리) · CLAUDE.md 「LLM-우선 제어」.
> ★개정★ **선행 조건이 사라졌다.** 구판: "선행 = ADR-0081 릴레이 구현. 이 TRD 밖. §6 Step 3 이후는 그 배선 위에 선다." → 현판: **그 배선을 이 TRD가 짓는다**(§6 Step 3). 확정 구조가 "새로 짓는 것은 둘뿐 — 공통 도구 crate + 데몬→셸 인바운드 수신"이라, 인바운드 수신기가 곧 ADR-0081 릴레이의 실물이기 때문이다. 실측 근거는 그대로 유효하다(재확인 2026-08-13): `RegisterRole`·`RelayUi`·`UiCommand`·`UiResult` `.rs` hit 0 · `src-tauri/src/daemon_client/`(`connection.rs`·`lifecycle.rs`·`mod.rs`·`protocol_state.rs`·`replay_flight.rs`·`tests.rs` — 6개. 구판은 `tests.rs`를 빼고 5개로 적었다)의 읽기 루프가 `Message::Text`를 `AgentEvent`로만 디코드한다(`src-tauri/src/daemon_client/connection.rs:805`, 이벤트 분기 `:1041~1072`).

---

## 0. 확정된 구조 (이 판의 정본 요약)

```
① 선언 + 본문 = 한 세트, 생산자 모듈 옆에
     core        agent.*            (AgentManager 가 실제 일을 하니 여기)
     daemon      mail.*
     src-tauri   window/tab/slot
     webview     theme/chat

② 공통 도구 crate (신규) — 봉투 · 오류 · 선언 매크로만. 워크스페이스 의존 0. 명령은 0개
③ 조립은 실행 파일에서 — 이름·인자 스키마는 자동 수집, 실물은 조립 때 주입(MakeTable(deps))
④ 배달은 홉마다 같은 3단계 — 내 표에 있나 → 명부에 있나 → 오류
```

★개정(2판)★ **등록이 나르는 것 = `{이름 + 모양}`.** 위 블록은 등록 페이로드를 적지 않아 1판이 그것을 「이름만」으로 읽었고(§2-4 ③ 구판 문장), ADR-0135가 그 읽기를 뒤집었다. `모양`은 **불투명 문자열 한 칸(`help`)**이고 데몬은 저장·중계만 한다(§3-7).

**뒤집힌 판단 3건**(구판이 반대를 말한다):

1. **선언을 `protocol` 한 곳에 모으는 안 → 폐기, 생산자 옆으로.** 귀결로 「주인 칸」과 「ADR-0081 가드 부분 폐기 도장」 둘이 동시에 불필요해졌다(§2-2 · §9 모순 1·2).
2. **전역 자동등록 → 금지.** 자동 수집은 이름·인자 스키마까지, 핸들러 실물은 조립 때 주입(§3-3 · §7).
3. **데몬이 빌드 목록을 조회 → 폐기.** 데몬 명부는 **런타임 등록만으로** 찬다(§3-7).

---

## 1. 목표 · 비목표

### v1이 하는 것

- ★개정★ **명령의 정적 계약을 생산자 모듈 옆에 선언한다** — 이름 · 인자 모양 · 반환 · 타입드 오류 · 읽기/쓰기 표식 · 세대 번호. 구판: "**Rust 한 곳에** 선언한다 · 선언처 = `engram-dashboard-protocol`(워크스페이스 의존 0)". 폐기 근거: 사용자 지적 「생산자와 사용자가 명확히 분리돼야 한다」. `protocol`이 의존 0이라는 실측(`crates/engram-dashboard-protocol/Cargo.toml`에 `path =` 항목 0줄 — 재실측 2026-08-13)은 여전히 참이지만, **그 crate는 wire 계약이라 명령 어휘의 집주인이 아니다.**
- ★개정★ **선언에 「주인」 칸을 두지 않는다.** 선언이 사는 crate가 곧 주인이다. 구판: "`#[owner(Daemon|Shell|View)]` 칸을 둔다". 폐기 근거: step-log.md:1669(사용자 지적, 2026-08-12) + 인계 「do-not 2 — 사용자가 두 번 철회시킨 제안」.
- **도구만 담는 신규 crate 하나를 판다** — 봉투 · 오류 · 선언 매크로 · 표/라우팅 계약. **명령은 0개, 워크스페이스 의존 0.** 근거: `core`는 지금 워크스페이스 의존이 **0개**다(실측 2026-08-13 — `crates/engram-dashboard-core/Cargo.toml`에 `path =` 0줄). `core`가 `agent.*`를 선언하려면 도구가 필요한데, 그것을 `protocol`에서 받으면 코어가 **wire 타입까지** 보게 된다. 도구 crate가 그 유입을 막는다.
- **파생물 둘을 뽑는다** — TypeScript 바인딩(기존 ts-rs 경로)과 LLM용 JSON 스키마. ★개정★ 단 **생성 지점이 crate마다로 흩어진다**(§2-4 · §5 게이트 영향 — 여기서 기존 CI 게이트 하나가 부족해진다).
- ★개정★ **각 프로세스가 자기 표를 채우고 자기가 구현한 것을 `{이름 + 모양}`으로 데몬에 등록한다.** 구판: "자기가 구현한 **이름만** 등록한다 · 데몬은 주인 토큰 → **이름 집합**만 쥔다". 현판에서 데몬이 쥐는 것은 주인 토큰 → `{이름, help}` 집합이고, `help`는 **불투명 문자열**이라 데몬은 여전히 "클라이언트 셸"이라는 구체 개념도 명령의 뜻도 배우지 않는다(ADR-0135 · §3-7).
- ★2판 추가★ **발견 경로가 왕복 없이 닫힌다.** 등록이 이름과 모양을 함께 나르므로 **프로세스 밖 호출자(LLM·CLI)가 명부 조회 한 번으로 인자를 채울 수 있다** — 소유자에게 되묻는 왕복이 없고, 주인이 꺼져 있어도 마지막으로 등록된 모양이 tombstone에 남아 있다(§2-4 ③ · §3-7 조항 4).
- **대칭 봉투 하나로 전달한다** — `{ name, request_id, owner, proto_ver, args }` + 프레임 종류(Request/Reply/Event). **방향 필드는 없다** — 어느 연결에 썼는가가 방향이다(§3-2).
- ★개정★ **디스패치 규칙이 전 프로세스·전 홉에서 같은 3단계다** — `내 표에 있나 → 명부에 있나 → 오류`. 구판은 2단계였다("내 맵에 있으면 직접 실행, 아니면 봉투로 보낸다"). 중간 홉(셸)이 명부를 갖게 되면서 단계가 하나 늘었다(§3-5 · §3-8).
- **사람 클릭과 중계된 에이전트 호출이 같은 핸들러 함수에 떨어진다.** 셸 소유 명령에서 그 함수 = ADR-0081이 이미 요구한 단일 적용 서비스.

### v1이 하지 않는 것 (명시)

| 안 하는 것 | 왜 |
|---|---|
| 취소 · 진행 보고 | §4-⑤ — 계약 자리만 예약하고 기제는 안 만든다 |
| 키바인딩 커스터마이징 | ADR-0055가 이미 골격 밖으로 미뤄둔 것. 이 버스가 열어줄 뿐 v1 범위 아님 |
| 픽셀(위치·크기) 제어 | ADR-0132 결정 7 ③ — 레이아웃 디스크 영속 선행 |
| 다중 앱 인스턴스 | ADR-0081 「단일 앱 인스턴스 전제」 유지. 주인 토큰은 다중을 표현할 수 있으나 v1 정책은 last-wins 하나 |
| 40개 tauri command 전량 이관 | §6 — 23개는 남긴다(이유 거기) |
| 선언을 한 crate에 모으기 | ★개정★ **폐기된 구판 안.** 이제는 비목표가 아니라 **금지**다(§0 뒤집힌 판단 1) |

★개정★ **구판 표에는 「값 읽기(조회)를 명령 목록에 담기 — §8 가정 B 미결」 행이 있었다.** ADR-0135로 **포함**이 확정돼 그 행을 삭제한다. 읽기는 이제 비목표가 아니라 v1 범위다(§8 가정 B · §6 Step 3·5).

---

## 2. 명령 정의 형태

### 2-1. 어휘 근거 — 지금 실재하는 세 어휘

| 어휘 | 형태 | 실물 |
|---|---|---|
| 화면 | 점 구분 id | `theme.set`(`src/commands/themeCommands.ts:9`) · `tab.create`(`src/commands/tabCommands.ts:29`) · `agent.spawn`(`src/commands/agentCommands.ts:41`) — 총 33개 |
| CLI | 계열 + 동사 | `CLI_GROUP_AGENT="agent"`(`crates/engram-dashboard-core/src/agent/types.rs:237`) + `CLI_AGENT_VERBS=["list","spawn","new","rename","move"]`(`types.rs:246`) · `CLI_GROUP_MAIL="mail"`(`types.rs:179`) + `CLI_MAIL_VERBS=["send","status","pending"]`(`types.rs:212`) |
| wire | PascalCase variant | `AgentCommand::SpawnProfile`(`crates/engram-dashboard-protocol/src/messages.rs:124`) 포함 **총 25종**(`messages.rs:25-214` — 재실측 2026-08-13. 구판 "외 25종"은 26종을 함의해 1 어긋났다) |

**카탈로그 이름 = 점 구분 `<계열>.<동사>`로 통일한다.** (구판 그대로 — 뒤집히지 않았다.) 화면 어휘를 정본으로 삼는 이유는 33개가 이미 그 형태로 안정 id를 쌓았고(ADR-0055 「안정적 id」), CLI 표면은 점을 공백으로 바꾸면 그대로 나오기 때문이다 — `agent.spawn` → `engram agent spawn`. 반대 방향으로 가면 화면 33개 id를 전부 갈아야 한다. 이 규칙이 서면 step-log.md:1673 ②(화면 어휘와 데몬 어휘가 서로를 모른다)가 닫힌다.

**덧붙임(개정):** 어휘 통일과 선언 위치는 **별개 축**이다. 이름 규칙이 하나라는 것이 선언이 한 파일에 모여야 한다는 뜻이 아니다 — 구판은 이 둘을 붙여 읽어 중앙화로 갔다.

### 2-2. 선언 구문 ★개정★

**구판:** `engram-dashboard-protocol` 안의 선언 전용 매크로 · `#[owner(...)]` 칸 포함 · 「새 crate 0」이 장점.
**현판:** 선언 매크로는 **신규 도구 crate**가 제공하고, **호출은 생산자 모듈 안에서** 한다. `#[owner(...)]` 칸은 삭제한다.

> ★개정★ **crate 이름 = 확정. `engram-dashboard-command`.** (ADR-0134.) 구판: "이름은 **잠정** — 네이밍은 구현 갈림길이라 **사용자 결정 사항**(CLAUDE.md 「개발 스텝」 2)". 확정됐으므로 §5 게이트 정규식도 이 이름으로 박는다(더는 「이름이 바뀌면」 조건절이 붙지 않는다).

```rust
// crates/engram-dashboard-core/src/agent/commands.rs   ← 일하는 코드 옆
use engram_dashboard_command::declare_commands;

declare_commands! {
    /// 에이전트를 띄운다(잠든 것 깨우기 포함).
    #[effect(Write)] #[since(1)]
    "agent.spawn" => args AgentSpawnArgs {
        /// 깨울 대상 이름. cwd 와 상호배타.
        target: Option<String>,
        /// 새로 만들어 띄울 작업 디렉토리. target 과 상호배타.
        cwd: Option<String>,
        name: Option<String>,
    } -> ok AgentSpawnOk {
        agent_id: String,
        state: String,
    } errors [INVALID_ARGUMENT, NOT_FOUND, NAME_TAKEN, INTERNAL];
}

/// ★조립 때 주입★ — 전역 static 표를 만들지 않는다(§7 규칙 T-1).
pub fn make_table(manager: Arc<AgentManager>) -> CommandTable { /* … */ }
```

매크로가 만드는 것은 넷이다.

1. **인자·반환 struct** — ★개정(Step 1 실측 2026-08-14)★ 매크로가 다는 것은 `#[derive(Serialize, Deserialize, TS)]`**까지**다. **구판(축약 보존):** "`#[derive(Serialize, Deserialize, TS)] #[ts(export)]`가 붙는다 · `messages.rs:12-13`의 기존 패턴 그대로다." → `#[ts(export)]`는 **안 단다**(실측: 매크로 출력·선언 어디에도 0건). 대신 `crates/engram-dashboard-core/tests/ts_export.rs`가 `export_all_to`로 **명시 export**한다(그 파일 헤더가 근거를 적고 있다). `#[ts(export)]`가 만드는 것은 ts-rs의 암묵 테스트이고, 거기 기댔다가 손관리 생성물이 된 `src-tauri/bindings/`의 실패 모드는 **§5가 정본**이다 — 되풀이하지 않으려고 protocol의 `tests/ts_export.rs`와 같은 형태로 export를 직접 부른다.
2. **`CommandSpec` 정적 항목** — ★개정★ 구판은 `protocol` 안의 `COMMAND_CATALOG: &[CommandSpec]` 배열 하나에 넣었다. 현판은 **링커 수집**으로 모은다: 각 선언이 자기 crate에서 정적 항목을 등록하고, 링크된 실행 파일이 `command_specs()`로 자기 바이너리 안의 전량을 훑는다. → **바이너리마다 보이는 집합이 다르다**(데몬 = core+daemon 선언, 셸 = core+protocol+src-tauri 선언). 이것은 결함이 아니라 의도다 — 각 프로세스는 자기가 조립할 수 있는 것만 알면 된다.
   - **대가(보고 대상):** 링커 수집은 **새 외부 crate 1개**(`inventory` 계열)를 들인다. CLAUDE.md 「의존성(변경 시 보고)」에 걸리는 항목이다. 구판의 "새 crate·새 툴체인 0"은 여기서 깨진다 — 다만 깨지는 것은 **외부 의존 1개**이지 새 IDL 툴체인이 아니다.
3. **JSON Schema 문자열** — 매크로가 필드 이름·타입을 구문에서 읽어 그대로 찍는다. ★2판 추가★ **이 문자열이 곧 등록 패킷의 `help`가 된다**(§3-7 · ADR-0135) — Rust 선언은 매크로가 자동으로 채우므로 손으로 적을 것이 없고, TypeScript 선언만 한 줄을 손으로 적는다.
   - ★**2판 확정 — `help`가 「어느 문자열」인가**★: **그 명령의 파생 스키마 항목 하나를 통째로 직렬화한 JSON 텍스트**다. 즉 §2-4 ②의 `commands.schema.json` `commands` 배열의 **원소 하나**(`name`·`effect`·`since`·`summary`·`args`·`ok`·`errors`)가 그대로 `help` 값이 된다. `args_schema` 한 칸도, `ok_schema` 한 칸도 아니다 — **항목 전체**다. 이유 둘: ㉠ 파생 파일과 등록 패킷이 **같은 한 출처**(매크로가 찍는 그 항목)에서 나오므로 둘이 갈릴 수 없고, ㉡ `ok`가 함께 실려 **조회 명령의 반환 모양**(§8 가정 B)이 자동으로 덮인다. 매크로는 이 항목 하나를 찍고, `make_table`/등록 경로는 그것을 **문자열로** 실어 보낸다.
   - **대가: 허용 타입 알파벳이 닫힌다**(`String`·`bool`·`u32`/`i64`/`f64`·`Option<T>`·`Vec<T>`·선언 블록 안에서 선언된 struct/enum). 그 밖은 컴파일 에러. 실무를 덮는지는 **미확인** — Step 1에서 실제로 선언해 보고 좁으면 `schemars` 도입 여부를 사용자 결정으로 올린다.
4. **`CATALOG_VERSION`** — ★개정★ 구판은 "선언 블록이 바뀔 때 손으로 올리는 **전역 상수** 하나". 현판은 **crate마다 하나**다(`core::CATALOG_VERSION` 등). 이유: 선언이 흩어졌는데 번호가 하나면 남의 crate 변경 때문에 내 번호가 올라간다. 봉투의 `proto_ver`는 **보낸 쪽 crate의 세대**이고, 받는 쪽은 이 값만으로 거절하지 않는다(§4-①).

★삭제된 것★ — 구판 §2-2 말미의 "「`#[owner(...)]` 칸을 두는 근거는 §9 모순 1을 볼 것」". 칸 자체가 없어져 근거도 함께 사라진다(§9 모순 1).

### 2-3. 주인별 실례 1건씩 ★개정★

**구판:** 「주인 **등급**별」 표 — `OwnerClass` 칸(`Daemon`/`Shell`/`View`)이 선언에 있다는 전제.
**현판:** 주인은 선언이 사는 곳이다. 아래 표의 첫 칸은 **선언처**이고 등급 필드가 아니다.

| 선언처 (= 주인) | 예시 | 본문이 사는 곳 | 근거 |
|---|---|---|---|
| `engram-dashboard-core` | `agent.spawn` | `AgentManager`. wire는 기존 `AgentCommand::SpawnProfile`(`messages.rs:124`)/`SpawnByCwd`(`messages.rs:98`) | 에이전트 제어는 전부 데몬 프로세스가 쥔 `AgentManager`에 있다. **선언은 코어**(일하는 코드가 코어에 있으므로), **호스팅은 데몬**(ADR-0029) — 이 둘이 갈리는 유일한 자리다 |
| `engram-dashboard-daemon` | `mail.*` | 데몬. CLI 어휘 `CLI_MAIL_VERBS`(`types.rs:212`)가 지금의 표면 | 우편 호스트 = 데몬(ADR-0110 조립실) |
| `src-tauri` | `tab.create` | `src-tauri`. 오늘은 `create_tab`(`src-tauri/src/commands/layout.rs:98`, 속성은 `:97`)이 `ViewManager::create_tab`을 **직접** 부른다(`layout.rs:108`) | 레이아웃 권위 = `src-tauri`(ADR-0035/0057) |
| 웹뷰(`src/commands/`) | `theme.set` | 웹뷰(TypeScript, `src/commands/themeCommands.ts:9`) | ★개정★ 구판: "**선언만 Rust에 두고** 구현은 TypeScript(C++ 헤더처럼 가른다)". 현판: **선언도 TypeScript에 둔다** — 선언과 본문을 가르지 않는다. 대신 셸이 웹뷰의 이름 목록을 **대신 등록한다**(§3-7). 구판의 근거였던 step-log.md:1669 뒷문장("화면 전용 명령은 Rust 모듈이 없어 목록에서 빠지므로 선언만은 Rust에")은 **런타임 등록이 그 구멍을 메우면서 불필요해졌다** |

★위 표의 마지막 행이 이 개정의 핵심이다★ — 「화면 명령이 목록에서 빠진다」가 구판의 중앙화를 정당화한 유일한 실제 문제였고, 등록 wire가 그것을 푼다.

### 2-4. 파생물이 정확히 어떤 모양인가 ★개정★

**① TypeScript 바인딩** — 기존 경로를 쓰되 **생성 지점이 여럿이 된다.**
- `protocol`: `crates/engram-dashboard-protocol/tests/ts_export.rs:9-20`이 `export_all_to("<crate>/bindings/")`를 부른다(현재 23개 파일).
- `src-tauri`: **이미 자기 `bindings/`를 갖고 있다** — `src-tauri/bindings/`에 8개(`LayoutNode.ts`·`View.ts`·`SlotContent.ts` 등), 선언은 `src-tauri/src/layout/types.rs:15,31,59,78,91,100`의 `#[ts(export)]`. 그 파일 헤더가 배치 규약을 자인한다: "이 타입들은 **src-tauri 안에서만** 정의·export 된다. protocol/daemon crate 에 …"(`types.rs:3`).
- `core`: 신설(오늘 `bindings/` 없음).

★이 실물이 개정의 선례다★ — 셸 소유 타입을 protocol에 누설하지 않기 위해 **선언처 옆에서 TS를 뽑는 방식이 이 repo에 이미 서 있다.** 구판은 그 선례를 거슬러 명령 어휘만 protocol로 모으려 했다.

```ts
// crates/engram-dashboard-core/bindings/AgentSpawnArgs.ts (생성물, 신설)
export type AgentSpawnArgs = { target: string | null, cwd: string | null, name: string | null, }
```

**② LLM용 JSON 스키마** — ★개정★ 구판은 "`protocol`의 같은 테스트가 한 파일 더 쓴다"였다. 선언이 흩어져 **한 파일로 모을 주체가 사라졌다.** 현판은 crate마다 자기 몫을 찍는다(`crates/*/bindings/commands.schema.json`).

```jsonc
// crates/engram-dashboard-core/bindings/commands.schema.json (생성물)
{
  "catalogVersion": 1,
  "commands": [
    { "name": "agent.spawn", "effect": "Write", "since": 1,   // ← "owner" 칸 없음(★개정★)
      "summary": "에이전트를 띄운다(잠든 것 깨우기 포함).",
      "args": { "type": "object",
                "properties": { "target": { "type": ["string","null"] } } },
      "ok":   { "type": "object", "properties": { "agent_id": { "type": "string" } } },
      "errors": ["INVALID_ARGUMENT", "NOT_FOUND", "NAME_TAKEN", "INTERNAL"] }
  ]
}
```

★2판 확정★ **위 `commands` 배열의 원소 하나 = 등록 패킷의 `help` 문자열 하나다**(§2-2 3 · §3-7). 파일과 패킷은 **다른 경로로 나가되 같은 출처**를 쓴다 — 매크로가 항목을 한 번 찍고, 디스크에는 배열로 모아 쓰고 wire에는 항목을 문자열로 실어 보낸다. 그래서 파생 파일이 갱신되지 않아도(§5 파생 게이트 문제) **명부에 등록되는 모양은 바이너리와 항상 일치한다.**

**③ 이 파생물은 누구를 위한 것인가 — 범위를 좁힌다.** ★개정★ 구판은 파생물이 **LLM의 발견 경로**를 겸한다고 읽었다. 현판에서 파생물의 역할은 **웹뷰가 타입 안전하게 부르게 하는 것**이고, "지금 부를 수 있는가"는 **런타임 등록**이 답한다(§0 ③·④, 인계 「이름을 아는 두 층」).

★2판 단서★ 위 좁히기는 유지된다 — **디스크의 파생 파일이 발견 표면인 것은 아니다.** 다만 매크로가 찍는 스키마 항목(위 배열의 **원소 하나**)이 직렬화돼 **등록 패킷의 `help`로 실려 나간다**(§2-2 3 · ADR-0135). 즉 같은 내용이 두 경로로 나가되, LLM이 읽는 것은 파일이 아니라 **명부에 등록된 사본**이다.

★**개정 — 닫혔다(ADR-0135)**★: LLM이 **셸·웹뷰 소유 명령의 인자 모양**을 어디서 얻는가 = **등록 패킷에 동봉한다.**

**구판(축약 보존):** "★미확인 — 이 개정이 만든 실제 구멍★. 등록 패킷은 **이름만** 나르고(확정 구조) 데몬 명부에 스키마가 없다 · 웹뷰 명령은 TypeScript 선언이라 **Rust 파생물에 아예 없다** · `engram` CLI는 ADR-0081 가드상 UI 어휘를 컴파일에 넣지 않아 자기 안에도 없다. 후보 둘 — ㉠ 등록 패킷에 스키마 blob 동봉 ㉡ 소유자에게 `command.describe` 왕복. **임의 확정하지 않는다 — 사용자 결정 사항.**" (구멍의 진단 자체는 유효했다. 아래가 그 답이다.)

**채택 = ㉠ 등록 패킷 동봉.** 등록이 나르는 단위는 `{name, help}`이고 `help`는 **불투명 문자열 한 칸**이다(§3-7). 데몬은 그것을 저장·중계만 하고 **파싱·검증·분기하지 않는다**(하드 제약 — §3-7). Rust 선언은 매크로가 자동으로 채우고(§2-2 3), TypeScript 선언은 **손으로 한 줄** 적는다(§7 seam 표가 「비어 있지 않은지」를 단언한다).

**거부 = ㉡ `command.describe` 왕복.** 이유 셋:
- **발견 경로에 실패 모드가 생긴다.** 모양을 얻는 데 네트워크 왕복이 끼면 발견이 타임아웃·주인 부재로 실패할 수 있고, 그러면 §4-②·④의 오류 표를 **발견 단계에도** 한 벌 더 깔아야 한다.
- **등록은 1회지만 조회는 매번이다.** 동봉은 붙을 때 한 번 나르면 끝이고, 왕복은 물어볼 때마다 홉을 탄다(§3-8의 2단 배달을 그대로).
- **앱이 꺼지면 이름만 나온다.** 주인이 없으면 `describe`가 답할 수 없어, 정확히 「지금 못 부르는 명령을 어떻게 부르는지 알고 싶은」 상황에서 모양이 사라진다. 동봉은 tombstone에 모양이 함께 남는다(§3-7 조항 4 · 만료 없음).

**④ CI 게이트 — 그대로 못 쓴다.** ★개정★ 구판: "기존 ts-rs 바인딩 sync 게이트가 두 파생물을 **같이** 덮는다. 새 게이트를 만들지 않는다." 실측상 거짓이 된다 — §5 「각 격리 게이트에 미치는 영향」에 근거와 필요한 조치를 적었다.

---

## 3. 인터페이스

★개정★ 구판: "**전부 `engram-dashboard-protocol`에 산다.**" → 현판: **아래 §3-1~§3-6은 신규 도구 crate에 산다.** 도구 crate는 워크스페이스 의존 0이고 명령을 0개 담는다. `AgentCommand`/`AgentEvent`의 additive variant(§3-7)만 `protocol`에 남는다 — 그건 wire 계약이라 원래 그 자리다.

**새 외부 의존:** 링커 수집 crate 1개(§2-2). 비동기 시그니처는 `std::pin::Pin` + `core::future::Future`로 적어 `futures`는 안 들인다.

### 3-1. 계약 표 ★개정★

```rust
// ★삭제★ pub enum OwnerClass { Daemon, Shell, View }   ← 구판. 선언처가 주인이라 불필요
// ★개정★ 구판 주석: "v1 목록엔 Write 만 실린다(§8 가정 B — 미결)".
// 현판: Read 항목도 v1 목록에 실린다(ADR-0135 — §8 가정 B 채택). Read 는 §4-⑥ dedup 면제.
pub enum Effect { Write, Read }

pub struct CommandSpec {
    pub name: &'static str,
    // ★삭제★ pub owner: OwnerClass,
    pub effect: Effect,
    pub since: u32,
    pub summary: &'static str,
    pub args_schema: &'static str,       // JSON Schema 텍스트(매크로 생성)
    pub ok_schema: &'static str,
    pub errors: &'static [ErrorCode],
}

/// 이 바이너리에 링크된 선언 전량(★개정★ 구판 = 단일 `COMMAND_CATALOG` 배열).
pub fn command_specs() -> impl Iterator<Item = &'static CommandSpec>;
pub fn spec_of(name: &str) -> Option<&'static CommandSpec>;

/// ★2판 확정★ 등록이 나르는 단위 — **거처는 이 도구 crate다**(구판·1판은 §3-7 wire 블록에 적어
///   protocol 소속처럼 읽혔다). help = 파생 스키마 항목 하나의 직렬화(§2-2 3 · §2-4 ②).
pub struct CommandDecl { pub name: String, pub help: String }
```

★**`CommandDecl`을 도구 crate에 두는 이유 — protocol이 어차피 이 crate를 본다**★ (2판 확정)
§3-2에서 봉투 자체가 `AgentCommand::Command { envelope }`로 실리기로 확정됐으므로, **`protocol`은 `CommandEnvelope`·`CommandReply`를 보려고 이미 도구 crate를 의존해야 한다.** `CommandDecl` 하나를 protocol로 옮겨도 그 의존은 사라지지 않는다. 따라서 **거부 = 「양쪽에 같은 모양의 타입을 두고 매핑」** — 중복 타입과 변환 코드만 늘고 얻는 것이 없다.

- **귀결: `protocol`의 워크스페이스 의존이 0 → 1이 된다**(도구 crate 하나). §5 protocol 행 · §9 실측에 같은 말로 적었다.
- ★**오해 방지**★ — **도구 crate 자신의 「워크스페이스 의존 0」은 그대로다.** 남이 나를 의존하는 것은 내 의존이 아니다. 화살표는 `protocol → command` 한 방향뿐이고, 도구 crate는 여전히 아무 워크스페이스 crate도 보지 않는다.

### 3-2. 봉투 (대칭 — 방향 필드 없음)

```rust
// ★삭제(2판)★ pub enum FrameKind { Request, Reply, Event }
//   1판은 프레임 종류를 별도 enum 으로 뒀다("구판엔 없던 명시"). variant 가 종류를 가르기로
//   확정돼(아래) 어느 struct 의 필드도 아닌 고아가 됐으므로 삭제한다.

pub struct CommandEnvelope {
    pub name: String,                 // ★겉봉 — 데몬이 읽는다★
    pub request_id: RequestId,        // ADR-0081: 왕복 전 구간 동일. relay_id 없음
    pub owner: OwnerToken,            // ★개정★ 등록으로 얻은 런타임 주인 식별자(선언된 등급 아님)
    pub proto_ver: u32,               // 보낸 쪽 crate 의 CATALOG_VERSION
    pub args: serde_json::Value,      // ★속 — 데몬 불투명★
}

pub struct CommandReply {
    pub request_id: RequestId,
    pub outcome: Result<serde_json::Value, CommandError>,
}
```

**방향 필드를 넣지 않는 이유를 계약으로 못 박는다.** 봉투는 대칭이라 모든 홉에서 같은 형태이고, **어느 연결에 썼는가가 방향이다.** 방향을 필드로 두면 같은 봉투가 두 가지 진실(필드 값 / 실제 연결)을 갖게 되고 둘이 어긋나는 순간 라우팅이 갈린다. (인계 「do-not 5」.)

★개정(2판)★ **프레임 종류는 봉투의 필드가 아니라 variant가 가른다.** 구판(축약 보존): "프레임 종류는 필드가 아니라 **프레임 층**에 있다." 그 표현은 **봉투가 wire의 어디에 실리는지를 비워 둬** §5 net 행(「봉투는 `AgentCommand`의 variant로 실려」)과 갈렸다. **확정: 봉투도 additive variant로 싣는다.**

```rust
// 요청 = AgentCommand 에 additive
Command      { envelope: CommandEnvelope }
// 답장 = AgentEvent 에 additive
CommandReply { reply: CommandReply }
```

이 둘 + 등록 3종 + `CommandList`(§3-7) = **`protocol`에 붙는 additive variant 총 6종**(§5 protocol 행이 같은 수를 적는다). **새 프레임 층·새 소켓·새 채널을 만들지 않으므로 셸 쪽 결론은 그대로다** — 새로 생기는 분기는 **하나뿐**이다. `src-tauri/src/daemon_client/connection.rs:805`의 `Message::Text` 처리에 `Request` 갈래를 더한다. 오늘 그 자리는 `AgentEvent`만 디코드하고(`:1041~1072` 이벤트 분기), `Reply`는 이미 correlation 경로가 있다(ADR-0081 결정 4).

`name`을 겉봉에 두는 것이 ADR-0081의 순수 opaque 봉투로부터의 **유일한 형태 변경**이다. 근거: 이름이 겉봉에 있으면 데몬이 인자를 모른 채로도 **명령 단위 인가·관측**을 할 수 있고, 감추면 그게 전부 불가능해진다(조사 §2 — "명령 이름까지 불투명 봉투 안에 넣은 선례는 없었다"). `args`는 여전히 데몬이 파싱하지 않으므로 ADR-0081 「데몬 opaque 유지」의 본체는 산다. (인계 「do-not 7」.)

### 3-3. 프로세스별 핸들러 표 ★개정★

```rust
pub type CommandFuture = Pin<Box<dyn Future<Output = Result<serde_json::Value, CommandError>> + Send>>;

pub trait CommandHandler: Send + Sync {
    fn call(&self, args: serde_json::Value) -> CommandFuture;
}

pub struct CommandTable { /* HashMap<&'static str, Arc<dyn CommandHandler>> */ }

impl CommandTable {
    pub fn insert(&mut self, name: &'static str, h: Arc<dyn CommandHandler>) -> Result<(), TableError>;
    pub fn get(&self, name: &str) -> Option<&Arc<dyn CommandHandler>>;
    // ★개정(2판)★ 등록 패킷을 그대로 만든다 — 1판: `fn names(&self) -> Vec<&'static str>`(이름만).
    //   등록 단위가 `{이름 + 모양}`으로 바뀌어(§3-7) 이름만으로는 패킷을 못 만든다.
    pub fn decls(&self) -> Vec<CommandDecl>;    // 데몬에 등록할 명단 {name, help}
}
```

★**전역 자동등록 금지 — 이 절의 핵심 개정**★
구판은 "각 프로세스가 자기 표를 채운다"까지만 적고 **어떻게** 채우는지를 비워 뒀고, 그 빈칸을 「자동 수집이 알아서 채운다」로 읽으면 전역 표에 핸들러가 박힌다. 그 형태를 **금지**한다.

- 자동 수집이 다루는 것은 **이름 · 인자 스키마 · 오류 집합까지**다.
- **핸들러 실물은 조립 때 주입한다** — 각 모듈은 `pub fn make_table(deps) -> CommandTable` 하나를 공개하고, 실행 파일이 그것을 부른다. `make_table(manager)`·`make_table(mailbox)` 꼴.
- **명령이 늘어도 조립 코드는 안 바뀐다** — 늘어나는 것은 `declare_commands!` 블록과 `make_table` 안의 한 줄이지 실행 파일이 아니다.
- 금지 근거는 편의가 아니라 **테스트**다 → §7 규칙 T-1(하드 제약).

나머지 규칙:
- `insert`는 **중복 이름에 `Err`**를 낸다 — 조용한 덮어쓰기 금지(조사 §5-6: 선례는 있으나 그 선례가 후회한다). 프론트 `src/commands/registry.ts:35-40`은 지금 warn 후 마지막 승리인데, 이건 HMR 재평가 때문이라 **프론트는 예외로 유지**하고 사유를 코드 주석에 남긴다.
- ★개정★ 구판: "`insert`는 이름이 **`COMMAND_CATALOG`에 없으면** `Err`다." → 현판: **자기 crate의 선언 집합에 없으면 `Err`**다. 전역 카탈로그가 없어졌으므로 대조 상대가 「내 crate가 선언한 것」으로 좁혀진다. 막으려던 것(카탈로그 밖 이름이 두 번째 어휘를 만드는 일)은 그대로 막힌다 — 남의 crate 이름을 내 표에 넣는 것도 이 규칙에 걸린다.
- 화면 쪽 대응물은 이미 있다: `src/commands/registry.ts:29`의 `Map<string, Command>` + `register`(:35) / `run`(:46) / `list`(:67). **새로 만들지 않는다**(CLAUDE.md 「LLM-우선 제어」 — 두 번째 표면 금지).

### 3-4. 전송 seam

```rust
pub trait CommandLink: Send + Sync {
    /// 봉투를 상대 프로세스로 보내고 같은 request_id 의 답을 기다린다.
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>>;
}
```

구현체는 프로세스마다 하나다 — 데몬은 WS 연결, 셸은 `daemon_client`, 화면은 Tauri `invoke`. **`CommandLink`가 교체점**이라 전송 방식이 코드에 안 묶인다(CLAUDE.md 「아키텍처 원칙」).

### 3-5. 라우팅 (전 프로세스·전 홉 동일) ★개정★

```rust
pub async fn route(
    table: &CommandTable,          // 내 표
    roster: &Roster,               // ★추가★ 내가 아는 명부(없는 프로세스는 빈 명부를 넘긴다)
    link: &dyn CommandLink,
    env: CommandEnvelope,
) -> CommandReply;
```

**규칙 3단계:**
1. `table.get(&env.name)` → 있으면 **직접 실행**.
2. 없으면 `roster.owner_of(&env.name)` → 있으면 **그 주인 쪽 연결로 `link.send`**.
3. 둘 다 없으면 **오류**(§4-② — `UNKNOWN_COMMAND` / `OWNER_UNAVAILABLE`을 갈라 낸다).

구판은 2단계였고("있으면 직접, 없으면 보낸다") **보낼 곳이 하나뿐이라는 전제**가 깔려 있었다. 홉이 둘이 되면(§3-8) 그 전제가 깨진다. 3단계는 **특별 케이스 없이** 모든 홉에서 같으므로, 나중에 새 주인(예: 모바일 클라이언트)이 붙어도 배달 코드는 안 바뀌고 **등록이 하나 늘 뿐이다.**

### 3-6. 인바운드 수신기

```rust
pub trait InboundCommands: Send + Sync {
    /// ★즉시 반환해야 한다★ — 적용은 연결 태스크 밖에서 돈다.
    fn on_command(&self, env: CommandEnvelope, reply: ReplySink);
}

pub struct ReplySink { /* oneshot 으로 request_id 에 상관 */ }
```

**연결 태스크 안에서 적용하면 합성 명령이 교착한다.** 실측: `crates/engram-dashboard-daemon/src/connection_core.rs:582`의 `dispatch`는 `crates/engram-dashboard-daemon/src/agent_conn.rs:217` → `crates/engram-dashboard-net/src/ws.rs:553`을 거쳐 **연결당 단일 read 태스크 안에서 인라인으로 `.await`**된다(spawn은 `ws.rs:393`의 연결당 1회뿐). 셸 쪽 `spawn_into`(`src-tauri/src/commands/layout.rs:404`)가 자기 안에서 `DaemonClient::send_command().await`를 부르므로, 이 명령을 연결 태스크에서 인라인으로 기다리면 **자기 답을 자기가 못 꺼낸다** — ADR-0081 결정 3 개정이 잡아낸 self-deadlock 그대로다. `on_command`의 "즉시 반환" 계약이 그 회귀를 형태로 막는다.

★개정에서 강화★ — 이 조항은 구판에도 있었으나 **셸 한 곳**을 겨눴다. 3단 라우팅과 2단 배달이 서면 **중간 홉이 자기 답을 기다리는 경로가 하나 더 생긴다**(데몬 → 셸 → 웹뷰에서 셸이 중간). 따라서 「받은 자리에서 실행하지 않는다」는 **셸·데몬·웹뷰 모두에 적용되는 전역 규칙**이다. (인계 「do-not 6」.)

### 3-7. 등록 wire ★개정★

**구판:** `RegisterCommands` + `ListCommands` variant 2종 · 데몬이 `CommandListEntry { name, owner: OwnerClass, available }`를 낸다.
**현판:** 아래. 바뀐 것은 ㉠ `OwnerClass` 삭제 ㉡ **델타**와 **연결 해제 무효화**를 계약에 명시 ㉢ **셸이 웹뷰 몫을 대신 등록**한다는 조항 ㉣ 데몬이 빌드 목록을 **조회하지 않는다**는 금지 조항 ★2판 추가★ ㉤ 등록 단위가 **이름에서 `{name, help}`로** 바뀐다(ADR-0135).

```rust
// AgentCommand 에 additive 로 붙는 variant
RegisterCommands { owner: OwnerToken, decls: Vec<CommandDecl>, catalog_version: u32, request_id: RequestId }
UpdateCommands   { owner: OwnerToken, added: Vec<CommandDecl>, removed: Vec<String>, request_id: RequestId }
ListCommands     { request_id: RequestId }

// ★개정★ 등록이 나르는 단위 = `CommandDecl { name, help }`. 구판 = `names: Vec<String>` /
//   `added: Vec<String>`(이름만). removed 는 그대로 이름만 — 지울 때 모양은 필요 없다.
//   ★2판 확정★ 이 타입의 **선언은 도구 crate**에 있다(§3-1) — protocol 은 봉투 때문에 이미 그 crate 를
//     의존하므로 여기 두지 않는다. 표에서 목록을 뽑는 것은 `CommandTable::decls()`(§3-3).
//   help = 파생 스키마 항목 하나의 직렬화(§2-2 3 · §2-4 ②). 데몬에겐 불투명 문자열 한 칸이다.

// AgentEvent 에 additive 로 붙는 variant
CommandList { request_id: RequestId, entries: Vec<CommandListEntry> }
// ★개정★ CommandListEntry = { name: String, help: String, available: bool }
//   1판: owner: OwnerClass 삭제 · 2판: help 추가(ADR-0135)
```

**네 조항:**

1. **붙을 때 패킷 하나.** 주인은 인증 직후 자기 선언 **전량**을 `RegisterCommands` 한 방으로 보낸다(★2판★ 구판은 "자기 **이름** 전량" — 이제 `{이름 + 모양}` 전량이다). 이름마다 왕복하지 않는다. 재연결마다 재전송하고 중복 owner는 last-wins — ADR-0081 「RegisterRole 재연결 재전송 + last-wins」와 같은 규칙이다.
2. **셸이 웹뷰 몫을 대신 얹는다.** 웹뷰는 데몬과 직접 연결이 없다(§3-8). 웹뷰가 부팅 시 자기 `registry.ts` 목록을 셸에 알리면, 셸이 자기 이름들과 함께 등록한다. **웹뷰 이름의 주인 토큰은 셸 토큰이다** — 데몬 입장에서 두 층은 구분되지 않고, 구분할 필요도 없다(2단 배달이 셸에서 다시 갈라준다).
3. **바뀌면 델타만.** 웹뷰가 늦게 뜨거나 기능이 켜지고 꺼지면 `UpdateCommands`로 차분만 보낸다. 전량 재전송은 재연결 때만.
4. **연결이 끊기면 그 주인의 이름은 무효화된다.** ADR-0081 「연결 cleanup sweep」과 같은 트리거를 쓴다. ★단 지우지 않고 **비가용 표시(tombstone)로 남긴다**★ — §4-②가 요구하는 두 오류 구분이 여기 달려 있다. ★2판 확정 — **만료를 두지 않는다**(ADR-0135)★: tombstone은 **데몬 수명 동안** 유지되고 TTL·LRU로 지워지지 않는다. 같은 이름이 재등록되면 **last-wins**로 덮는다(조항 1의 owner last-wins와 같은 규칙). `help`도 함께 남으므로 **주인이 꺼져 있어도 모양은 조회된다**(§2-4 ③ 채택 근거). 거부 이유는 §4-②에 적었다.

★개정★ **구판:** "데몬이 나르는 것은 **이름과 가용 여부뿐**이다 — 설명·인자 스키마는 나르지 않는다(§2-4 ③의 미확인 항목)."
**현판:** 데몬은 **모양도 함께 나른다.** 단 `help`를 **불투명 문자열로만** 다룬다 — 받아서 명부에 넣고 조회에 그대로 돌려줄 뿐이다.

★**하드 제약 — 데몬은 help를 열어보지 않는다**★ (ADR-0135)
- 데몬 코드가 `help`를 **파싱·검증하거나 그 내용으로 분기하면 위반**이다. JSON으로 파싱해 보는 것도, 스키마 유효성을 검사하는 것도 데몬 몫이 아니다.
- **형태로 막는다:** 자료형을 `String` 하나로 고정한다. 전용 struct나 `serde_json::Value`로 올리면 데몬 코드가 들여다볼 손잡이가 생기므로 **자료형을 올리지 않는 것 자체가 게이트**다(§7 seam 표가 바이트 보존을 단언한다).
- **「데몬은 명령의 뜻을 배우지 않는다」는 그대로 산다. ★단 근거가 바뀌었다★** — 구판 근거는 「안 나른다」였고, 현판 근거는 **「나르되 안 열어본다」**다. 「데몬은 클라이언트 셸을 모른다」(C1)도 같은 근거 위에 선다: 데몬이 컴파일에 넣는 UI 타입은 여전히 0이고, 늘어난 것은 **의미를 모르는 문자열 한 칸**뿐이다.

★**금지 — 데몬은 빌드 목록을 조회하지 않는다**★ (뒤집힌 판단 3)
구판은 데몬이 `COMMAND_CATALOG`를 갖고 명부와 대조하는 그림이었다. 폐기 근거 둘:
- **결합.** 데몬이 전량 목록을 컴파일에 넣으면 `tab.create`·`theme.set` 같은 **셸·화면 이름을 정적으로 알게 된다.** 뒤집힌 판단 1이 피하려던 바로 그 결합이 다른 문으로 들어온다.
- **거짓 가용성.** 앱이 안 떠 있어도 데몬이 "그 명령 있다"고 답하게 된다. 명부가 런타임 등록만으로 차야 「지금 부를 수 있는가」가 사실이 된다.

### 3-8. 2단 배달 ★신설★

**데몬은 웹뷰와 직접 연결이 없다.** `theme.set`은 `데몬 → 셸 → 웹뷰`로 간다.

```
에이전트 → (WS) → 데몬 → (WS) → 셸 → (invoke/Channel) → 웹뷰
             ①3단계      ②3단계        ③내 표에 있음 → 실행
```

- **각 홉이 §3-5의 같은 3단계를 돌릴 뿐이다.** 홉마다 다른 규칙을 두지 않는다.
- 홉 ①에서 데몬은 `theme.set`의 주인 토큰 = 셸 토큰임을 명부에서 읽고 셸로 보낸다(데몬은 그게 웹뷰 것인지 모른다 — 알 필요도 없다).
- 홉 ②에서 셸은 자기 표에 없음을 확인하고 자기 명부(웹뷰 등록분)를 보고 웹뷰로 내린다.
- `request_id`는 전 구간 동일(ADR-0081 결정 4). 홉마다 새 id를 만들지 않는다.
- **중간 홉은 반드시 연결 태스크 밖에서 넘긴다**(§3-6) — 셸이 자기 read 루프 안에서 웹뷰 답을 기다리면 그 루프가 답을 못 읽는다.

---

## 4. 오류 · 진화 계약

조사 §9가 "이게 비면 어느 안도 완성 아키텍처가 아니다"라고 지목한 일곱 항목 + 개정에서 하나 추가(⑧). 각 항목은 **규칙**과 **호출자가 보는 것**을 함께 적는다.

### ① 프로토콜 버전 — 번호가 둘이고 성격이 다르다

| 번호 | 무엇을 잰다 | 불일치 시 | 실물 |
|---|---|---|---|
| `PROTOCOL_VERSION` | 프레임·핸드셰이크 형태 | **연결 자체가 안 선다**(정확 일치 요구) | `crates/engram-dashboard-protocol/src/lib.rs:58` = `3` · 핸드셰이크 `AuthFrame::Auth{token, protocol_version}`(`crates/engram-dashboard-net/src/auth.rs:33-36` — enum 선언 `:33`, 필드 `:35-36`) · 응답 `AgentEvent::Hello{protocol_version,...}`(`messages.rs:220`) |
| `CATALOG_VERSION` | 명령 어휘의 세대 | **연결은 서고 개별 명령만 실패한다**(스큐 허용) | 신설. ★개정★ 구판 = 전역 상수 1개 → 현판 = **선언 crate마다 1개**(§2-2 4) |

★**명령을 하나 더한다고 `PROTOCOL_VERSION`을 올리지 않는다.**★ 올리면 additive 확장 하나가 모든 피어의 연결을 끊는다. 봉투의 `proto_ver`는 보낸 쪽 crate의 `CATALOG_VERSION`이고 **받는 쪽은 이 값만으로 거절하지 않는다** — 거절 판정은 이름 하나 단위(②)로 내린다. 근거: 최근접 피어(WezTerm)도 호환 불가 변경에만 코덱 버전을 올린다(조사 §9).

★개정으로 생긴 주의★ — 세대 번호가 crate마다면 **하나의 봉투가 어느 세대인지는 「보낸 crate 기준」**이다. 받는 쪽이 자기 번호와 비교해 의미를 부여하면 틀린다. 이 값은 **로그·진단용**이고 분기 재료가 아니다.

- **호출자가 보는 것:** 프레임 버전이 다르면 `PROTOCOL_MISMATCH`로 접속 실패(기존 경로 그대로). 카탈로그가 뒤처졌으면 접속은 되고 특정 명령만 ②의 오류.

### ② 미지 명령 vs 주인 부재 — 두 오류를 반드시 가른다 ★개정에서 강화★

인계 「오류 2종을 구분한다」가 이 절의 정본이다. **부르는 쪽의 대응이 갈리므로 합쳐서는 안 된다.**

| 판정 | 뜻 | 코드 | 호출자 대응 |
|---|---|---|---|
| 명부에 그 이름 자체가 없다 | 오타 · 구버전 클라이언트 · 오래된 빌드 | `UNKNOWN_COMMAND` | 재시도 무의미. 이름을 다시 발견해야 한다 |
| 이름은 아는데 주인이 지금 없다 | 앱이 안 떠 있음 · 주인이 재기동 중 | `OWNER_UNAVAILABLE` | **나중에 다시** 부르면 된다 |
| 봉투를 받은 주인의 표에 없다 | 명부와 실제가 어긋남(주인이 옛 빌드로 재기동) | `UNKNOWN_COMMAND` | 위와 같음 |

★**데몬이 둘을 어떻게 구분하나 — 개정이 만든 문제와 그 답**★
명부가 **런타임 등록만으로** 차면(뒤집힌 판단 3), 주인이 끊긴 순간 그 이름들도 사라져 데몬은 「모르는 이름」과 「주인 없는 이름」을 구분할 수 없게 된다. 답: **§3-7 조항 4의 tombstone** — 연결이 끊기면 이름을 지우지 않고 `available: false`로 남긴다.

- **대가(숨기지 않음):** **데몬이 한 번도 본 적 없는 주인의 이름은 `UNKNOWN_COMMAND`가 된다.** 데몬만 떠 있고 앱이 한 번도 붙은 적 없는 상태에서 `tab.create`를 부르면 그렇다. 정확한 답(`OWNER_UNAVAILABLE`)을 내려면 데몬이 전량 목록을 알아야 하는데 그건 뒤집힌 판단 3이 금지한 것이다. **정확성보다 결합 회피를 택한 자리이고, 그 선택을 여기 적어둔다.**
- ★**개정 — 닫혔다(ADR-0135)**★ **tombstone에 만료를 두지 않는다.** 데몬 수명 동안 유지하고, 재등록 시 last-wins로 덮는다. 구판(축약 보존): "보존 기간 **미확정**(데몬 수명 전체 / TTL) — ADR-0081 「dedup store 바운드(TTL·LRU)」와 같은 축이라 그 형태를 따르는 것이 자연스러우나 여기서 확정하지 않는다."
  - **TTL을 거부한 이유:** 시간 만료를 두면 **같은 질문에 시점에 따라 다른 오류가 나간다.** 앱이 한 번 붙었던 `tab.create`가 만료 전에는 `OWNER_UNAVAILABLE`, 만료 후에는 `UNKNOWN_COMMAND`가 되고, 그러면 위 표의 두 판정이 **호출자 입장에서 구분 불가능해진다**(재시도가 의미 있는지 없는지가 시계에 달린다). 두 오류를 가르려고 둔 장치를 시간이 도로 부수는 셈이라, dedup store와 같은 축으로 보던 1판의 읽기 자체가 틀렸다 — dedup는 *진행 중 왕복*이라 유한하지만 명부는 *어휘*라 유한하지 않다(아래 ★개정 주의★와 같은 구분).
  - **바운드 대신:** 이름 집합은 빌드가 아는 어휘 크기로 제한되고 재등록이 last-wins라 무한 성장하지 않는다. 늘어나는 것은 **주인이 실제로 등록한 적 있는 이름 수**뿐이다.

조용한 무시·다른 명령으로의 폴백은 없다. 조사 §3-④가 정리한 세 형태 중 우리에 맞는 것은 ①(프로토콜 오류)뿐이다.

★**닫힌 열거형 금지**★ — 디코더는 `name`을 `String`으로 받는다. 생성된 TS도 명령 이름을 union 타입으로 좁히지 않는다. 조사 §3-① 「경고 1」의 LSP 사고(산문 규격은 미지 값 허용을 요구했는데 메타모델이 표식을 빠뜨려 엄격한 생성기가 닫힌 열거형을 만들고 클라이언트를 깨뜨림)를 그대로 밟지 않기 위해서다.

- **호출자가 보는 것:** `{ code: "UNKNOWN_COMMAND" | "OWNER_UNAVAILABLE", message: "<name>", retry: … }`. 앞은 `retry: "never"`, 뒤는 `retry: "after-condition"`.

### ③ 필드 추가 규칙 (additive)

| 해도 되는 것 | 하면 안 되는 것 |
|---|---|
| 인자 struct에 **기본값 있는 선택 필드** 추가 | 필드 이름 변경 |
| 반환 struct에 필드 추가 | 필드 타입 변경 |
| 새 이름 추가 | 필드 제거 |
| 오류 코드 집합에 코드 추가 | 기존 이름의 **뜻** 변경 |

기계 강제 형태: 인자 struct의 신규 필드는 `#[serde(default)]`를 단다. 어떤 struct에도 `deny_unknown_fields`를 달지 않는다 — 새 호출자가 보낸 모르는 필드가 옛 주인을 깨면 additive가 성립하지 않는다.

★**경계 규칙**★: 옛 주인은 모르는 필드를 무시하고 **옛 의미로 실행한다.** 따라서 "필드를 더해 동작 의미를 바꾸는" 확장은 필드 추가가 아니라 **새 이름**이어야 한다. 이 선을 넘으면 호출자는 성공 응답을 받고도 의도와 다른 일이 일어난다.

명령 제거: 선언에서 지우지 않고 `#[deprecated_since(N)]`를 단다. 파생 JSON 스키마는 그 항목을 광고에서 빼지만 핸들러는 최소 한 세대(그 crate의 `CATALOG_VERSION` +1) 동안 계속 답한다.

★개정에서 추가된 규칙★ — **이름을 다른 crate로 옮기는 것은 additive가 아니다.** 선언 위치가 주인이므로 이사는 주인 교체이고, 그 이름의 등록 주체가 바뀌면 명부가 재편된다. 옮겨야 하면 **새 이름으로 선언하고 옛 이름을 deprecate**한다. (구판에는 이 위험 자체가 없었다 — 선언이 한 곳이었으므로. 개정이 만든 새 실패 모드다.)

- **호출자가 보는 것:** 확장은 조용히 성공한다. 옛 필드만 보낸 호출자는 계속 돈다. deprecated 명령은 목록에서 사라지지만 이름으로 부르면 여전히 듣는다.

### ④ 소유자 부재 — 확실성을 코드에 인코딩한다

| 상황 | 코드 | 확실성 | `retry` |
|---|---|---|---|
| 명부에 있으나 주인이 지금 비가용(tombstone) | `OWNER_UNAVAILABLE` | **미적용 확정** | `after-condition`(주인 기동) |
| 주인이 등록돼 있었는데 왕복 중 연결이 끊겼다 | `OUTCOME_UNKNOWN` | **불명** | `same-request-id` |
| 마감시각 초과 | `TIMEOUT` | **불명** | `same-request-id` |

★`retry` 플래그가 "불명 상태를 안전하게 재실행해도 된다"를 함의해선 안 된다★ — 안전한 무조건 재시도는 **미적용 확정**일 때뿐이다. 이 인코딩은 S17 TRD가 이미 세운 것(`docs/process/S17-llm-control-surface/spec/trd.md:56`, `:72`)이고 여기서 `APP_OFFLINE`을 주인 일반으로 넓힌 것뿐이다.

수명 관리는 ADR-0081 「relay 라우팅 표 수명 바운드 + 연결 cleanup sweep」을 그대로 쓴다 — 엔트리 `{request_id → 원 ConnId, deadline}`, evict 트리거 = 응답 수신 / 어느 쪽 연결이든 cleanup 시 그 ConnId 키 sweep / 타임아웃. **sweep이 없으면 ④의 "불명"이 "영원한 무응답"이 된다.**

★개정 주의★ — **sweep과 tombstone은 다른 표다.** sweep은 *진행 중 왕복*을 정리하고(엔트리 삭제), tombstone은 *이름 명부*를 남긴다(플래그만 내림). 둘을 한 표로 합치면 연결 정리가 이름까지 지워 ②의 구분이 무너진다.

- **호출자가 보는 것:** 실패의 종류가 아니라 **재실행해도 되는지**가 응답에 실려 온다.

### ⑤ 취소 · 진행 보고 — v1 미도입, 자리만 예약

기제를 만들지 않는다. 근거: 지금 장시간이라 부를 만한 명령이 없다 — `agent.spawn`은 프로세스를 띄운 시점에 답하지 에이전트가 준비될 때까지 기다리지 않는다.

예약 형태만 못 박는다.
- **취소 핸들 = `request_id`.** 별도 토큰을 만들지 않는다(ADR-0081의 단일 request_id 원칙). 열 때 `command.cancel { request_id }`.
- **진행은 답장이 아니라 이벤트다.** 하나의 `request_id`에 대한 `CommandReply`는 **정확히 하나**다. 중간 상태가 필요해지면 `AgentEvent` 쪽에 variant를 더한다. 이 선을 지금 그어두지 않으면 나중에 "답장이 여러 번 온다"가 되어 상관 로직이 깨진다.
- ★개정에서 추가★ — 2단 배달(§3-8)에서도 **답장은 하나**다. 중간 홉이 자기 답을 하나 더 끼워 넣지 않는다.

- **호출자가 보는 것:** 진행 보고가 **없다는 것이 계약이다.** 오래 걸릴 수 있는 호출은 반드시 마감시각(⑥)에 기대야 하고, 중간 상태를 물어보려면 **별도 조회 명령을 부른다.** ★개정★ 구판은 여기에 "그리고 조회는 §8 가정 B로 v1에 없다 — 이 조합의 대가를 여기 적어둔다"를 달았다. 가정 B가 채택되면서(ADR-0135) **그 대가가 사라졌다** — 진행 보고는 여전히 없지만 상태를 물어볼 경로는 v1 안에 있다.

### ⑥ 타임아웃 · 재시도

- 마감시각은 **호출자가 정한다**(기본 10초 — S17 TRD `trd.md:72`의 `--timeout` 기본값 승계). 데몬 라우팅 표 엔트리가 같은 `deadline`을 들고 있어 호출자가 사라져도 표가 안 샌다.
- 초과 = `TIMEOUT`, 확실성 **불명**.
- **안전한 재시도는 같은 `request_id`로만 한다.** 새 id로 재시도하면 at-least-once가 되어 같은 조작이 두 번 적용될 수 있다.
- dedup 저장소는 S17 TRD가 이미 설계한 것을 그대로 쓴다(`trd.md:50-53`, `:72`): 완료분 = 캐시된 원 결과 재생 · in-flight 중복 = 같은 pending에 coalesce · 같은 id + 다른 페이로드 = `REQUEST_ID_CONFLICT` · 재시도 창 `retryWindowMs` 기본 300000(5분). 창 밖 같은 id = 신규 취급.
- ★쓰기 명령의 dedup 엔트리는 **성공 응답을 받은 뒤에만** 커밋한다★ — 사전 캐싱하면 타임아웃 때 거짓 성공이 캐시된다(ADR-0081 「dedup는 UiResult(적용 후)에만 커밋」).
- ★개정에서 추가★ — **dedup은 한 곳에서만 한다.** 2단 배달에서 홉마다 dedup을 걸면 같은 id가 두 표에 앉아 만료 시점이 갈린다. 소유 지점 = **최초 수신 데몬**(ADR-0081 relay 표와 같은 자리).
- ★2판 추가(ADR-0135 — 가정 B 채택의 귀결)★ **읽기 명령(`Effect::Read`)은 dedup 대상에서 면제한다.** 멱등이라 두 번 실행돼도 상태가 안 바뀌므로 중복 방지가 지킬 것이 없고, 면제하면 조회가 5분 재시도 창의 표를 채우지 않는다. dedup 커밋 규칙(위 항목)은 **쓰기에만** 적용된다. 구판에는 이 규칙이 없었다 — 전 명령이 dedup 대상이었고, 그건 읽기가 v1에 없다는 전제 위였다(§8 가정 B 「뒤집으면 바뀌는 절」이 예고한 항목).

- **호출자가 보는 것:** 같은 id로 다시 부르면 두 번 적용되지 않는다. 5분이 지나면 그 보장이 끝나고, 그 사실이 오류 본문의 `retryWindowMs`로 온다.

### ⑦ 타입드 오류 모델

지금은 문자열뿐이다 — 데몬은 실패를 `AgentEvent::Error { request_id, message }`(`messages.rs:334`)로만 답하고, CLI가 그 문자열을 패턴매칭해 코드를 **합성**한다(S17 TRD `trd.md:69`가 "문자열 계약은 취약"이라고 자인). 이 설계가 그것을 계약으로 승격한다.

```rust
pub struct CommandError {
    pub code: ErrorCode,      // 닫힌 집합(아래) — 단 wire 는 문자열
    pub message: String,      // 사람·로그용. 기계 분기 금지
    pub retry: RetryMode,     // never | same-request-id | after-condition
}
```

초기 집합(전송·라우팅 계층 + 공통):
`INVALID_ARGUMENT` · `UNKNOWN_COMMAND` · `OWNER_UNAVAILABLE` · `OUTCOME_UNKNOWN` · `TIMEOUT` · `REQUEST_ID_CONFLICT` · `ALREADY_APPLIED` · `AUTH_FAILED` · `PROTOCOL_MISMATCH` · `NOT_FOUND` · `CONFLICT` · `UNSUPPORTED` · `INTERNAL`.

- **명령마다 자기 부분집합을 선언한다**(§2-2 `errors [...]`) → 파생 JSON 스키마가 그것을 광고하므로 호출자가 부르기 전에 실패 모양을 안다.
- ★wire의 `code`는 문자열이고 디코더는 미지 코드를 받아들인다★ — 모르는 코드는 `INTERNAL` + `retry: never`로 낮춰 다룬다. 여기서도 닫힌 열거형을 만들지 않는다(②와 같은 이유).
- `message`로 기계 분기하지 않는다. 지금 CLI가 하는 문자열 패턴매칭은 Step 2에서 걷어낸다.
- ★개정에서 추가★ — 오류 코드 enum은 **도구 crate**에 산다(§3). 선언이 흩어져도 오류 어휘는 하나여야 홉을 건너며 뜻이 유지된다.

- **호출자가 보는 것:** 실패가 코드 + 재시도 지시로 온다. 문자열은 사람이 읽는 자리로만 남는다.

### ⑧ 인가는 호출마다 ★신설★

**등록 시 한 번 검사하고 이후 세션 식별자로만 라우팅하지 않는다.** 봉투마다 `name`을 보고 그 호출을 인가한다.

- **근거(출처 있음):** 조사 초판은 "권한 검사는 등록 시 한 번이 흔하다"고 적었으나 적대 리뷰가 **반증**했다 — 버스 데몬은 메시지가 연결 사이로 라우팅될 때 정책을 확인하고, 클러스터 API는 **요청마다** 인가하며, RPC 인터셉터는 본질적으로 per-call이다(조사 §2 「권한 검사는 호출마다」 항목 + §9 반증 표 3행).
- **이 설계가 그것을 가능케 하는 이유:** `name`이 겉봉에 있기 때문이다(§3-2). 이름을 속에 감췄으면 명령 단위 인가가 원천 불가능하다.
- **범위:** v1은 「어떤 자격이 어떤 명령을 부를 수 있나」의 정책 내용을 정하지 않는다 — 그건 ADR-0086(least-privilege) 계열의 별건이고 **보안 정책 판단이라 담당 결정 경로를 탄다.** v1이 확정하는 것은 **검사 지점이 호출마다라는 형태**뿐이다.

### ⑨ 릴리즈 프로필의 패닉 정책 ★신설 — 사용자 결정 기록★

**릴리즈 프로필의 `panic = "abort"`를 유지한다**(루트 `Cargo.toml:24-30`). 사용자 결정이고, 이 TRD는 재론하지 않는다.

- **근거:** 패닉이 난 뒤의 프로세스 상태는 믿을 수 없다. 미심쩍은 상태로 살려 두고 이어 가는 것보다 **죽고 다시 오는 것**이 잃는 게 적다 — 세션 복원이 그 값을 치러 준다(CLAUDE.md 「세션 복원」 · ADR-0008/0082).
- ★**귀결 — 릴리즈 빌드에서는 패닉 그물이 서지 않는다. 그것이 의도다.**★ 패닉 그물 둘 — `blocking_handler`가 두르는 `guard_panic`(`crates/engram-dashboard-command/src/route.rs:20`, 호출부 `table.rs:178`)과 `route()`가 두르는 `CatchUnwind`(`route.rs:60`) — 은 **개발·테스트 빌드에서만** 실효가 있다. 릴리즈에서는 핸들러 패닉이 오류 답장이 아니라 **프로세스 종료**가 된다. 그물을 「릴리즈에서도 답장 하나를 보장한다」로 읽지 말 것 — §4-⑤의 「한 `request_id`에 답장 하나」는 그 빌드에서 그물이 아니라 **호출자의 마감시각**(⑥)이 지킨다.
- **그래서 규약이 하나 선다: 명령 핸들러는 터져서 죽지 않는다 — 오류를 값으로 돌려준다.** 그물은 실수를 덮는 안전망이지 오류 경로가 아니다. 릴리즈에서 그물이 없어지므로, 패닉을 오류 전달 수단으로 쓰면 그 빌드에서 **에이전트 하나의 잘못된 인자가 데몬을 내린다.** 리뷰 체크리스트가 이 항목을 본다(`.claude/skill-bindings/review.md` 「code 단계 게이트」).

---

## 5. 모듈 경계 ★개정 — 표 전면 교체★

**구판:** 카탈로그·봉투·표·라우팅이 전부 `engram-dashboard-protocol`. `core`는 "v1엔 변경 없음".
**현판:** 아래.

| crate / 폴더 | 이 설계가 넣는 것 |
|---|---|
| **`engram-dashboard-command`(신규)** — ★개정★ 구판 "이름 잠정" → 이름 확정(ADR-0134) | 선언 매크로 · `CommandSpec`/`command_specs()` · `CommandEnvelope`/`CommandReply`/`CommandError`/`ErrorCode` · ★2판★ **`CommandDecl`**(§3-1 — 등록 단위) · `CommandTable`/`CommandHandler`/`CommandLink`/`InboundCommands`/`Roster` · `route()`. **워크스페이스 의존 0 · 명령 0개.** ★개정(Step 1 실측 2026-08-14)★ 외부 의존은 **넷**(`serde`·`serde_json`·`inventory`·`uuid`)이다. 구판(축약 보존): "외부 의존 = serde + 링커 수집 crate" — 둘로 줄여 적었다. 회계 전체는 아래 게이트 표의 「코어 워크스페이스 의존 0」 칸에 모았다. ★오해 방지★ 아래 protocol 행이 **이 crate를 의존한다** — 그건 protocol의 의존이지 **이 crate의 의존이 아니다.** 화살표는 `protocol → command` 한 방향뿐이라 「의존 0」은 깨지지 않는다 |
| `engram-dashboard-core` | ★개정★ 구판 "변경 없음" → **`agent.*` 선언 + `make_table(manager)`.** 그리고 **이 crate의 첫 워크스페이스 의존**이 생긴다(오늘 0개 — 실측). 받는 것은 도구 crate 하나뿐이고 `protocol`은 여전히 안 본다 |
| `engram-dashboard-daemon` | `mail.*` 선언 + `make_table` · 주인 명부(토큰 → 이름, tombstone 포함 — ★2판★ 각 이름의 `help` 문자열을 **불투명하게** 보관한다. 파싱·검증·분기 금지, 자료형은 `String` 고정: §3-7 하드 제약) · 라우팅 표(ADR-0081 형태) · `connection_core.rs:582` `dispatch`(sink 인자 `:586`)에 새 variant arm |
| `src-tauri` | `window`/`tab`/`slot` 선언 + `make_table` · **ADR-0081이 요구한 공유 적용 서비스**(오늘 없음) · `daemon_client` 인바운드 수신기(오늘 없음 — `connection.rs:805`에 `Request` 갈래 추가) · 웹뷰 몫 대리 등록 |
| `src/commands/` | `theme`/`chat` 선언(TypeScript) + 셸에 자기 목록 통지 · ★2판 개정★ **`registry.ts`는 additive로 한 칸 넓힌다**(`Command`에 `help: string` · `list()` 반환에 그 필드 — §6 Step 4). 구판(축약 보존): "기존 `registry.ts` **그대로**" — 그러면 `help`가 웹뷰에서 나갈 길이 없다 |
| `engram-dashboard-protocol` | ★개정★ 구판 "카탈로그 전부" → **`AgentCommand`/`AgentEvent`의 additive variant 6종만.** 내역 = 등록 3종(`RegisterCommands`·`UpdateCommands`·`ListCommands`) + `AgentEvent::CommandList`(§3-7) + **봉투 2종**(`AgentCommand::Command`·`AgentEvent::CommandReply` — §3-2). ★2판 정정★ 1판은 「4종」으로 적어 봉투 2종을 빠뜨렸고, 그래서 아래 net 행(「봉투는 variant로 실려」)과 갈렸다. 명령 선언은 여전히 0개. ★**2판 귀결 — 이 crate의 워크스페이스 의존이 0 → 1이 된다**★: 위 6종이 `CommandEnvelope`·`CommandReply`·`CommandDecl`을 실으므로 **도구 crate 하나를 의존한다**(§3-1 — 그래서 `CommandDecl`을 protocol로 되옮겨도 의존이 안 사라져 「양쪽에 두고 매핑」을 거부했다). 실측 기준선은 오늘 0(§9) |
| `engram-dashboard-net` | ★**변경 없음**★ |
| `engram-dashboard-messaging` | 변경 없음. ★단 게이트 하나가 새 crate를 못 본다 — 아래★ |

### 각 격리 게이트에 미치는 영향 ★개정★

| 게이트 (CLAUDE.md · `.github/workflows/ci.yml`) | 영향 |
|---|---|
| 코어 tauri import 0줄 (`ci.yml:254`) | **없음.** 도구 crate에 tauri가 없다. 셸·화면 명령의 *본문*은 코어 밖이고 코어가 참조하지 않는다. (ADR-0003) |
| **코어 워크스페이스 의존 0** | ★**0 → 1로 바뀐다.**★ 실측: `crates/engram-dashboard-core/Cargo.toml`에 `path =` 0줄(2026-08-13). 도구 crate 하나가 들어온다. **이 수치를 지키는 CI 게이트는 오늘 없다**(있는 것은 tauri import 게이트뿐) — 즉 게이트가 깨지지는 않지만, 「코어는 아무것도 안 본다」는 문장이 「코어는 도구 하나만 본다」로 바뀐다. 이 완화가 개정의 대가이고, `protocol`을 보게 하는 것(=wire 타입까지 유입)보다 작다는 것이 도구 crate를 판 이유다.<br>★**개정(Step 1 실측 2026-08-14) — 대가는 이 한 줄이 아니었다**★ 구판(축약 보존): 이 칸은 「0 → 1」만 적고 "이 완화가 개정의 대가"로 닫았다. 실측하니 셋이 더 있다. ㉠ **`core`가 `ts-rs`를 `[dependencies]`로 받는다** — 선언 매크로가 `TS` derive를 달아야 해서이고, dev가 아니라 **production 의존**이다(`crates/engram-dashboard-core/Cargo.toml`). ㉡ `command`와 그것이 데려온 `inventory`는 `core`를 타고 **데몬·셸 릴리즈 바이너리에 함께 링크된다**(`cargo tree -i`로 확인 — `core` → `daemon`/`net`/`discovery`/`src-tauri`). ㉢ 외부 의존 수가 표 첫 행의 「serde + 링커 수집 crate」보다 둘 많다(위 정정). **단 `ts-rs`는 이번에 처음 들어온 것이 아니다** — `protocol`과 `src-tauri`가 이미 `ts-rs = "10"`을 production으로 쓰고 있어 릴리즈 그래프에 **새로 든 crate는 `inventory` 하나**다(`Cargo.lock` 신규 패키지 = `inventory` + 워크스페이스 멤버 자신, 둘뿐 — 실측). 즉 늘어난 것은 **바이너리 크기·컴파일 시간이 아니라 의존 간선**이 주고, `core`가 이제 TS 생성 도구를 production 그래프에 안고 있다는 사실이 남는다 |
| 메시징 커널 워크스페이스 crate 참조 0줄 (`ci.yml:269`) | ★**게이트에 구멍이 생긴다.**★ 정규식이 `engram_dashboard_(core\|daemon\|protocol\|discovery)`라 **신규 crate 이름이 알파벳에 없다.** 메시징이 도구 crate를 쓰게 돼도 게이트가 안 잡는다. **조치: 정규식에 새 이름을 더한다**(ADR-0110 불변식을 지키려면 필수). 메시징엔 net 같은 의존 상한(`cargo tree`) 게이트가 없어 정규식이 유일한 벽이다 |
| net 소스 참조 0줄 (`ci.yml:340`, `daemon\|messaging\|discovery`) | **정규식은 같은 구멍이 있으나 실질 영향 없음** — net의 의존 상한 게이트(아래)가 새 간선을 잡는다 |
| **net — core 심볼 allowlist 정확히 2줄** (`ci.yml:355`) | ★**바뀌지 않는다.**★ 허용된 둘은 `agent::platform::pid_alive_with_start_time`·`current_process_start_time`(portfile stale 판정 전용). 이 설계는 core 심볼을 net에서 새로 쓰지 않는다 |
| **net — 직접 워크스페이스 의존 정확히 3줄** (`ci.yml:379`) | ★**바뀌지 않는다 — 단 근거가 바뀐다.**★ 구판 근거: "카탈로그가 사는 곳이 **이미 그 3에 든 protocol**이라 새 의존이 생기지 않는다." 현판 근거: **net은 명령을 선언하지도 실행하지도 않아 도구 crate를 볼 일이 없다.** 상한 3(자기 + core + protocol)은 그대로다. ★2판 확인★ `protocol`이 도구 crate를 의존하게 되지만(위 §5 protocol 행) **이 게이트는 `--depth 1`이라 net의 *직접* 의존만 센다**(`ci.yml`의 `cargo tree --locked -p engram-dashboard-net --depth 1 …` — 실측 확인 2026-08-13). 도구 crate는 net에게 **깊이 2**라 3줄에 안 잡힌다 |
| net — auth 어휘 재유입 금지 (`ci.yml:400`) | **없음.** 봉투는 `AgentCommand::Command`/`AgentEvent::CommandReply` variant로 실려(§3-2) net에겐 **불투명 텍스트 프레임**이다 — `ws.rs:553`이 `handler.on_text(...)`로 넘길 뿐 안을 안 본다 |
| **생성물 sync 게이트** (`ci.yml:142`) | ★**그대로 못 쓴다 — 구판의 「새 게이트 0」이 여기서 깨진다.**★ 근거 둘: ㉠ 게이트가 보는 경로가 `crates/engram-dashboard-protocol/bindings/` **하나**다. `core/bindings/`(신설)와 `src-tauri/bindings/`(이미 8개 존재)는 **오늘 아무도 안 본다.** ㉡ 워크스페이스 테스트는 `--exclude engram-dashboard`(`ci.yml:96-99`)라 **src-tauri의 ts-rs export가 CI에서 돌지 않는다.** 즉 셸 소유 선언은 기존 파생 게이트에 그냥 얹히지 않는다. ★개정(Step 1 실측 2026-08-14)★ **㉠의 절반은 닫혔다** — Step 1이 이 게이트를 `core/bindings/`까지 넓히고 `git add -N -f`로 신규 미커밋 파일도 잡게 했다(위 앵커가 그 게이트다). 남은 것은 `src-tauri/bindings/`이고 ㉡은 그대로다. ★실측으로 닫힘(2026-08-13)★ — 아래 문단: `src-tauri/bindings/`는 **오늘 실질적으로 손으로 관리되는 생성물**이다 |

★**net 게이트 둘이 안 바뀌는 이유 한 줄**★: **이 설계는 네트워크 행에 아무 개념도 추가하지 않는다.** 명령 어휘는 선언처(core·daemon·src-tauri·웹뷰)에서 시작해 거기서 끝나고, net은 그 사이를 나르는 바이트 통로로만 참여한다(ADR-0129의 계층 분리 그대로).

★**실측으로 닫힘(2026-08-13) — 파생 게이트를 어떻게 되살릴지**★

**구판(축약 보존):** "★미확인★ — `src-tauri/bindings/` 8개 파일이 **오늘 어떤 절차로 생성·갱신되는지** 확인하지 못했다. 이 경로가 서지 않으면 셸 소유 명령의 TS 파생은 **손으로 관리되는 생성물**이 된다. Step 3 착수 전에 실측이 필요하다." → **실측했다. 답은 「절차가 없다」다** — 추정이 아니라 측정 결과이며, 구판의 우려가 그대로 사실로 확인됐다.

**판정: 실질적으로 손으로 관리되는 생성물이다.** 재생성 메커니즘은 **존재하지만 실행 경로가 하나도 없다.**

- **선언은 있다** — `src-tauri/src/layout/types.rs`의 `#[ts(export)]` 6개 타입(15·31·59·78·91·100줄), 전이 의존까지 합쳐 8개 파일.
- **메커니즘이 암묵적이다.** protocol의 `crates/engram-dashboard-protocol/tests/ts_export.rs` 같은 **명시적 export 테스트가 `src-tauri`엔 없다**(통합 테스트 디렉터리 자체가 없다). `src-tauri/src/layout/mod.rs:29-32`가 자인한다 — 수동 `export_all_to` 미러는 derive와 "이중출처"라 제거했고(주석의 FIX-2), 대신 ts-rs derive가 타입마다 자동 생성하는 `export_bindings_<타입>` 테스트에 의존한다. 그 테스트들은 **`src-tauri` 패키지의 lib 테스트 타깃**에 속한다.
- **그 타깃이 로컬에서 안 돈다.** `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`로 기동 자체가 죽는다(`src-tauri/build.rs` 상단 주석 · `docs/testing-strategy.md:75`·`:99`). export 테스트도 그 안에 있어 함께 안 돈다.
- **CI에서도 안 돈다.** 워크스페이스 테스트가 그 패키지를 제외하고(`ci.yml:85`·`:96-99`), `cargo build`(`ci.yml:151-153`)는 빌드만 한다. **워크플로 전체에서 `src-tauri/bindings`를 참조하는 줄이 0건이다.**
- **다른 경로도 없다.** npm 스크립트·`scripts/`·cargo alias·git hook 어디에도 재생성 경로가 없다.
- **이력이 방증한다(★2판 톤 하향★).** 신규 바인딩 파일(`SlotContent.ts`)이 **git 미추적으로 남아 리뷰에서 지적됐고 그 뒤 add됐다**는 기록이 `docs/process/step-log.md:732`에 있다. 구판(축약 보존): "**사람이 수동으로 add**한 사례가 **실측**으로 남아 있다" — 원문은 Codex FIX 「신규 바인딩 git 미추적」 → 「FIX 반영(SlotContent.ts add)」일 뿐 **누가 어떤 경로로 add했는지는 적혀 있지 않아** 라벨이 근거보다 셌다. **판정은 이 줄에 기대지 않는다** — 위 네 항목(선언 있음 · 메커니즘 암묵 · 로컬 미동작 · CI 미동작)이 각각 독립으로 선다. 과거 TRD(`docs/process/C-slot-content/TRD.md:19-20`)가 "ts-rs 재생성으로 자동 갱신(수동 편집 금지)"이라는 **의도**를 적었으나 게이트로 실현된 적이 없다.

**남는 미확인 하나(축소된 형태):** `cargo build`(테스트가 아닌 일반 컴파일)가 ts-rs export 경로를 트리거하는지는 실측하지 않았다. ts-rs export는 통상 `#[test]` 경로로만 도는 것으로 알려져 빌드가 트리거하지 않을 가능성이 높으나 **확정하지 않았다.**

**귀결:** Step 3은 이제 "파생 경로가 있는지 확인"이 아니라 **"파생 경로를 세우기"**를 실작업으로 포함한다. 어떤 형태로 세울지는 **Step 3 착수 시 정하며 이 TRD가 지금 확정하지 않는다.**

---

## 6. 마이그레이션

각 Step은 **혼자 배포 가능하고 혼자 검증 가능**하다. 앞 Step이 뒤를 안 깬다.

★개정 — Step 0이 사라졌다★
구판 Step 0 = "선행(이 TRD 밖): ADR-0081 릴레이 구현". → **Step 3에 흡수한다.** 확정 구조가 "새로 짓는 것은 도구 crate + 데몬→셸 인바운드 둘뿐"이라 릴레이는 별개 선행이 아니라 이 작업의 한 조각이다. ADR-0132 결정 7의 **순서**(릴레이가 셸·화면 명령보다 먼저)는 그대로 지켜진다 — Step 3이 Step 4보다 앞이다.

### Step 1 — 도구 crate + 첫 입주자(`agent.*`), 배선 0

도구 crate 신설(봉투·오류·매크로·표·라우팅) + `core`에 `agent.*` 선언 + `make_table(manager)`. 아직 아무도 이 표로 라우팅하지 않는다. 첫 입주자는 이미 CLI에 실재하는 동사들이다(`types.rs:246` 5개).
- **검증:** `cargo test -p engram-dashboard-command`(신설) · `cargo test -p engram-dashboard-core` · `core/bindings/` diff 게이트 **신설**(§5).

### Step 2 — 데몬: 자기 표 + 명부 + 등록 wire

`mail.*` 선언(`types.rs:212` 3동사) + `make_table` · 주인 명부(tombstone 포함) · `RegisterCommands`/`UpdateCommands`/`ListCommands`를 `connection_core.rs:582` dispatch에 additive arm으로 추가. `engram` CLI(`crates/engram-dashboard-daemon/src/bin/engram.rs`)가 이름으로 부르게 바꾸고, 문자열 패턴매칭 오류 합성(S17 TRD `trd.md:69`)을 타입드 오류로 교체한다.
- 에이전트 문(HTTP `/control`)의 5동사 match(`crates/engram-dashboard-daemon/src/control/agent.rs:222-236`)가 **지금의 벽**이다 — 이 Step이 그 match를 표 조회로 바꾼다.
  - ★착지(2026-08-16, 슬라이스 3)★ **그 match는 삭제됐다 — 위 줄번호로 찾지 말 것.** 현행 입구는 `handle_agent`(`crates/engram-dashboard-daemon/src/control/agent.rs`) 하나이고 `{verb}` → `agent.<verb>` 이름 조립 → `contains` → `check_args`(ADR-0136) → `call` 순으로 간다.
- **검증:** 데몬 단위 테스트(§7) · `engram agent list`/`spawn`/`new`/`rename`/`move` 동작 불변.
  - ★정정(2026-08-16, 슬라이스 3 착지)★ **「동작 불변」은 wire 층까지 참이 아니다.** 불변인 것은 **`engram agent <verb>`의 종료코드와 사용자가 보는 결말**이고, 봉투 **구조**(`{status,code,hint}` · `status:"error"`)와 hint 꼬리에 칠 수 있는 명령이 붙는다는 성질도 그대로다. **의도적으로 바뀐 것 둘은 사용자 승인분이다:** ① 성공 payload가 중첩(`{"agent":{id,name,state}}`)에서 표의 평평한 Ok 구조체(`{agent_id,name,state,created}` 등)로 — 평평한 모양 독해는 슬라이스 2가 CLI에 미리 깔았고 중첩 독해 제거는 슬라이스 4다 ② 오류 `code` 어휘가 도메인 문자열에서 도구 crate의 타입드 `ErrorCode` wire 문자열로(`AGENT_NOT_FOUND`→`NOT_FOUND` · `AGENT_AMBIGUOUS`·`MOVE_REJECTED`·`ROSTER_FULL`·`NAME_SPACE_EXHAUSTED`→`CONFLICT` · `SPAWN_FAILED`→`INTERNAL` · `INVALID_AGENT_ARGS`→`INVALID_ARGUMENT` · 모르는 동사→`UNKNOWN_COMMAND`). **이 줄을 근거로 문을 중첩 모양·도메인 코드로 되돌리지 말 것.**

### Step 3 — 셸: 인바운드 수신기 + 공유 적용 서비스 + 자기 선언

★**이 Step이 가장 무겁다**★ — ADR-0081이 요구한 서비스가 **오늘 없다.** `src-tauri/src/commands/layout.rs`의 핸들러들이 `LayoutState`를 잠그고 `ViewManager` 메서드를 직접 부른다(`layout.rs:108`·`:145`·`:227`). ADR-0081 「거부한 대안」이 명시적으로 거부한 그 형태다.
1. ★개정★ **`layout.rs`의 `#[tauri::command]` 16개 전부**를 전송 중립 적용 서비스 뒤로 옮기고, 그 옆에서 `window`/`tab`/`slot`을 선언한다. 구판: "**쓰기 12개**(16개 중 **읽기 4개 제외** — §8 가정 B)". 가정 B가 채택되면서(ADR-0135) 읽기 4종(`get_view` `:497` · `list_tabs` `:508` · `list_windows` `:520` · `resolve_spatial` `:530`)이 합류해 **16 = 쓰기 12 + 읽기 4** 전량이 이 Step의 범위다(재실측 2026-08-13).
2. `#[tauri::command]`는 그 서비스를 부르는 얇은 껍데기가 된다 — 사람 클릭 경로는 그대로.
3. `daemon_client`에 인바운드 수신기를 단다 — `connection.rs:805`의 `Message::Text` 처리에 `Request` 갈래 하나. **적용은 연결 태스크 밖(spawn)에서** 돈다(§3-6).
4. 셸이 데몬에 자기 이름을 등록한다(§3-7).
- **검증:** 사람 클릭 GUI 실측(`scripts/cdp.mjs`) + 중계 왕복 + §7의 교착 회귀 테스트 + `src-tauri/bindings/` 게이트 — **오늘 그 파생 경로가 없다는 것이 실측으로 확인됐으므로**(§5) 이 Step이 **경로를 세우는 것까지** 포함한다(측정이 아니라 구축이 남았다).

### Step 4 — 화면: 자기 선언 + 셸 경유 등록

부팅 시 `list()` 결과를 셸에 통지하고, 셸이 대신 등록한다(§3-7 조항 2).

★**개정(2판) — `registry.ts`를 additive로 한 칸 넓힌다**★
**구판(축약 보존):** "`registry.ts`는 **손대지 않는다**(`src/commands/registry.ts:29~79`)."
**현판:** 그 문장의 뜻은 **「두 번째 제어 표면을 만들지 않는다」**였지 **「한 필드도 못 넣는다」**가 아니었다. 문자 그대로 읽으면 계약이 모순된다 — 오늘 `Command` 인터페이스(`registry.ts:11-27`)에 설명·인자 칸이 없고 `list()`(`:67-74`)는 `id`·`title`·`category`·`keybinding` **넷만** 돌려주므로, 등록 패킷의 `help`(§3-7)가 웹뷰에서 **나갈 길 자체가 없다.** 그런데 §7 seam 표는 그 값이 비어 있지 않은지를 vitest로 단언한다.

- **넓히는 폭 = 정확히 한 필드.** `Command`에 `help: string`을 더하고 `list()` 반환의 `Pick<...>`에 `'help'`를 더한다. `register`/`run`/`getCommand`/`__resetRegistryForTest`와 Map 골격은 **그대로**다.
- **ADR-0055가 지키던 것은 하나도 안 바뀐다** — 안정 id · 「상태 권위가 아니다」(발견·라우팅·메타만) · DOM-free · 소비자는 `run(id, args)` 하나. 새 전역 핸들도 두 번째 레지스트리도 생기지 않는다(CLAUDE.md 「LLM-우선 제어」).
- **값의 내용은 §2-2 3이 정한다** — 그 명령의 스키마 항목 하나를 직렬화한 JSON 텍스트. TypeScript엔 매크로가 없어 **손으로 적는 유일한 자리**이고, 그래서 §7이 누락을 단언으로 막는다.

★개정(1판)★ 구판: "부팅 시 `list()`를 **카탈로그의 View 소유 이름과 대조**한다." → 대조 상대(중앙 카탈로그)가 없어졌다. 현판의 대조는 **화면 선언 ↔ `registry.ts` 등록분** 사이에서만 일어난다.

33개의 분류는 **handler가 어디로 라우팅하는가**로 정한다. id는 안정 id라 바꾸지 않는다(ADR-0055).

| 주인(선언처) | 해당 id (분류 근거 = handler의 라우팅 대상) |
|---|---|
| 웹뷰 | `theme.set`·`theme.toggle`(`themeCommands.ts:9`,`:22`) 등 순수 store 조작 |
| `src-tauri` | `tab.*`·`window.*`·`slot.*`·`layout.setSlotContent`·`agent.spawnInto` — `invoke`로 셸에 간다(`tabCommands.ts:29~181`·`slotCommands.ts:48~98`) |
| `core`(호스팅은 데몬) | `agent.spawn`(`agentCommands.ts:41`)·`agent.rename`(`:76`)·`agent.kill`(`slotContentCommands.ts:118`)·`preset.*`(`presetCommands.ts:14~69`) — `agentClient`로 데몬에 간다 |

- **검증:** `npm test`에 「화면 선언 ↔ `list()`」 대조 테스트 추가(어휘 drift를 CI가 잡는다) + ★2판★ `list()` 각 항목의 `help`가 비어 있지 않은지 단언(§7 화면 표 행) + `npx tsc --noEmit`(필드 추가가 기존 등록처를 깨지 않는지).

### Step 5 — 곁문 둘 철거

| 핸들 | 실물 | 처리 |
|---|---|---|
| `window.__engramLayout` | `src/store/eventBus.ts:80-111`, 메서드 15개(실측). 자기 주석이 "정식 command 버스 전까지의 임시 경로"(`eventBus.ts:72` — 구판은 `:70-71`로 적었으나 그 두 줄은 다른 문장이다)·`setSlotContent`는 "`layout.setSlotContent`와 병행 경로"(`:93-95`)라 자인 | command로 안 덮인 것(`setRenderMode`·`clearRenderMode`·`enableDomMode`·`disableDomMode`·`toggleDomMode`·`moveSlotToWindow`)을 **먼저 command로 승격**한 뒤 핸들 삭제 |
| `window.__engramChat` | `eventBus.ts:118-124`, 5항목 | ★개정★ **5항목 전부 command로 승격 → 핸들 삭제.** 구판: "`set`·`patch`·`reset`만 승격 · `get`(`:119`)·`defaults`(`:123`)는 **읽기**라 §8 가정 B에 걸려 v1 범위 밖 → 핸들을 통째로 못 지운다". 조회가 v1에 들어와(ADR-0135) 그 둘도 command가 된다 |
| `window.__engramCmd` | `eventBus.ts:134-137` | **남긴다.** 이게 화면 표의 로컬 입구다 |

★개정★ **곁문 둘을 v1에서 전부 철거한다.** 구판: "가정 B의 실제 대가가 여기 드러난다 — 「곁문 둘을 없앤다」가 v1에선 「하나 반」이 된다." 가정 B가 채택되면서(ADR-0135) 그 대가가 사라졌고, 이 Step의 이름(「곁문 둘 철거」)이 문자 그대로 성립한다.

### Step 6 — CLI 어휘를 선언에서 파생

`types.rs:246`의 `CLI_AGENT_VERBS` 같은 손 배열을 같은 crate의 선언 생성물로 바꾼다. ★개정에서 쉬워진 자리★ — `agent.*` 선언이 **바로 그 파일 옆(core)** 에 있으므로 crate 경계를 넘지 않는다. 구판(선언이 `protocol`)이었다면 `core`가 `protocol`을 보게 돼 워크스페이스 의존 0이 깨졌다.
**마지막이어야 한다** — 앞 Step이 서기 전에 하면 CLI가 없는 핸들러를 가리킨다.

### v1에서 이관하지 않는 것

`src-tauri`의 40개 `#[tauri::command]`(`src-tauri/src/lib.rs:147-188` — `generate_handler!` 블록, 재실측 40개) 중 **23개는 남긴다**: `commands/agent.rs` 9개(이미 `AgentCommand` 직송의 얇은 래퍼) · `commands/discovery.rs` 9개 · `commands/autostart.rs` 2개 · `commands/tray.rs` 3개(클라이언트 로컬 생애주기). 이유: 전자는 데몬 소유 명령의 중복 표면이 되고, 후자는 "데몬이 없을 때도 눌러야 하는 것"이라 버스에 태우면 순환이 생긴다. 필요해지면 `src-tauri` 선언으로 additive.

**남는 17개의 내역**(구판이 안 적었다): `commands/layout.rs` 16개 + `commands/popout.rs` 1개. ★개정★ 구판: "Step 3이 다루는 것은 layout **12개(쓰기)**이고, layout **읽기 4개는 §8 가정 B에**, popout 1개는 미분류로 남는다." 현판: **Step 3이 layout 16개 전부**를 다루고(ADR-0135), 남는 미분류는 **popout 1개뿐**이다 — 이관 시점에 주인을 정한다.

---

## 7. 테스트 전략

ADR-0012 — 모든 조각이 외부 의존을 seam으로 끊고 단독 하네스를 갖는다.

### ★규칙 T-1 (하드 제약) — 전역 자동등록 표 금지★

**핸들러가 미리 박힌 전역 표를 만들지 않는다.** 각 모듈은 `make_table(deps) -> CommandTable`만 공개하고, 실물 의존은 **조립 때 주입**한다.

- **왜 하드 제약인가:** 전역 표에 핸들러를 박으면 테스트가 **가짜 관리자를 꽂을 자리가 없다.** `agent.spawn` 하나를 단언하려고 진짜 `AgentManager`가 딸려오고, 그러면 **단위 테스트가 실제 프로세스를 띄운다.** 이건 취향이 아니라 ADR-0012 「외부 의존을 seam으로 끊는다」의 직접 위반이다.
- **판정 기준(코드 리뷰에서 볼 것):** `static`/`lazy_static`/링커 수집이 **`Arc<dyn CommandHandler>`를 담고 있으면 위반**이다. 링커 수집이 담아도 되는 것은 `CommandSpec`(이름·스키마·오류)까지다.
- 자동 수집을 아예 안 쓰는 게 아니다 — **이름·인자 스키마는 자동, 실물은 수동**이 이 규칙의 전부다.

### seam 표

| 조각 | 끊는 seam | 하네스 |
|---|---|---|
| 선언 파생 | 없음(순수 매크로 확장) | 선언 crate마다 `cargo test -p <crate>` — 이름·오류 집합 골든 + `bindings/` diff 게이트. ★개정★ 구판은 `protocol` 한 곳이었다. 이제 **crate마다** 돈다 |
| `CommandTable` | 없음(순수 자료구조) | 중복 삽입 = `Err` · 자기 crate 선언 밖 이름 = `Err` · 미지 조회 = `None` |
| **`make_table` 주입** ★신설★ | **의존 인자 그대로**(`AgentManager` 등) | 가짜 관리자를 넣어 `agent.spawn` 핸들러가 **프로세스 없이** 단언된다. 이 하네스가 서지 않으면 T-1이 깨진 것이다 — **이 테스트가 T-1의 기계적 감시자다** |
| `route()` | **`CommandLink` + `Roster`** | `FakeLink`(보낸 봉투를 기록하고 미리 정한 답을 돌려준다) + 빈/채운 명부 — 소켓 0으로 **3단계 규칙**을 단언(내 표 / 명부 / 오류). ★개정★ 구판은 2단계만 단언했다 |
| **2단 배달** ★신설★ | `FakeLink` 두 개(데몬→셸, 셸→웹뷰) | 같은 `request_id`가 전 구간 유지되는지 · 중간 홉이 답을 하나만 내는지 · 홉마다 같은 3단계를 도는지 |
| 인바운드 수신기 | **`ReplySink` + spawn 주입** | ★교착 회귀 테스트★: 핸들러 안에서 같은 링크로 두 번째 명령을 부르는 fake를 넣는다. 인라인 실행이면 hang, spawn이면 통과. **지금 이 회귀를 잡는 테스트가 없다** — ADR-0081 결정 3의 self-deadlock을 형태로 고정한다. ★개정★ 셸뿐 아니라 **중간 홉(셸)의 경우도** 같은 하네스로 돌린다 |
| 데몬 명부 | **`OutboundSink`**(`connection_core.rs:582` dispatch가 이미 `&dyn OutboundSink`를 받는다 — 새 seam 불요) | 등록 · 델타 · last-wins · 연결 해제 시 **tombstone 전환**(삭제 아님) · 주인 부재 = `OWNER_UNAVAILABLE` · 한 번도 못 본 이름 = `UNKNOWN_COMMAND`(§4-② 대가를 테스트로 못 박는다) · ★2판★ **`help` 왕복 골든** — 등록 → 명부 → `CommandList` 조회에서 문자열이 **바이트 그대로** 보존되는지 · **데몬이 파싱하지 않는지**(JSON이 아닌 임의 문자열을 넣어도 등록·조회가 그대로 성공해야 한다 — 파싱이 끼면 여기서 깨진다) |
| 셸 적용 서비스 | **`AppHandle`·`State<..>` 제거** | ★난점★: 오늘 핸들러는 `AppHandle` + `State` 여럿을 받아(`layout.rs:98`·`:122`·`:215`) 단독 호출이 불가하다. 적용 서비스는 그것들을 인자에서 걷어낸 순수 함수여야 단독 테스트가 선다. **Step 3의 실제 난이도가 여기다** |
| 화면 표 | **`__resetRegistryForTest`**(`src/commands/registry.ts:77-79` — 이미 있다) | `npm test`(vitest)에 화면 선언 ↔ `list()` 대조 추가 · ★2판★ **선언마다 `help`가 비어 있지 않은지** — TypeScript 선언은 손으로 적는 값이라(§2-2 3) **누락이 조용히 새는 유일한 자리**다. Rust 쪽은 매크로가 채우므로 이 단언이 필요 없다 |

CI는 push마다 위 명령을 windows 러너에서 돌린다(CLAUDE.md 「CI」). 로컬 몫으로 남는 것은 GUI 실측(Step 3·5) 하나다. ★단 `src-tauri` 패키지는 워크스페이스 테스트에서 제외돼 있어(`ci.yml:96-99`) 셸 조각의 CI 커버리지가 **오늘 비어 있다** — §5의 파생 게이트 항목(실측으로 닫힘)과 같은 뿌리다.★

---

## 8. 가정

### 가정 A — 목록 취합을 데몬이 받는다 ★개정 — 미결에서 채택으로★

**구판:** "지금 내가 걸어둔 기본값 · 사용자 결정 대기".
**현판:** **채택된 형태다.** 확정 구조의 등록 wire(§3-7)가 이 형태를 전제로 서 있고, 뒤집힌 판단 3(데몬이 빌드 목록을 조회하지 않는다)이 그 위에서만 뜻이 통한다.

**채택 근거 둘 — 선례가 아니라 우리 구조에서 나온다:**
1. **병합 로직을 복제해야 하는 호출자 수가 줄어든다.** 호출자는 셋 이상이다 — `engram` CLI · MCP 우편 경로 · 화면. 셋이 각자 발견·병합을 짜면 어휘가 또 갈리고, 그건 지금 고치려는 증상 자체다(step-log.md:1673 ②).
2. **2단 배달이 균일하게 유지된다**(§3-8). 데몬이 명부를 쥐면 홉마다 규칙이 하나다. 호출자 측 병합으로 가면 「누가 명부를 갖나」가 홉마다 달라지고 특별 케이스가 생긴다 — 확정 구조 ④(「특별 케이스 없음」)가 무너진다.

★개정(2판)★ **데몬이 배우는 것은 이름 · 주인 토큰 · 불투명 `help` 문자열 한 칸뿐이다.** 구판(축약 보존): "여전히 **이름과 주인 토큰뿐**이다." ADR-0135로 모양이 등록 패킷에 동봉되면서(§1 · §3-7 · §5 daemon 행) 칸이 하나 늘었으므로 구판 문장은 그대로 두면 거짓이다.

**단 C1(데몬은 셸을 모른다) 형식 보존은 그대로다** — 데몬은 그 문자열을 **열어보지 않는다**(§3-7 하드 제약: 파싱·검증·분기 금지, 자료형 `String` 고정). 저장하고 조회에 되돌려줄 뿐이라 "클라이언트 셸"이라는 구체 개념도 명령의 뜻도 배우지 않고, **컴파일에 들어가는 UI 타입은 여전히 0**이다(§9 모순 2와 같은 논거).

★**숨기지 않는 긴장 — 조사 보고서는 이쪽에 서 있지 않다**★ (구판에서 그대로 유지)
- 조사 §5-3이 "데몬을 목록 취합자로 승격 = C1(데몬은 셸을 모른다) 파괴"를 **거부 대안**으로 적었다.
- 조사 §8 3위(호출자 측 발견·병합)를 두고 "데몬의 셸 무지를 **가장 깨끗하게 보존**한다"고 판정했다.
- 조사 §3-③ 반례 (가)의 **탈중앙 발견 선례(ROS 2)** — 중앙 마스터 없이 임의 로컬 노드가 자기가 발견한 그래프에서 전체 서비스 목록을 조회 — 가 **실제로 대안 쪽을 지지한다.** 이 선례는 조사 초판의 "선례 없음"을 반증하며 등장한 것이라 무게가 가볍지 않다.

**즉 선례만 보면 대안이 앞선다.** 채택 근거는 선례 우위가 아니라 **호출자 수 + 홉 균일성**이고, 그 사실을 여기 적어둔다. 조사 보고서는 **판정 BLOCK**이라 이 긴장을 닫는 근거로도 쓸 수 없다(§9).

**뒤집으면 바뀌는 절:** §3-7(등록 wire가 사라지고 `QueryCommands{owner}` + 호출자 측 병합기) · §3-8(홉마다 명부 소유가 달라져 특별 케이스 발생) · §4-②·④(데몬이 명부를 안 가지므로 `OWNER_UNAVAILABLE`을 못 낸다 → 전부 호출자 타임아웃으로 격하, 즉 **미적용 확정이 사라지고 전부 불명이 된다**) · §5(공용 발견 클라이언트가 새 모듈로 필요 — 어느 crate에 둘지가 새 질문) · §6 Step 2 · §7(데몬 명부 하네스 → 병합기 하네스).

### 가정 B — 값 읽기(조회)를 v1 명령 목록에 넣는다 ★개정 — 미결에서 채택으로(ADR-0135)★

**구판:** "값 읽기(조회)는 v1 목록에 **넣지 않는다** · `Effect::Read` 표식은 타입으로 **존재하되 v1 목록엔 항목이 없다** · **미결, 사용자 결정 대기**(인계 「정지 조건」)". 그 기본값의 이유는 하나였다 — **되돌릴 수 있는 쪽이다.** 나중에 읽기를 넣는 것은 §4-③ additive가 그대로 덮는 덧붙이기지만, 넣었다 빼는 것은 계약 파괴다.
**현판:** **포함이 채택됐다.** `Effect::Read` 항목이 v1 목록에 실린다.

**채택 근거 — 발견 경로를 반만 닫을 수 없다:** `help`는 「**무슨 키**가 있나」를 알려주지 「**지금 값**이 뭔가」를 알려주지 않는다. 등록 패킷 동봉(D1)으로 인자 *모양*의 발견은 닫혔지만, 조회가 없으면 LLM은 **어떤 탭이 있는지도 모른 채** `tab.close`를 불러야 한다 — 모양만 알고 대상은 모르는 상태다. 구판 근거였던 「되돌릴 수 있음」은 **늦추는** 이유였지 **안 하는** 이유가 아니었고, 발견 경로를 닫는 이 판에서 조회만 빼면 발견이 절반에서 멈춘다.

**실물 근거(추정 아님)** — 구판이 「실제 대가」로 적어둔 것이 그대로 **합류 목록**이 된다:
- `src-tauri/src/commands/layout.rs`의 읽기 4종 — `get_view`(`:497`) · `list_tabs`(`:508`) · `list_windows`(`:520`) · `resolve_spatial`(`:530`), 넷 다 소스 주석이 "★조회만★"이라 자인 — 이 넷이 v1 command가 된다(같은 파일 `#[tauri::command]` **16개** = 쓰기 12 + 읽기 4, 재실측 2026-08-13).
- `__engramChat`의 `get`(`eventBus.ts:119`)·`defaults`(`:123`) — 이 둘이 들어오면서 §6 Step 5의 곁문 철거가 완결된다(구판에선 이 둘 때문에 핸들을 통째로 못 지웠다).
- §4-⑤(진행 보고 없음)와 겹치던 구멍이 메워진다 — 중간 상태를 물어볼 경로가 v1 안에 생긴다.

조사 §3-②는 이 갈림길에서 **수렴하지 못했다** — 합치는 쪽이 다수(LSP·D-Bus·K8s·sway·WezTerm·CDP·OBS)이고, 가르는 쪽은 실행 의미가 달라서 갈랐다(GraphQL의 query/mutation, MCP의 Resources/Tools). 조사가 승자를 못 고른 자리라 **채택 근거는 선례가 아니라 우리 발견 경로에서 나온다**(가정 A와 같은 형태의 논거다).

**이미 반영된 변경**(구판의 「뒤집으면 바뀌는 절」 목록 = 이제 이 판의 반영 내역):
- §1 비목표 표 — 해당 행 삭제. **완료.**
- §2-2 · §3-1 — `Effect::Read`가 실항목을 갖는다(표식 자체는 이미 타입에 있었다). 위 6개가 입주자에 합류. **완료(선언 자체는 Step 3·4에서).**
- §4-⑥ — 읽기는 멱등이라 dedup 면제 규칙 추가. **완료.**
- §6 Step 3 — 쓰기 12개 → **layout 16개 전부.** **완료.**
- §6 Step 5 — 곁문 둘을 v1에 **전부** 철거. **완료.**
- §7 — 읽기 명령의 read-your-writes 검증 추가. ★**미반영**★ — 이 판에서 seam 표에 넣지 않았다. Step 3·4 착수 시 하네스에 더한다. ★2판 보완★ **§9 「미확인」에도 올렸다** — 자인이 이 절에만 있으면 발견 표면(§9)을 보는 다음 세션이 놓친다.
- §2-4 ③ — 구판은 "읽기가 늘면 LLM 스키마 미확정 구멍의 표면이 함께 넓어진다"였다. D1이 그 구멍을 닫아 **문제 자체가 소멸했다.**

---

## 9. 근거

### 출처

- **확정 구조(이 판의 정본):** `.claude/handoff/latest.md` 「★확정된 구조★」·「★세션 중 뒤집힌 판단 3건★」·「★실패한 접근 / do-not★」
- **선례 조사:** `docs/research/unified-command-bus-survey-2026-08-12.md` — **판정 BLOCK**
- **사용자 방향(2026-08-12):** `docs/process/step-log.md:1667-1675` — 특히 `:1669`(주인 칸 불필요) · `:1673`(어휘가 서로를 모른다). ★2판 재실측(2026-08-13)★ step-log에 S20.1·S20.2 두 절이 「다음(미진행)」 앞에 끼어 그 아래가 **+16줄** 밀렸다 — 구판 번호(`1651-1659` · `:1653` · `:1657`)는 이제 딴 곳을 가리킨다. 이 문서의 step-log 인용 6곳을 전부 되짚어 고쳤다(`:732`는 삽입 지점 위라 그대로).
- **이 설계 트랙의 타임라인:** `docs/process/step-log.md:1649`(S20.1 — 선례 조사 → TRD 초안 → 개정 1판의 반전 3건) · `:1657`(S20.2 — 발견 경로 결정 · ADR-0134/0135 · 개정 2판과 적대 리뷰 FIX). **왜**는 ADR, **언제·무엇**은 이 두 절이다.
- **결정:** **ADR-0134(S20 구조 — 이 판의 정본) · ADR-0135(발견 경로 3건 — 등록 패킷 동봉 · 값 읽기 v1 포함 · tombstone 만료 없음)** · ADR-0022(방향 — 상태 **제안**) · ADR-0055(프론트 레지스트리 골격) · ADR-0064(메뉴 = command id 참조) · ADR-0081(릴레이 형태) · ADR-0132 결정 7(후속 순서) · ADR-0129(net 3층 분리) · ADR-0035/0057(레이아웃 권위) · ADR-0012(테스트 격리) · ADR-0003(코어 격리) · ADR-0110(메시징 커널 무의존)
- **선행 TRD:** `docs/process/S17-llm-control-surface/spec/trd.md` — §4-④/⑥/⑦의 확실성 인코딩·dedup·오류 코드는 **거기서 이미 결정된 것을 승계**한 것이지 새로 만든 것이 아니다.
- **실측(2026-08-13, 이 개정에서 재확인):** `core` 워크스페이스 의존 0(`Cargo.toml`에 `path =` 0줄 — ★이 설계가 0→1로 바꾼다: 도구 crate, §5★) · `protocol` 0(`Cargo.toml:8-13` — ★2판★ **이 설계가 0→1로 바꾼다** — 봉투·등록 단위 타입 때문에 도구 crate 하나를 의존한다. §3-1 · §5 protocol 행) · `daemon` 5(`Cargo.toml:79-92`) · `messaging` 0 · `src-tauri`는 core·protocol·discovery·net 4 · `protocol/bindings/` 23개 · `src-tauri/bindings/` 8개 · CI 바인딩 게이트는 `protocol/bindings/`만 본다(`ci.yml:124-131`) · 워크스페이스 테스트는 `--exclude engram-dashboard`(`ci.yml:96-99`) · `AgentCommand` variant 25종 · `src/commands/` 등록 명령 33개 · `layout.rs` `#[tauri::command]` 16개(쓰기 12·읽기 4) · `lib.rs` 등록 handler 40개 · `RegisterRole`/`RelayUi`/`UiCommand`/`UiResult` `.rs` hit 0

### ★조사 보고서를 얼마나 믿을지 (후속 독자 경고)★

**판정은 BLOCK이다.** 보고서 스스로 "§8·§9의 열린 항목이 닫히기 전까지 ADR 거부 근거로 사용 금지"라고 적었다(조사 §6 첫 항목). 아래는 적대 리뷰에서 **반증된** 주장들이라, 이 문서들만 읽고 되살리지 말 것.

| 조사 초판 주장 | 상태 |
|---|---|
| Zellij가 "단일 `Action` 열거형 = 유일 명령 어휘"의 완전 동형 선례 | **반증** — 플러그인 API가 별도 함수 표면을 대규모로 갖는다. 부분 선례로 강등 |
| OBS는 외부 미도달(D계열 반면교사) | **반증** — OBS 28+ 는 WebSocket 제어 API를 기본 포함. D계열 예시는 VS Code만 |
| 권한 검사는 등록 시 한 번이 흔하다 | **반증** — 매 호출 검사가 조사 표본의 표준. ★이 반증이 §4-⑧ 「인가는 호출마다」의 근거다★ |
| 비중앙 당사자가 목록을 합치는 기제의 선례 없음 | **반증** — ROS 2의 탈중앙 발견. ★이 반증은 §8 가정 A의 **대안** 쪽을 지지한다★ |
| "생성기를 두고 이름만 생성하는 성숙 사례 0건" | **표본 한정** — 검색 범위 미제시. 이 문장 단독으로 거부 근거 불가 |
| 형제 도구(`tauri-specta`)가 "정확히 그 범위를 덮는다" | **정정** — 생성물이 앱 프레임워크 invoke 경로를 직접 부른다(전송 교체와 충돌). §2-2가 매크로를 택한 이유 |

grounding 전수는 **미실시**이고, 리뷰 스팟체크 5건 중 4건이 NOT SUPPORTED로 나왔다(조사 §6 한계 2).

### ★기존 기록과 어긋나던 자리 3건 — 개정 후 상태★

**모순 1 — 주인 칸. ★해소(구판 주장 철회)★**

- **구판 주장:** step-log.md:1669이 "주인 지정은 불필요 — 정의 위치가 곧 주인"이라 기록했는데도 `#[owner(Daemon|Shell|View)]` 칸을 되살린다. 근거는 "선언을 protocol 한 곳에 모으면 **정의 위치가 주인을 말해주지 않으므로** 칸이 필요해진다"였다.
- **개정 후:** **선언 중앙화가 폐기되면서 그 근거가 통째로 사라졌다.** 정의 위치가 다시 주인을 말한다. 칸을 삭제한다(§2-2 · §3-1).
- **남길 교훈:** 구판의 논리 자체는 옳았다 — 「중앙화하면 주인 칸이 필요하다」는 참이다. 틀린 것은 **전제(중앙화)**였다. 사용자가 두 번 철회시킨 제안이 되살아난 것은 우연이 아니라 전제가 그것을 다시 요구했기 때문이고, 전제를 바꾸는 것이 정답이었다. (인계 「do-not 2」.)

**모순 2 — ADR-0081 「Opaque 결합 가드」. ★철회(폐기 도장 불요)★**

- **구판 주장:** ADR-0081 영향절과 S17 TRD `trd.md:36`(「★Opaque 결합 가드(H)★」)이 "UI 명령 enum + payload→`ViewCommand` dispatch 맵은 **오직 `src-tauri`** — `engram-ctl`·데몬 relay·`core`는 UI enum을 **import하지 않는다**"라고 못 박았는데, 이 설계는 셸·화면 명령의 이름과 인자 스키마를 `protocol`에 두고 **데몬과 CLI가 그 crate를 의존**하므로 가드의 문자 그대로와 충돌한다 → **"가드 H에 부분 폐기 도장이 필요하다"**(단 판단은 사용자 몫).
- **개정 후: 그 결론은 틀렸다. 도장은 필요 없다.**
- **왜 옛 읽기가 틀렸나 — 충돌은 「이름이 겉봉으로 나온 것」이 아니라 「선언 위치」에서 왔다.** 구판은 충돌 원인 둘(① 이름 노출 ② 선언 위치)을 나란히 적고 **둘 다 가드를 건드린다**고 읽었다. 실제로 가드가 금지하는 것은 **타입 import**다("UI enum을 import하지 않는다"). ①은 데몬이 `name: String`을 겉봉에서 읽는 것이라 **타입 import가 아니다.** ②만이 진짜 위반이었고, ②가 폐기되면서 위반이 사라진다.
- **개정 후 데몬이 아는 것:** 셸·화면 명령의 **이름 문자열뿐**이고, 그것도 **런타임 등록으로** 받는다(§3-7). 컴파일에 들어가는 UI 타입은 0이다. **데몬은 `src-tauri`도 웹뷰도 의존하지 않는다.** 즉 가드는 **문자 그대로 유지된다.**
- **남는 확장 1건(폐기 아님):** ADR-0081의 봉투는 순수 opaque였고 여기서 `name`이 겉봉으로 나온다. 이건 가드(타입 결합)가 아니라 **봉투 형태**의 additive 확장이고, 조사 §2가 지지한다(명령 단위 인가·관측이 여기 달려 있다 — §4-⑧). `args`는 여전히 불투명이라 ADR-0081 결정 2의 본체는 산다. (인계 「do-not 3」.)

**모순 3 — ADR-0055의 탈중앙 논거. ★해소★**

- **구판 주장:** ADR-0055는 "enum-데이터 단일 커맨드(variant + 중앙 dispatch)"를 거부하며 "**새 커맨드마다 중앙 enum + match + 메타맵을 수정** → 기능이 자기 command를 스스로 등록하는 탈중앙 목표에 위배"를 근거로 들었는데(ADR-0055 「거부한 대안」 1항), 카탈로그를 protocol 한 곳에 모으는 구판은 그 중앙화를 일부 되불렀다. 구판의 완화 근거는 "매크로 한 블록이 세 곳 수정을 한 곳으로 줄인다" + "실행부 등록은 여전히 탈중앙"이었고, "그래도 선언은 중앙이 됐다"고 자인했다.
- **개정 후:** **선언도 탈중앙이 됐다.** 각 모듈이 자기 명령을 자기 자리에서 선언하고 자기 표를 만든다 — ADR-0055가 지키려던 것 그 자체다. 완화가 아니라 **정합**이다.
- **덧붙임:** ADR-0055의 "백엔드 미러 = 후속" 조항이 이 작업을 예정해 뒀으므로 번복이 아니라 그 후속의 실물이다. 구판에서 유일하게 어긋나던 한 줄(선언 중앙화)이 사라졌다.

### 미확인 (추측으로 메우지 않음)

- ~~**LLM이 셸·웹뷰 소유 명령의 인자 모양을 어디서 얻는가**~~ — ★개정 — 닫혔다(ADR-0135)★ **등록 패킷에 동봉**한다(§2-4 ③ · §3-7). 구판: "등록 패킷은 이름만 나르고 웹뷰 선언은 Rust 파생물에 없다. 후보 둘(스키마 동봉 / `command.describe` 왕복)은 굵은 형태 결정이라 **사용자 결정**." 이 자리에 남는 미확인은 없다.
- ~~**`src-tauri/bindings/` 8개 파일의 생성·갱신 절차**~~ — ★실측으로 닫힘(2026-08-13)★ **절차가 없다** — 재생성 메커니즘(ts-rs derive의 `export_bindings_<타입>` 테스트)은 있으나 로컬(`0xc0000139`)에서도 CI(패키지 제외)에서도 돌지 않고 npm·`scripts/`·alias·hook 어디에도 경로가 없다. 즉 **손으로 관리되는 생성물**이다. 구판: "CI가 그 패키지 테스트를 제외하므로 파생 게이트를 어떻게 되살릴지 미정 — Step 3 착수 전 실측 필요." 근거·이력은 §5. **남는 미확인은 하나로 줄었다:** `cargo build`가 ts-rs export를 트리거하는지 미실측(안 할 가능성 높음, 미확정).
- ~~**tombstone 보존 기간**~~ — ★개정 — 닫혔다(ADR-0135)★ **만료 없음**(데몬 수명 동안 유지 · 재등록 시 last-wins). 구판: "데몬 수명 전체인지 TTL인지 **미확정**". 거부 이유는 §4-②.
- **읽기 명령의 read-your-writes 검증** — §8 가정 B가 반영 항목으로 적었으나 **§7 seam 표에 아직 없다**(그 자리에서 ★미반영★으로 자인). 조회가 방금 쓴 값을 돌려주는지를 어느 하네스에서 단언할지 미정 — **Step 3·4 착수 시** seam 표에 더한다.
- **링커 수집 crate 선정** — §2-2. 새 외부 의존 1개는 CLAUDE.md 「의존성(변경 시 보고)」 대상.
- 매크로가 직접 찍는 JSON Schema의 타입 알파벳이 실무를 덮는지 — Step 1에서 실측(§2-2).
- 33개 화면 command의 주인 분류 경계 사례 유무 — §6 Step 4의 표는 handler 라우팅 대상 기준의 1차 분류이고, 개별 검증은 이관 시점.
- `core`가 CLI 어휘 상수(`types.rs:179~246`)를 왜 갖고 있는지 — **호출부 미확인**(스폰 시 에이전트에게 가르치는 용도로 추정). Step 6이 이 상수를 건드리므로 그 전에 확인.
- ADR-0022를 확정으로 올릴지 후속 ADR로 갈지 — 이 TRD가 그 미해결 forks(레지스트리 위치·LLM 발견 경로·백엔드 미러)를 소진하므로 결정 시점에 `/adr`. **단 조사 보고서가 BLOCK이라 거부 대안 근거로는 못 쓴다.**
