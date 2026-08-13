# TRD — 통합 command 버스 (S20)

> 상태: **작성 중.** 사용자 방향(step-log.md:1651~1659) + 선례 조사(`docs/research/unified-command-bus-survey-2026-08-12.md`, 판정 **BLOCK**) 위에 "어떻게"를 세운다. 굵은 결정은 ADR로 박제하고, 미결 둘은 §8에 가정으로 남긴다(임의 확정 금지).
> 앵커: ADR-0022(방향) · ADR-0055(프론트 레지스트리 골격) · ADR-0064(메뉴 = command id 참조) · ADR-0081(릴레이 형태 — 이 문서는 **확장**이지 번복이 아니다) · ADR-0132 결정 7(후속 순서) · ADR-0129(net 격리) · CLAUDE.md 「LLM-우선 제어」.
> 선행 조건: **ADR-0081 릴레이 구현.** 실측상 `RegisterRole`·`RelayUi`·`UiCommand`·`UiResult`는 `.rs` 소스 hit 0이고 `src-tauri/src/daemon_client/`에 인바운드 처리가 없다. 이 TRD의 §6 Step 3 이후는 그 배선 위에 선다.

---

## 1. 목표 · 비목표

### v1이 하는 것

- **명령의 정적 계약을 Rust 한 곳에 선언한다** — 이름 · 인자 모양 · 반환 · 타입드 오류 · 주인 등급 · 읽기/쓰기 표식 · 카탈로그 버전. 선언처 = `engram-dashboard-protocol`(워크스페이스 의존 0 — `crates/engram-dashboard-protocol/Cargo.toml:8-13`에 `path=` 항목이 하나도 없다. 계약이 아무것도 끌어오지 않는다).
- **파생물 둘을 뽑는다** — TypeScript 바인딩(기존 ts-rs 경로)과 LLM용 JSON 스키마. 둘 다 기존 생성물 diff 게이트에 얹는다.
- **각 프로세스가 자기 표를 채우고 자기가 구현한 이름만 데몬에 보고한다**(런타임 가용성). 데몬은 소유자 토큰 → 이름 집합만 쥐고 "클라이언트 셸"이라는 구체 개념을 배우지 않는다.
- **대칭 봉투 하나로 전달한다** — `{ name, request_id, owner, proto_ver, args }`. 방향은 **어느 연결에 썼는가**로 정해지고 필드가 되지 않는다.
- **디스패치 규칙이 전 프로세스에서 같다** — `내 맵에 있으면 직접 실행, 아니면 봉투로 보낸다`.
- **사람 클릭과 중계된 에이전트 호출이 같은 핸들러 함수에 떨어진다.** 셸 소유 명령에서 그 함수 = ADR-0081이 이미 요구한 단일 적용 서비스.

### v1이 하지 않는 것 (명시)

| 안 하는 것 | 왜 |
|---|---|
| 값 읽기(조회)를 명령 목록에 담기 | §8 가정 B — 되돌릴 수 있는 쪽을 기본값으로 |
| 취소 · 진행 보고 | §4-⑤ — 계약 자리만 예약하고 기제는 안 만든다 |
| 키바인딩 커스터마이징 | ADR-0055가 이미 골격 밖으로 미뤄둔 것. 이 버스가 열어줄 뿐 v1 범위 아님 |
| 픽셀(위치·크기) 제어 | ADR-0132 결정 7 ③ — 레이아웃 디스크 영속 선행 |
| 다중 앱 인스턴스 | ADR-0081 「단일 앱 인스턴스 전제」 유지. 주인 토큰은 다중을 표현할 수 있으나 v1 정책은 last-wins 하나 |
| 40개 tauri command 전량 이관 | §6 — 23개는 남긴다(이유 거기) |

---

## 2. 명령 정의 형태

### 2-1. 어휘 근거 — 지금 실재하는 세 어휘

| 어휘 | 형태 | 실물 |
|---|---|---|
| 화면 | 점 구분 id | `theme.set`(`src/commands/themeCommands.ts:9`) · `tab.create`(`src/commands/tabCommands.ts:29`) · `agent.spawn`(`src/commands/agentCommands.ts:41`) — 총 33개 |
| CLI | 계열 + 동사 | `CLI_GROUP_AGENT="agent"`(`crates/engram-dashboard-core/src/agent/types.rs:237`) + `CLI_AGENT_VERBS=["list","spawn","new","rename","move"]`(`types.rs:246`) · `CLI_GROUP_MAIL="mail"`(`types.rs:179`) + `CLI_MAIL_VERBS=["send","status","pending"]`(`types.rs:212`) |
| wire | PascalCase variant | `AgentCommand::SpawnProfile`(`crates/engram-dashboard-protocol/src/messages.rs:124`) 외 25종(`messages.rs:25-214`) |

**카탈로그 이름 = 점 구분 `<계열>.<동사>`로 통일한다.** 화면 어휘를 정본으로 삼는 이유는 33개가 이미 그 형태로 안정 id를 쌓았고(ADR-0055 「안정적 id」), CLI 표면은 점을 공백으로 바꾸면 그대로 나오기 때문이다 — `agent.spawn` → `engram agent spawn`. 반대 방향(CLI 어휘를 정본)으로 가면 화면 33개 id를 전부 갈아야 한다. 이 규칙이 서면 step-log.md:1657 ②(화면 어휘와 데몬 어휘가 서로를 모른다)가 닫힌다.

### 2-2. 선언 구문 (제안)

`engram-dashboard-protocol` 안의 선언 전용 매크로. **새 crate·새 툴체인 0** — 조사 §3-①의 형제 도구(`tauri-specta`) 안은 새 crate·새 게이트인 데다 생성물이 앱 프레임워크 invoke 경로를 직접 부르므로(조사 §4 C5 `△`) 전송 교체 원칙과 충돌한다. 그래서 매크로를 쓴다.

```rust
// crates/engram-dashboard-protocol/src/commands/catalog.rs
engram_commands! {
    /// 에이전트를 띄운다(잠든 것 깨우기 포함).
    #[owner(Daemon)] #[effect(Write)] #[since(1)]
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
    ...
}
```

매크로가 만드는 것은 넷이다.

1. **인자·반환 struct** — `#[derive(Serialize, Deserialize, TS)] #[ts(export)]`가 붙는다. `messages.rs:12-13`의 기존 패턴 그대로라 파생 경로가 이미 신뢰하는 것과 같다.
2. **`CommandSpec` 정적 항목** — `COMMAND_CATALOG: &[CommandSpec]`에 들어간다.
3. **JSON Schema 문자열** — 매크로가 필드 이름·타입을 구문에서 읽어 그대로 찍는다. 새 의존 0. **대가: 허용 타입 알파벳이 닫힌다**(`String`·`bool`·`u32`/`i64`/`f64`·`Option<T>`·`Vec<T>`·카탈로그 안에서 선언된 struct/enum). 그 밖의 타입은 컴파일 에러로 막는다. 이 제약이 실무에서 좁은지는 **미확인** — Step 1에서 33개 + CLI 8동사를 실제로 선언해 보고 좁으면 그때 `schemars` 도입 여부를 사용자 결정으로 올린다.
4. **`CATALOG_VERSION`** — 선언 블록이 바뀔 때 손으로 올리는 상수. `PROTOCOL_VERSION`(`crates/engram-dashboard-protocol/src/lib.rs:58`, 현재 `3`)과 **별개 번호**다(§4-①).

`#[owner(...)]` 칸을 두는 근거는 §9의 모순 1을 볼 것 — 사용자가 한 번 철회시킨 칸이고, 여기서 되살아나는 이유가 거기 있다.

### 2-3. 주인 등급별 실례 1건씩

| 등급 | 예시 | 실행부가 사는 곳 | 근거 |
|---|---|---|---|
| `Daemon` | `agent.spawn` | `AgentManager`(데몬 프로세스). wire는 기존 `AgentCommand::SpawnProfile`(`messages.rs:124`)/`SpawnByCwd`(`messages.rs:98`) | 에이전트 제어는 전부 데몬에 있다(ADR-0132 근거절 실측) |
| `Shell` | `tab.create` | `src-tauri`. 오늘은 `#[tauri::command] create_tab`(`src-tauri/src/commands/layout.rs:97`)이 `ViewManager::create_tab`을 **직접** 부른다(`layout.rs:108`) | 레이아웃 권위 = `src-tauri`(ADR-0035/0057) |
| `View` | `theme.set` | 웹뷰. Rust 모듈이 없다 — **선언만 Rust에 두고 구현은 TypeScript**(`src/commands/themeCommands.ts:9`) | step-log.md:1653 — "C++ 헤더처럼 선언과 구현을 가른다" |

### 2-4. 파생물이 정확히 어떤 모양인가

**① TypeScript 바인딩** — 기존 경로 그대로. `crates/engram-dashboard-protocol/tests/ts_export.rs:9-20`이 `export_all_to("<crate>/bindings/")`를 부르고, 지금 23개 파일이 나온다. 카탈로그 struct가 `#[ts(export)]`를 달면 같은 호출에 딸려 나온다.

```ts
// bindings/AgentSpawnArgs.ts (생성물)
export type AgentSpawnArgs = { target: string | null, cwd: string | null, name: string | null, }
```

**② LLM용 JSON 스키마** — 같은 테스트가 한 파일 더 쓴다. 사람이 손대지 않는다.

```jsonc
// bindings/commands.schema.json (생성물)
{
  "catalogVersion": 1,
  "commands": [
    { "name": "theme.set", "owner": "View", "effect": "Write", "since": 1,
      "summary": "테마를 지정한 값으로 바꾼다.",
      "args": { "type": "object",
                "properties": { "theme": { "type": "string" } },
                "required": ["theme"] },
      "ok":   { "type": "object", "properties": {} },
      "errors": ["INVALID_ARGUMENT"] }
  ]
}
```

**③ CI 게이트 재사용** — CLAUDE.md 「CI」절의 ts-rs 바인딩 sync 게이트(`git diff --exit-code -- crates/engram-dashboard-protocol/bindings/`)가 두 파생물을 **같이** 덮는다. 새 게이트를 만들지 않는다. 조사 §3-① 「경고 2」가 "팀이 실제로 신뢰하는 방식은 생성물 diff 게이트"라고 짚은 그것이다.

---

## 3. 인터페이스

전부 `engram-dashboard-protocol`에 산다. **새 외부 의존 0** — 비동기 시그니처는 `std::pin::Pin` + `core::future::Future`로 적어 `futures` crate를 안 들인다.

### 3-1. 계약 표

```rust
pub enum OwnerClass { Daemon, Shell, View }
pub enum Effect { Write, Read }          // v1 카탈로그엔 Write 만 실린다(§8 가정 B)

pub struct CommandSpec {
    pub name: &'static str,
    pub owner: OwnerClass,
    pub effect: Effect,
    pub since: u32,
    pub summary: &'static str,
    pub args_schema: &'static str,       // JSON Schema 텍스트(매크로 생성)
    pub ok_schema: &'static str,
    pub errors: &'static [ErrorCode],
}

pub const COMMAND_CATALOG: &[CommandSpec];
pub const CATALOG_VERSION: u32;
pub fn spec_of(name: &str) -> Option<&'static CommandSpec>;
```

### 3-2. 봉투 (대칭 — 방향 필드 없음)

```rust
pub struct CommandEnvelope {
    pub name: String,                 // ★겉봉 — 데몬이 읽는다★
    pub request_id: RequestId,        // ADR-0081: 왕복 전 구간 동일. relay_id 없음
    pub owner: OwnerToken,            // 목적지 지시(주인 등급 아님 — 등록된 토큰)
    pub proto_ver: u32,               // 보낸 쪽 CATALOG_VERSION
    pub args: serde_json::Value,      // ★속 — 데몬 불투명★
}

pub struct CommandReply {
    pub request_id: RequestId,
    pub outcome: Result<serde_json::Value, CommandError>,
}
```

`name`을 겉봉에 두는 것이 ADR-0081의 순수 opaque 봉투로부터의 **유일한 형태 변경**이다. 근거: 이름이 겉봉에 있으면 데몬이 인자를 모른 채로도 **명령 단위 인가·관측**을 할 수 있고, 감추면 그게 전부 불가능해진다(조사 §2 — "명령 이름까지 불투명 봉투 안에 넣은 선례는 없었다"). `args`는 여전히 데몬이 파싱하지 않으므로 ADR-0081 「데몬 opaque 유지」의 본체는 산다.

### 3-3. 프로세스별 핸들러 표

```rust
pub type CommandFuture = Pin<Box<dyn Future<Output = Result<serde_json::Value, CommandError>> + Send>>;

pub trait CommandHandler: Send + Sync {
    fn call(&self, args: serde_json::Value) -> CommandFuture;
}

pub struct CommandTable { /* HashMap<&'static str, Arc<dyn CommandHandler>> */ }

impl CommandTable {
    pub fn insert(&mut self, name: &'static str, h: Arc<dyn CommandHandler>) -> Result<(), TableError>;
    pub fn get(&self, name: &str) -> Option<&Arc<dyn CommandHandler>>;
    pub fn names(&self) -> Vec<&'static str>;   // 데몬에 보고할 명단
}
```

- `insert`는 **중복 이름에 `Err`**를 낸다 — 조용한 덮어쓰기 금지(조사 §5-6: 선례는 있으나 그 선례가 후회한다). 프론트 `registry.ts:35-40`은 지금 warn 후 마지막 승리인데, 이건 HMR 재평가 때문이라 **프론트는 예외로 유지**하고 그 사유를 코드 주석에 남긴다.
- `insert`는 이름이 `COMMAND_CATALOG`에 없으면도 `Err`다 — 카탈로그 밖 이름을 등록하는 순간 두 번째 어휘가 생긴다.
- 화면 쪽 대응물은 이미 있다: `src/commands/registry.ts:29`의 `Map<string, Command>` + `register`(:35) / `run`(:46) / `list`(:67). **새로 만들지 않고 카탈로그 대조만 붙인다.**

### 3-4. 전송 seam

```rust
pub trait CommandLink: Send + Sync {
    /// 봉투를 상대 프로세스로 보내고 같은 request_id 의 답을 기다린다.
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>>;
}
```

구현체는 프로세스마다 하나다 — 데몬은 WS 연결, 셸은 `daemon_client`, 화면은 Tauri `invoke`. **`CommandLink`가 교체점**이라 전송 방식이 코드에 안 묶인다(CLAUDE.md 「아키텍처 원칙」).

### 3-5. 라우팅 (전 프로세스 동일)

```rust
pub async fn route(
    table: &CommandTable,
    link: &dyn CommandLink,
    env: CommandEnvelope,
) -> CommandReply;
```

규칙 한 줄: `table.get(&env.name)`이 있으면 직접 실행, 없으면 `link.send(env)`. 사람 클릭도 LLM 중계도 이 함수 하나를 지난다.

### 3-6. 인바운드 수신기

```rust
pub trait InboundCommands: Send + Sync {
    /// ★즉시 반환해야 한다★ — 적용은 연결 태스크 밖에서 돈다.
    fn on_command(&self, env: CommandEnvelope, reply: ReplySink);
}

pub struct ReplySink { /* oneshot 으로 request_id 에 상관 */ }
```

**연결 태스크 안에서 적용하면 합성 명령이 교착한다.** 실측: `crates/engram-dashboard-daemon/src/connection_core.rs:582`의 `dispatch`는 `crates/engram-dashboard-daemon/src/agent_conn.rs:217` → `crates/engram-dashboard-net/src/ws.rs:553`을 거쳐 **연결당 단일 read 태스크 안에서 인라인으로 `.await`**된다(spawn은 `ws.rs:393`의 연결당 1회뿐). 셸 쪽 `spawn_into`(`src-tauri/src/commands/layout.rs:403`)가 자기 안에서 `DaemonClient::send_command().await`를 부르므로, 이 명령을 연결 태스크에서 인라인으로 기다리면 자기 답을 자기가 못 꺼낸다 — ADR-0081 결정 3 개정이 잡아낸 self-deadlock 그대로다. `on_command`의 "즉시 반환" 계약이 그 회귀를 형태로 막는다.

### 3-7. 등록·발견 wire (가정 A의 실물)

```rust
// AgentCommand 에 additive 로 붙는 variant
RegisterCommands { owner: OwnerToken, names: Vec<String>, catalog_version: u32, request_id: RequestId }
ListCommands     { request_id: RequestId }

// AgentEvent 에 additive 로 붙는 variant
CommandList { request_id: RequestId, entries: Vec<CommandListEntry> }
// CommandListEntry = { name: String, owner: OwnerClass, available: bool }
```

**데몬이 나르는 것은 이름과 가용 여부뿐이다.** 설명·인자 스키마는 호출자가 자기 안의 `COMMAND_CATALOG`에서 붙인다(호출자도 protocol crate를 의존하므로 갖고 있다). 그래서 데몬은 명령의 **뜻**을 배우지 않고, 「데몬은 클라이언트 셸을 모른다」가 형식으로 산다. 재연결마다 `RegisterCommands`를 재전송하고 중복 owner는 last-wins — ADR-0081 「RegisterRole 재연결 재전송 + last-wins」와 같은 규칙이다.

---

## 4. 오류 · 진화 계약

조사 §9가 "이게 비면 어느 안도 완성 아키텍처가 아니다"라고 지목한 일곱 항목. 각 항목은 **규칙**과 **호출자가 보는 것**을 함께 적는다.

### ① 프로토콜 버전 — 번호가 둘이고 성격이 다르다

| 번호 | 무엇을 잰다 | 불일치 시 | 실물 |
|---|---|---|---|
| `PROTOCOL_VERSION` | 프레임·핸드셰이크 형태 | **연결 자체가 안 선다**(정확 일치 요구) | `crates/engram-dashboard-protocol/src/lib.rs:58` = `3` · 핸드셰이크 `AuthFrame{token, protocol_version}`(`crates/engram-dashboard-net/src/auth.rs:33`) · 응답 `AgentEvent::Hello{protocol_version,...}`(`messages.rs:220`) |
| `CATALOG_VERSION` | 명령 어휘의 세대 | **연결은 서고 개별 명령만 실패한다**(스큐 허용) | 신설(§2-2) |

★**명령을 하나 더한다고 `PROTOCOL_VERSION`을 올리지 않는다.**★ 올리면 additive 확장 하나가 모든 피어의 연결을 끊는다. 봉투의 `proto_ver`는 보낸 쪽 `CATALOG_VERSION`이고 **받는 쪽은 이 값만으로 거절하지 않는다** — 거절 판정은 이름 하나 단위(②)로 내린다. 근거: 최근접 피어(WezTerm)도 호환 불가 변경에만 코덱 버전을 올린다(조사 §9).

- **호출자가 보는 것:** 프레임 버전이 다르면 `PROTOCOL_MISMATCH`로 접속 실패(기존 경로 그대로). 카탈로그가 뒤처졌으면 접속은 되고 특정 명령만 `UNKNOWN_COMMAND`.

### ② 미지 명령 — 시끄럽게, 정확히 한 형태로

이름이 겉봉에 있어도 받는 쪽이 모를 수 있는 자리가 둘이다.

| 어디서 | 판정 | 코드 |
|---|---|---|
| 데몬 명부에 그 이름의 주인이 없다 | 아무도 구현 안 함 / 주인이 아직 등록 전 | `UNKNOWN_COMMAND`(카탈로그에도 없음) 또는 `OWNER_UNAVAILABLE`(카탈로그엔 있으나 등록 없음 — ④) |
| 봉투를 받은 주인 프로세스의 표에 없다 | 명부와 실제가 어긋남(주인이 옛 빌드로 재기동) | `UNKNOWN_COMMAND` |

조용한 무시·다른 명령으로의 폴백은 없다. 조사 §3-④가 정리한 세 형태 중 우리에 맞는 것은 ①(프로토콜 오류)뿐이다.

★**닫힌 열거형 금지**★ — 디코더는 `name`을 `String`으로 받는다. 생성된 TS도 명령 이름을 union 타입으로 좁히지 않는다. 조사 §3-① 「경고 1」이 든 LSP 사고(산문 규격은 미지 값 허용을 요구했는데 메타모델이 표식을 빠뜨려 엄격한 생성기가 닫힌 열거형을 만들고 클라이언트를 깨뜨림)를 그대로 밟지 않기 위해서다.

- **호출자가 보는 것:** `{ code: "UNKNOWN_COMMAND", message: "<name>", retry: "never" }`. LLM은 `commands.schema.json` 또는 `ListCommands` 결과를 다시 읽어 재조정한다.

### ③ 필드 추가 규칙 (additive)

| 해도 되는 것 | 하면 안 되는 것 |
|---|---|
| 인자 struct에 **기본값 있는 선택 필드** 추가 | 필드 이름 변경 |
| 반환 struct에 필드 추가 | 필드 타입 변경 |
| 카탈로그에 새 이름 추가 | 필드 제거 |
| 오류 코드 집합에 코드 추가 | 기존 이름의 **뜻** 변경 |

기계 강제 형태: 인자 struct의 신규 필드는 `#[serde(default)]`를 단다. 어떤 struct에도 `deny_unknown_fields`를 달지 않는다 — 새 호출자가 보낸 모르는 필드가 옛 주인을 깨면 additive가 성립하지 않는다.

★**경계 규칙**★: 옛 주인은 모르는 필드를 무시하고 **옛 의미로 실행한다.** 따라서 "필드를 더해 동작 의미를 바꾸는" 확장은 필드 추가가 아니라 **새 이름**이어야 한다. 이 선을 넘으면 호출자는 성공 응답을 받고도 의도와 다른 일이 일어난다.

명령 제거: 카탈로그에서 지우지 않고 `#[deprecated_since(N)]`를 단다. 파생 JSON 스키마는 그 항목을 광고에서 빼지만 핸들러는 최소 한 세대(`CATALOG_VERSION` +1) 동안 계속 답한다.

- **호출자가 보는 것:** 확장은 조용히 성공한다. 옛 필드만 보낸 호출자는 계속 돈다. deprecated 명령은 목록에서 사라지지만 이름으로 부르면 여전히 듣는다.

### ④ 소유자 부재 — 확실성을 코드에 인코딩한다

| 상황 | 코드 | 확실성 | `retry` |
|---|---|---|---|
| 카탈로그엔 있는데 등록한 주인이 없다 | `OWNER_UNAVAILABLE` | **미적용 확정** | `after-condition`(주인 기동) |
| 주인이 등록돼 있었는데 왕복 중 연결이 끊겼다 | `OUTCOME_UNKNOWN` | **불명** | `same-request-id` |
| 마감시각 초과 | `TIMEOUT` | **불명** | `same-request-id` |

★`retry` 플래그가 "불명 상태를 안전하게 재실행해도 된다"를 함의해선 안 된다★ — 안전한 무조건 재시도는 **미적용 확정**일 때뿐이다. 이 인코딩은 S17 TRD가 이미 세운 것(`docs/process/S17-llm-control-surface/spec/trd.md:56`, `:72`)이고 여기서 `APP_OFFLINE`을 주인 일반으로 넓힌 것뿐이다.

수명 관리는 ADR-0081 「relay 라우팅 표 수명 바운드 + 연결 cleanup sweep」을 그대로 쓴다 — 엔트리 `{request_id → 원 ConnId, deadline}`, evict 트리거 = 응답 수신 / 어느 쪽 연결이든 cleanup 시 그 ConnId 키 sweep / 타임아웃. **sweep이 없으면 ④의 "불명"이 "영원한 무응답"이 된다.**

- **호출자가 보는 것:** 실패의 종류가 아니라 **재실행해도 되는지**가 응답에 실려 온다.

### ⑤ 취소 · 진행 보고 — v1 미도입, 자리만 예약

기제를 만들지 않는다. 근거: 지금 장시간이라 부를 만한 명령이 없다 — `agent.spawn`은 프로세스를 띄운 시점에 답하지 에이전트가 준비될 때까지 기다리지 않는다.

예약 형태만 못 박는다.
- **취소 핸들 = `request_id`.** 별도 토큰을 만들지 않는다(ADR-0081의 단일 request_id 원칙). 열 때 `command.cancel { request_id }`.
- **진행은 답장이 아니라 이벤트다.** 하나의 `request_id`에 대한 `CommandReply`는 **정확히 하나**다. 중간 상태가 필요해지면 `AgentEvent` 쪽에 variant를 더한다. 이 선을 지금 그어두지 않으면 나중에 "답장이 여러 번 온다"가 되어 상관 로직이 깨진다.

- **호출자가 보는 것:** 진행 보고가 **없다는 것이 계약이다.** 오래 걸릴 수 있는 호출은 반드시 마감시각(⑥)에 기대야 하고, 중간 상태를 물어보려면 별도 조회 경로를 써야 한다(그리고 조회는 §8 가정 B로 v1에 없다 — 이 조합의 대가를 여기 적어둔다).

### ⑥ 타임아웃 · 재시도

- 마감시각은 **호출자가 정한다**(기본 10초 — S17 TRD `trd.md:72`의 `--timeout` 기본값 승계). 데몬 라우팅 표 엔트리가 같은 `deadline`을 들고 있어 호출자가 사라져도 표가 샌다.
- 초과 = `TIMEOUT`, 확실성 **불명**.
- **안전한 재시도는 같은 `request_id`로만 한다.** 새 id로 재시도하면 at-least-once가 되어 같은 조작이 두 번 적용될 수 있다.
- dedup 저장소는 S17 TRD가 이미 설계한 것을 그대로 쓴다(`trd.md:50-53`, `:72`): 완료분 = 캐시된 원 결과 재생 · in-flight 중복 = 같은 pending에 coalesce · 같은 id + 다른 페이로드 = `REQUEST_ID_CONFLICT` · 재시도 창 `retryWindowMs` 기본 300000(5분). 창 밖 같은 id = 신규 취급.
- ★쓰기 명령의 dedup 엔트리는 **성공 응답을 받은 뒤에만** 커밋한다★ — 사전 캐싱하면 타임아웃 때 거짓 성공이 캐시된다(ADR-0081 「dedup는 UiResult(적용 후)에만 커밋」).

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

- **명령마다 자기 부분집합을 카탈로그에 선언한다**(§2-2 `errors [...]`) → 파생 JSON 스키마가 그것을 광고하므로 LLM이 부르기 전에 실패 모양을 안다.
- ★wire의 `code`는 문자열이고 디코더는 미지 코드를 받아들인다★ — 모르는 코드는 `INTERNAL` + `retry: never`로 낮춰 다룬다. 여기서도 닫힌 열거형을 만들지 않는다(②와 같은 이유).
- `message`로 기계 분기하지 않는다. 지금 CLI가 하는 문자열 패턴매칭은 Step 2에서 걷어낸다.

- **호출자가 보는 것:** 실패가 코드 + 재시도 지시로 온다. 문자열은 사람이 읽는 자리로만 남는다.

---

## 5. 모듈 경계

| crate / 폴더 | 이 설계가 넣는 것 |
|---|---|
| `engram-dashboard-protocol` | 카탈로그 매크로 · `CommandSpec`/`COMMAND_CATALOG`/`CATALOG_VERSION` · `CommandEnvelope`/`CommandReply`/`CommandError` · `CommandTable`/`CommandHandler`/`CommandLink`/`InboundCommands` · `route()` · `AgentCommand`/`AgentEvent`의 additive variant 4종(§3-7) · 파생물 2종 |
| `engram-dashboard-daemon` | 주인 명부(토큰 → 이름) · 라우팅 표(ADR-0081 형태) · `connection_core.rs:582` `dispatch`에 새 variant arm · Daemon 소유 `CommandTable` |
| `src-tauri` | Shell `CommandTable` · **ADR-0081이 요구한 공유 적용 서비스**(오늘 없음) · `daemon_client` 인바운드 수신기(오늘 없음) |
| `src/commands/` | View `CommandTable` = 기존 `registry.ts` 그대로 + 카탈로그 대조 |
| `engram-dashboard-core` | v1엔 **변경 없음.** Step 6에서 `types.rs`의 CLI 동사 상수만 카탈로그 파생으로 바뀐다 |
| `engram-dashboard-net` | ★**변경 없음**★ |
| `engram-dashboard-messaging` | 변경 없음 |

### 각 격리 게이트에 미치는 영향

| 게이트 (CLAUDE.md) | 영향 |
|---|---|
| 코어 tauri import 0줄 | **없음.** 카탈로그·봉투 어디에도 tauri가 없다. `View`/`Shell` 명령의 *구현*은 코어 밖(웹뷰·`src-tauri`)이고 코어가 참조하지 않는다. (ADR-0003) |
| 메시징 커널 워크스페이스 crate 참조 0줄 | **없음.** 메시징은 이 설계를 모른다. (ADR-0110) |
| net 소스 참조 0줄(`daemon\|messaging\|discovery`) | **없음.** 새 심볼이 net에 들어가지 않는다 |
| **net — core 심볼 allowlist 정확히 2줄** | ★**바뀌지 않는다.**★ 허용된 둘은 `agent::platform::pid_alive_with_start_time`·`current_process_start_time`(portfile stale 판정 전용 — `crates/engram-dashboard-net/src/lib.rs:58-61`)이고, 이 설계는 core 심볼을 net에서 새로 쓰지 않는다 |
| **net — 직접 워크스페이스 의존 정확히 3줄** | ★**바뀌지 않는다.**★ 상한 3 = 자기 자신 + `engram-dashboard-core` + `engram-dashboard-protocol`(`lib.rs:65-68`). 카탈로그가 사는 곳이 **이미 그 3에 든 protocol**이라 새 의존이 생기지 않는다 |
| net — auth 어휘 재유입 금지(`AgentCommand`·`PROTOCOL_VERSION` 0 hits, `lib.rs:82-92`) | **없음.** 봉투는 `AgentCommand`의 variant로 실려 net에겐 **불투명 텍스트 프레임**이다 — `ws.rs:553`이 `handler.on_text(...)`로 넘길 뿐 안을 안 본다 |
| 생성물 sync 게이트 | **넓어진다**(파일 1개 추가). 게이트 명령은 그대로 — 디렉토리 단위 diff라 새 파일이 자동으로 덮인다 |

★net 게이트 둘이 안 바뀌는 이유를 한 줄로★: **이 설계는 네트워크 행에 아무 개념도 추가하지 않는다.** 명령 어휘는 protocol에서 시작해 daemon·src-tauri·웹뷰에서 끝나고, net은 그 사이를 나르는 바이트 통로로만 참여한다(ADR-0129의 계층 분리 그대로).

---

## 6. 마이그레이션

각 Step은 **혼자 배포 가능하고 혼자 검증 가능**하다. 앞 Step이 뒤를 안 깬다.

### Step 0 — 선행(이 TRD 밖): ADR-0081 릴레이 구현

실측 근거: `RegisterRole`·`RelayUi`·`UiCommand`·`UiResult` 소스 hit 0 · `src-tauri/src/daemon_client/`(`connection.rs`·`lifecycle.rs`·`mod.rs`·`protocol_state.rs`·`replay_flight.rs`)에 인바운드 처리 0. 이게 없으면 Shell·View 명령은 에이전트가 부를 수 없다. ADR-0132 결정 7이 이미 이 순서를 못 박았다.

### Step 1 — 계약만 (배선 0)

매크로 + 카탈로그 + 파생물 2종 + CI diff 게이트. 아무도 아직 이걸 안 쓴다. 첫 입주자는 이미 CLI에 실재하는 동사들(`types.rs:246` 5개 + `:212` 3개)과 `src-tauri/src/commands/layout.rs`의 쓰기 12개(16개 중 읽기 4개 제외 — §8 가정 B), 그리고 화면 33개 중 View 소유분.
- **검증:** `cargo test -p engram-dashboard-protocol` · `git diff --exit-code -- crates/engram-dashboard-protocol/bindings/`.

### Step 2 — 데몬: 자기 표 + 명부 + `command.list`

Daemon 소유 명령이 `CommandTable`을 지나게 하고, `RegisterCommands`/`ListCommands`를 `connection_core.rs:582` dispatch에 additive arm으로 더한다. `engram` CLI(`crates/engram-dashboard-daemon/src/bin/engram.rs`)가 이름으로 부르게 바꾸고, 문자열 패턴매칭 오류 합성(S17 TRD `trd.md:69`)을 타입드 오류로 교체한다.
- **검증:** 데몬 단위 테스트(§7) · `engram agent list`/`spawn`/`new`/`rename`/`move` 동작 불변.

### Step 3 — 셸: 공유 적용 서비스 + 인바운드 수신기

★**이 Step이 가장 무겁다**★ — ADR-0081이 요구한 서비스가 **오늘 없다.** `src-tauri/src/commands/layout.rs`의 핸들러들이 `LayoutState`를 잠그고 `ViewManager` 메서드를 직접 부른다(`layout.rs:108`·`:145`·`:227`). ADR-0081 「거부한 대안」이 명시적으로 거부한 그 형태다.
1. 쓰기 12개를 전송 중립 적용 서비스 뒤로 옮긴다.
2. `#[tauri::command]`는 그 서비스를 부르는 얇은 껍데기가 된다 — 사람 클릭 경로는 그대로.
3. `daemon_client`에 인바운드 수신기를 달고 **적용은 액터 밖(spawn)에서** 돈다(§3-6).
- **검증:** 사람 클릭 GUI 실측(`scripts/cdp.mjs`) + 중계 왕복 + §7의 교착 회귀 테스트.

### Step 4 — 화면: 표 보고 + 대조

`registry.ts`는 손대지 않는다. 부팅 시 `list()` 결과를 카탈로그의 View 소유 이름과 대조하고, 셸 경로로 데몬에 보고한다.
33개의 등급 분류는 **handler가 어디로 라우팅하는가**로 정한다. id는 안정 id라 바꾸지 않는다(ADR-0055).

| 등급 | 해당 id (분류 근거 = handler의 라우팅 대상) |
|---|---|
| `View` | `theme.set`·`theme.toggle`(`themeCommands.ts:9`,`:22`) 등 순수 store 조작 |
| `Shell` | `tab.*`·`window.*`·`slot.*`·`layout.setSlotContent`·`agent.spawnInto` — `invoke`로 `src-tauri`에 간다(`tabCommands.ts:29~181`·`slotCommands.ts:48~98`) |
| `Daemon` | `agent.spawn`(`agentCommands.ts:41`)·`agent.rename`(`:76`)·`agent.kill`(`slotContentCommands.ts:118`)·`preset.*`(`presetCommands.ts:14~69`) — `agentClient`로 데몬에 간다 |

- **검증:** `npm test`에 카탈로그↔`list()` 대조 테스트 추가(어휘 drift를 CI가 잡는다).

### Step 5 — 곁문 둘 철거

| 핸들 | 실물 | 처리 |
|---|---|---|
| `window.__engramLayout` | `src/store/eventBus.ts:80-111`, 메서드 15개. 자기 주석이 "정식 command 버스 전까지의 임시 경로"(`eventBus.ts:70-71`)·`setSlotContent`는 "`layout.setSlotContent`와 병행 경로"(`:93-95`)라 자인 | command로 안 덮인 것(`setRenderMode`·`clearRenderMode`·`enableDomMode`·`disableDomMode`·`toggleDomMode`·`moveSlotToWindow`)을 **먼저 command로 승격**한 뒤 핸들 삭제 |
| `window.__engramChat` | `eventBus.ts:118-124`, 5항목 | `set`·`patch`·`reset`은 command로 승격. ★`get`(`:119`)·`defaults`(`:123`)는 **읽기**라 §8 가정 B에 걸려 v1 범위 밖 → **핸들을 통째로 못 지운다**★ |
| `window.__engramCmd` | `eventBus.ts:134-137` | **남긴다.** 이게 View 표의 로컬 입구다 |

가정 B의 실제 대가가 여기 드러난다 — "곁문 둘을 없앤다"가 v1에선 "하나 반"이 된다.

### Step 6 — CLI 어휘를 카탈로그에서 파생

`types.rs:246`의 `CLI_AGENT_VERBS` 같은 손 배열을 카탈로그 생성물로 바꾼다. **마지막이어야 한다** — 앞 Step이 서기 전에 하면 CLI가 없는 핸들러를 가리킨다.

### v1에서 이관하지 않는 것

`src-tauri`의 40개 `#[tauri::command]`(`src-tauri/src/lib.rs:147-188`) 중 **23개는 남긴다**: `commands/agent.rs` 9개(이미 `AgentCommand` 직송의 얇은 래퍼) · `commands/discovery.rs` 9개 · `commands/autostart.rs` 2개 · `commands/tray.rs` 3개(클라이언트 로컬 생애주기). 이유: 전자는 Daemon 소유 명령의 중복 표면이 되고, 후자는 "데몬이 없을 때도 눌러야 하는 것"이라 버스에 태우면 순환이 생긴다. 필요해지면 `Shell` 소유로 additive.

---

## 7. 테스트 전략

ADR-0012 — 모든 조각이 외부 의존을 seam으로 끊고 단독 하네스를 갖는다.

| 조각 | 끊는 seam | 하네스 |
|---|---|---|
| 카탈로그 파생 | 없음(순수 매크로 확장) | `cargo test -p engram-dashboard-protocol` — 이름·주인·오류 집합 골든 + `tests/ts_export.rs:9-20` 확장 + `bindings/` diff 게이트 |
| `CommandTable` | 없음(순수 자료구조) | 중복 삽입 = `Err` · 카탈로그 밖 이름 = `Err` · 미지 조회 = `None` |
| `route()` | **`CommandLink`** | `FakeLink`(보낸 봉투를 기록하고 미리 정한 답을 돌려준다) — 소켓 0으로 "내 맵에 있으면 직접, 없으면 봉투" 규칙을 단언 |
| 인바운드 수신기 | **`ReplySink` + spawn 주입** | ★교착 회귀 테스트★: 핸들러 안에서 같은 링크로 두 번째 명령을 부르는 fake를 넣는다. 인라인 실행이면 hang, spawn이면 통과. **지금 이 회귀를 잡는 테스트가 없다** — ADR-0081 결정 3의 self-deadlock을 형태로 고정한다 |
| 데몬 명부 | **`OutboundSink`**(`connection_core.rs:582` dispatch가 이미 `&dyn OutboundSink`를 받는다 — 새 seam 불요) | 등록 · last-wins · 주인 부재 = `OWNER_UNAVAILABLE` · 연결 cleanup sweep |
| 셸 적용 서비스 | **`AppHandle`·`State<..>` 제거** | ★난점★: 오늘 핸들러는 `AppHandle` + `State` 4개를 받아(`layout.rs:98`·`:122`·`:215`) 단독 호출이 불가하다. 적용 서비스는 그 넷을 인자에서 걷어낸 순수 함수여야 단독 테스트가 선다. **Step 3의 실제 난이도가 여기다** |
| 화면 표 | **`__resetRegistryForTest`**(`src/commands/registry.ts:77-79` — 이미 있다) | `npm test`(vitest)에 카탈로그 JSON ↔ `list()` 대조 추가 |

CI는 push마다 위 명령을 windows 러너에서 돌린다(CLAUDE.md 「CI」). 로컬 몫으로 남는 것은 GUI 실측(Step 3·5) 하나다.

---

## 8. 가정 (사용자 결정 대기)

메인이 임의 확정하지 않는다. 아래 둘은 **지금 내가 걸어둔 기본값**이고, 뒤집으면 이 문서의 어느 절이 바뀌는지 함께 적는다.

### 가정 A — 목록 취합을 데몬이 받는다

**기본값:** 각 프로세스가 시작 시 자기가 구현하는 이름만 데몬에 등록하고(`RegisterCommands`, §3-7), 데몬이 합쳐 `ListCommands`로 낸다.

**이유:** 호출자가 셋 이상이다 — `engram` CLI · MCP 우편 경로 · 화면. 셋이 각자 발견·병합 로직을 복제하면 어휘가 또 갈리고, 그건 지금 고치려는 증상 자체다(step-log.md:1657 ②). 데몬은 이름과 소유자 토큰만 쥐므로 "클라이언트 셸"이라는 구체 개념을 배우지 않는다.

★**숨기지 않는 긴장**★ — 조사 보고서는 이 기본값 쪽에 서 있지 않다.
- §5-3이 "데몬을 목록 취합자로 승격 = C1(데몬은 셸을 모른다) 파괴"를 **거부 대안**으로 적었다.
- §8 3위(호출자 측 발견·병합)를 두고 "데몬의 셸 무지를 **가장 깨끗하게 보존**한다"고 판정했다.
- §3-③ 반례 (가)의 탈중앙 발견 선례(ROS 2 — 중앙 마스터 없이 임의 로컬 노드가 발견한 그래프에서 전체 서비스 목록을 조회)가 **실제로 대안 쪽을 지지한다.** 이 선례는 조사 초판의 "선례 없음"을 반증하며 등장한 것이라 무게가 가볍지 않다.

기본값이 이기는 근거는 **선례 우위가 아니라 호출자 수**다. 선례만 보면 대안이 앞선다 — 그 사실을 여기 적어둔다.

**뒤집으면 바뀌는 절:** §3-7(등록 wire가 사라지고 `QueryCommands{owner}` + 호출자 측 병합기가 들어온다) · §4-④(데몬이 명부를 안 가지므로 `OWNER_UNAVAILABLE`을 못 낸다 → 전부 호출자 타임아웃으로 격하, 즉 **미적용 확정이 사라지고 전부 불명이 된다**) · §5(공용 발견 클라이언트가 새 모듈로 필요 — 어느 crate에 둘지가 새 질문) · §6 Step 2(데몬 명부 대신 호출자 병합기) · §7(데몬 명부 하네스 → 병합기 하네스).

### 가정 B — 값 읽기(조회)는 v1 명령 목록에 넣지 않는다. 실행만 담는다

**기본값:** `Effect::Read` 표식은 타입으로 **존재하되 v1 카탈로그엔 항목이 없다.**

**이유:** 되돌릴 수 있는 쪽이 기본값이다. 나중에 읽기를 넣는 것은 §4-③의 additive 규칙이 그대로 덮는 덧붙이기지만, 넣었다 빼는 것은 계약 파괴다.

**실제 대가(추정 아님, 실물):**
- `src-tauri/src/commands/layout.rs`의 읽기 4종 — `get_view`(`:496`) · `list_tabs`(`:507`) · `list_windows`(`:519`) · `resolve_spatial`(`:529`) — 이 v1에서 command가 되지 않는다.
- `__engramChat`의 `get`(`eventBus.ts:119`)·`defaults`(`:123`) 때문에 §6 Step 5에서 그 핸들을 **통째로 못 지운다.**
- §4-⑤(진행 보고 없음)와 겹쳐, 오래 걸리는 조작의 중간 상태를 물어볼 경로가 v1에 아예 없다.

조사 §3-②는 이 갈림길에서 **수렴하지 못했다** — 합치는 쪽이 다수(LSP·D-Bus·K8s·sway·WezTerm·CDP·OBS)이고, 가르는 쪽은 실행 의미가 달라서 갈랐다(GraphQL의 query/mutation, MCP의 Resources/Tools). 조사가 승자를 못 고른 자리라 기본값은 "되돌릴 수 있는 쪽"이라는 원칙 하나로 정했다.

**뒤집으면 바뀌는 절:** §2-2(카탈로그에 `#[effect(Read)]` 실항목이 생기고 위 6개가 Step 1 입주자에 합류) · §4-⑥(읽기는 멱등이라 dedup 면제 규칙이 추가된다 — 지금은 전 명령이 dedup 대상) · §6 Step 5(곁문 둘을 v1에 **전부** 철거 가능) · §7(읽기 명령의 read-your-writes 검증 추가) · §1 비목표 표에서 해당 행 삭제.

---

## 9. 근거

### 출처

- **선례 조사:** `docs/research/unified-command-bus-survey-2026-08-12.md`
- **사용자 방향(2026-08-12):** `docs/process/step-log.md:1651-1659`
- **결정:** ADR-0022(방향 — 상태 **제안**) · ADR-0055(프론트 레지스트리 골격) · ADR-0064(메뉴 = command id 참조) · ADR-0081(릴레이 형태) · ADR-0132 결정 7(후속 순서) · ADR-0129(net 3층 분리) · ADR-0035/0057(레이아웃 권위) · ADR-0012(테스트 격리) · ADR-0003(코어 격리)
- **선행 TRD:** `docs/process/S17-llm-control-surface/spec/trd.md` — §4-④/⑥/⑦의 확실성 인코딩·dedup·오류 코드는 **거기서 이미 결정된 것을 승계**한 것이지 새로 만든 것이 아니다.

### ★조사 보고서를 얼마나 믿을지 (후속 독자 경고)★

**판정은 BLOCK이다.** 보고서 스스로 "§8·§9의 열린 항목이 닫히기 전까지 ADR 거부 근거로 사용 금지"라고 적었다(조사 §6 첫 항목). 아래는 적대 리뷰에서 **반증된** 주장들이라, 이 문서들만 읽고 되살리지 말 것.

| 조사 초판 주장 | 상태 |
|---|---|
| Zellij가 "단일 `Action` 열거형 = 유일 명령 어휘"의 완전 동형 선례 | **반증** — 플러그인 API가 별도 함수 표면을 대규모로 갖는다. 부분 선례로 강등 |
| OBS는 외부 미도달(D계열 반면교사) | **반증** — OBS 28+ 는 WebSocket 제어 API를 기본 포함. D계열 예시는 VS Code만 |
| 권한 검사는 등록 시 한 번이 흔하다 | **반증** — 매 호출 검사가 조사 표본의 표준. ★이 반증이 이 설계의 「인가는 호출마다」를 지지한다★ |
| 비중앙 당사자가 목록을 합치는 기제의 선례 없음 | **반증** — ROS 2의 탈중앙 발견. ★이 반증은 §8 가정 A의 **대안** 쪽을 지지한다★ |
| "생성기를 두고 이름만 생성하는 성숙 사례 0건" | **표본 한정** — 검색 범위 미제시. 이 문장 단독으로 거부 근거 불가 |
| 형제 도구(`tauri-specta`)가 "정확히 그 범위를 덮는다" | **정정** — 생성물이 앱 프레임워크 invoke 경로를 직접 부른다(전송 교체와 충돌). §2-2가 매크로를 택한 이유 |

grounding 전수는 **미실시**이고, 리뷰 스팟체크 5건 중 4건이 NOT SUPPORTED로 나왔다(조사 §6 한계 2).

### ★이 설계와 기존 기록이 어긋나는 자리 3건 (평탄화하지 않음)★

**모순 1 — 주인 칸.** step-log.md:1653은 "**주인 지정은 불필요 — 정의 위치가 곧 주인이다(사용자 지적으로 메인 제안 철회)**"라고 기록돼 있는데, 이 설계는 `#[owner(Daemon|Shell|View)]` 칸을 되살린다.
되살리는 근거는 같은 줄의 뒷문장이다 — "화면 전용 명령은 Rust 모듈이 없어 목록에서 빠지므로 **선언만은 Rust에 자리를 준다**(C++ 헤더처럼 선언과 구현을 가른다)". 선언을 전부 protocol crate 한 곳에 모으는 순간 **"정의 위치"가 주인을 더 이상 말해주지 않는다** — `theme.set`도 `agent.spawn`도 같은 파일에 적히기 때문이다. 즉 칸이 되살아난 것은 제안 재탕이 아니라 **선언 중앙화의 귀결**이다. 이 논거가 사용자 판단을 못 넘으면 대안은 "주인별로 선언 파일을 가르고 파일 경로가 주인을 말하게 한다"이며, 그 경우 §2-2 매크로 형태와 §5 crate 배치가 바뀐다.

**모순 2 — ADR-0081 「Opaque 결합 가드」.** ADR-0081 영향절과 S17 TRD `trd.md:36`은 "UI 명령 enum + payload→`ViewCommand` dispatch 맵은 **오직 `src-tauri`**에 산다 · `engram-ctl`·데몬 relay·`core`는 UI enum을 **import하지 않는다**"라고 못 박았다. 이 설계는 Shell·View 명령의 **이름과 인자 스키마를 `engram-dashboard-protocol`에 둔다** — 데몬과 CLI가 그 crate를 의존하므로 가드의 문자 그대로와 충돌한다.
충돌하지 않는 부분과 하는 부분을 갈라 적는다: **`args`는 여전히 데몬에게 불투명**하고 데몬은 `request_id → ConnId` 표만 유지하므로 ADR-0081 결정 2의 본체는 산다. 어긋나는 것은 ① 이름이 겉봉으로 나온 것(조사 §2가 지지 — 이름이 겉봉에 있어야 명령 단위 인가·관측이 가능) ② 선언 위치. **가드 H는 이 TRD가 확정되면 부분 폐기 도장이 필요하다** — 그 판단은 사용자 몫이고 여기서 대신 찍지 않는다.

**모순 3 — ADR-0055의 탈중앙 논거.** ADR-0055는 "enum-데이터 단일 커맨드(variant + 중앙 dispatch)"를 거부하며 근거로 "**새 커맨드마다 중앙 enum + match + 메타맵을 수정** → 기능이 자기 command를 스스로 등록하는 탈중앙 목표에 위배"를 들었다. 카탈로그를 protocol crate 한 곳에 모으는 이 설계는 그 중앙화를 일부 되불러온다.
완화 근거는 둘이다 — ① 매크로 한 블록이 enum·메타·스키마를 **동시에** 만들어 "세 곳 수정"이 "한 곳 수정"이 된다 ② 실행부 등록은 여전히 탈중앙이다(각 프로세스가 자기 `CommandTable`을 채운다). 그래도 **선언은 중앙이 됐고**, ADR-0055가 그것을 거부 사유로 적었다는 사실은 남는다. ADR-0055의 "백엔드 미러 = 후속" 조항이 이 작업을 예정해 뒀으므로 번복이 아니라 **그 후속의 실물**이지만, 거부 논거 한 줄과는 어긋난다.

### 미확인 (추측으로 메우지 않음)

- 매크로가 직접 찍는 JSON Schema의 타입 알파벳이 실무를 덮는지 — Step 1에서 실측(§2-2).
- 33개 화면 command 중 View/Shell/Daemon 등급 분류의 경계 사례 유무 — §6 Step 4의 표는 handler 라우팅 대상 기준의 1차 분류이고, 개별 검증은 이관 시점.
- ADR-0022를 확정으로 올릴지 후속 ADR로 갈지 — 이 TRD가 그 미해결 forks(레지스트리 위치·LLM 발견 경로·백엔드 미러)를 소진하므로 결정 시점에 `/adr`.
