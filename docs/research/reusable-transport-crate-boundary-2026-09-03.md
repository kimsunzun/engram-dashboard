# 재사용 전송 crate 의 경계 — 「패킷 정의만 주면 되는」 형태와 다중 접속 소유 모델

- **상태:** 조사 완료 · 적대 리뷰 **반영 완료**(판정 **BLOCK** → 적출 반영 · §9) · ★**재판정은 안 돌렸다**★ — 수정본에 대한 2차 리뷰는 미실행이다
- **방법:** `/research` deep · 설계-결정 모드 · 수집자 5(내부 제약 1 + 외부 4, 병렬·상호 비공개) · grounding = 메인이 1차 출처 직접 대조 · 적대 리뷰 = cross-family
- **날짜:** 2026-09-03
- **확신도 범례:** `확실`(원문 대조 + 독립 교차확증) · `가능성 높음`(원문 대조, 단일 출처) · `불확실`(미검·부재 확인) · `출처 없음`(인용 금지)

> ★**이 문서는 결정이 아니다.**★ 굵은 설계라 선택은 사용자가 한다. 여기 있는 것은 선택지와 그 대가다.

---

## §0 한 줄 결론

★**타입 seam 과 연결 수명을 둘 다 소유한 라이브러리는 존재한다. 못 찾은 것은 그 위에 「정체성이 각자 다른 피어 N 대의 레지스트리」까지 얹은 것이다.**★

- **반증된 옛 명제:** 「타입 seam 소유 진영과 수명 소유 진영이 둘로 갈리고 합친 물건이 없다」는 **거짓이다.** 반례 둘을 원문 대조했다 — `bevy_simplenet` 은 소비자가 `ChannelPack` trait 으로 **구체 메시지 타입을 정의**하게 하면서(*"Messages are automatically serialized and deserialized, so concrete types are required"*) 동시에 연결 수락(`AcceptorConfig`)·인증(`Authenticator::None/Secret/Token`)·**자동 재연결**·수명 사건(`ClientReport`/`ServerReport`)을 crate 가 소유한다. **Bevy 에 묶이지도 않는다** — bevy 는 `Resource` derive 를 더하는 optional feature 다. `tonic` 도 생성된 typed 메시지에 재연결·connect/serve·keepalive·엔드포인트 밸런싱을 함께 얹는다(고정 IDL 은 *대가*이지 부재가 아니다). `확실`(원문 대조)
- **살아남는 좁힌 명제:** `bevy_simplenet` 문서는 **한 클라이언트가 서로 다른 서버 여러 대에 붙는 형태를 다루지 않는다.** 즉 못 찾은 것은 「타입 seam + 수명」 조합이 아니라 **거기에 N-피어 레지스트리와 피어별 정체성까지 실은 것**이다. `불확실`(부재 확인 — 근거 강도는 약하다)

그래서 **「재사용하려면 I/O 를 포기해야 한다」는 인과도 성립하지 않는다.** 실제로 재사용된 것들은 그 I/O 층을 **안 팔았거나 따로 팔았을** 뿐이고, 층으로 겹치는 것 자체가 배제되지는 않는다.

**설계 질문은 「두 진영 중 어느 쪽을 고르나」가 아니라 「타입 seam·수명·N-피어 레지스트리 셋을 한 crate 안에서 어떻게 층으로 겹치나」다.** 후보 넷이 그 겹치는 방식으로 갈린다(§3).

---

## §1 선행 조사와의 관계 — 겹침·정정

★**착수 전 `docs/research/` 전수 확인을 했다.**★ 직전 세션이 이 절차를 건너뛰어 같은 주제를 두 번 돌린 실패의 처방이다.

### 1-1. 겹침 지도

| 선행 문서 | 판정 | 무엇이 겹치나 |
|---|---|---|
| `crate-granularity-survey-2026-08-17` | **겹침(권위)** | 「새 crate 를 파도 되는가」의 판정 기준 정본. 이 문서의 신규 crate 정당화는 그 기준을 통과해야 한다 |
| `module-split-criteria-2026-08-26` | **부분 겹침** | 같은 질문의 확장. **§9(적대 리뷰)가 본문 여러 줄을 정정하므로 본문 단독 인용 금지** |
| `client-identity-survey-2026-08-17` | **부분 겹침** | 핸드셰이크·재부착 소유권. **단 그 문서는 결론이 없다** — 위협 모형 미문서 상태로 "이 2판만으로 결정하지 말 것"이라 명시 |
| `daemon-version-mismatch-2026-08-14` | **겹침** | 버전 협상 소유층. 확정 결정은 ADR-0158 로 넘어갔고 이 문서가 그것을 읽었다 |
| `multi-window-layout-authority-topology-2026-06-27` | **부분 겹침** | 그 문서가 미결로 남긴 Q-B 를 **ADR-0036 이 이미 결정했다**(아래 1-3) |
| `agent-messaging-survey-2026-06-28` | **부분 겹침** | control/data plane 논리 분리 + transport seam 권고 |
| `unified-command-bus-survey-2026-08-12` | **부분 겹침** | 제약 「전송에 코드를 묶지 않는다」가 같은 판정축 |
| `tauri-channel-multiwindow-carrier-2026-06-28` | **겹침 없음** | in-proc carrier 얘기. 네트워크 전송 경계와 무관 |

### 1-2. 무너진 전제 — 근거로 인용 금지

- ★**「의존 방향을 강제하려면 crate 로 쪼개야 한다」 = 거짓**★ — ADR-0151 결정 4 가 그렇게 명시했고, 이 저장소는 이미 `rg` 격리 게이트 + `cargo tree` 상한 게이트로 방향을 강제하고 있다. `확실`
- ★**「crate 로 빼려면 두 번째 소비자가 있어야 한다」 = 반례로 깨짐**★ — rust-analyzer 가 소비자 하나뿐인 `ide` 를 API 경계 crate 로 삼는다. ADR-0151 결정 1 = "지금 실제 소비자가 있는지는 묻지 않는다". `가능성 높음`(반례는 실물이나 「규칙이 없다」 쪽은 부재 확인)
- **살아 있는 판정 기준 = 「독립적으로 쓸 수 있는가」 + 「무게」**(ADR-0175 결정 6 이 무게 축을 더했다: *lib 은 무게를 서로 비슷하게 맞추고 파일 하나짜리 lib 은 만들지 않는다*). `확실`

★**정정 — 「rule of three」는 위 두 명제와 다른 것이고, 무너지지 않았다.**★ 이 세션이 조사 지시서에 그것을 「무너진 전제」로 적었는데 틀렸다. 「소비자 수를 세라」(무너짐)와 「일반화 전 실사용 사례를 보라」(안 무너짐)는 다른 명제다.

**단 그 숫자 「3」 자체는 출처가 없다.** 계보 조사가 Biggerstaff(1987) → Lanergan & Grasso(1984) → Lanergan & Poynton(Raytheon, 1978/79)까지 되짚고 *"there's no reason that it shouldn't be the 'Rule of Five' or the 'Rule of Seven' instead"* 로 folklore 판정했다. 방향에는 named source 가 있다(Roberts & Johnson 1996, *Evolving Frameworks* 의 첫 패턴 "Three Examples"). ★**권고 문안 = 숫자가 아니라 조건으로 적을 것**★ — "세 번째에 추출" 대신 "실제 두 번째 소비자가 코드로 존재하기 전에는 경계를 확정하지 않는다". `출처 없음`(숫자) / `가능성 높음`(방향)

### 1-3. 이 조사가 되찾은 결정 둘 — 설계 전제가 바뀐다

★**정정 1 — 레이아웃 권위는 데몬이 아니라 셸(src-tauri)이다.**★ 선행 조사가 「데몬 권위」를 권고했으나 **ADR-0035 가 그 권고를 기각**했다. 그리고 그 ADR 이 계열을 못 박는다 — *tmux/Wezterm Mux 가 레이아웃을 서버에 두는 건 거기선 pane==PTY 이기 때문이고, engram 엔 그 전제가 없다. 우리는 tmux 모델이 아니라 **에디터 모델**에 속한다.* `확실`
→ **함의:** 레이아웃 축에 한해 터미널 멀티플렉서 계열은 우리 피어가 아니다. (전송·세션 생존 축에서는 여전히 피어다 — 축을 갈라 읽을 것.)

★**정정 2 — 선행 조사가 미결로 남긴 Q-B 는 이미 결정됐다(ADR-0036).**★
> "**src-tauri 가 데몬과 단일 WS 연결을 쥐고**, 자기가 소유한 레이아웃 라우팅 테이블로 각 창에 출력을 fan-out 한다. 창은 src-tauri 하고만 IPC — **데몬 직결 0.**"
> 불변식: "**단일 choke point**" · "**창↔데몬 직결 금지**" · "**데몬은 클라 수를 모르고 1 연결만 본다.**"

폐기 도장 없음, 후속 번복 ADR 없음(전수 grep). `확실`

★**그래서 사용자의 「다중 데몬 접속」 요구는 이 결정 위에 선다.**★ 확실한 것 하나 = **N 개 연결을 쥐는 주체는 창이 아니라 셸이어야** 이 결정과 안 부딪친다. **셸이 데몬 A·B 에 각각 1 연결씩 쥐는 형태가 이 결정을 위반하는지 확장하는지는 문서가 답하지 않는다** — 「1 연결만 본다」가 데몬 한 대 기준의 서술이기 때문이다. ★**이것이 §10 의 사용자 결정 1 이다. 세션이 임의 판정하지 않는다.**★

---

## §2 시장 관측 — 왜 원하는 물건이 없나

### 2-1. 두 쏠림 — 단 배타는 아니다

**타입 seam 쪽** — 소비자가 타입만 주면 typed 채널을 준다. 대신 **연결 수립을 안 한다.**
- `remoc` — 소비자는 `serde` derive 만 붙인다. `AsyncRead+AsyncWrite` **또는** `Sink+Stream` 위에 얹힌다(WebSocket 이 후자로 그대로 꽂힌다). 원문: *"Remoc implements no transport itself and thus depends on no networking crate; it runs over any byte stream you already have."* — **byte stream 수립까지가 소비자 몫이다.** ★**정정 — 「재연결·keepalive 가 없다」는 거짓이었다**★: `chmux::Cfg::connection_timeout` 이 *"Time after which the connection is closed when no data is received. Pings are send automatically when this is enabled"* 로 **자동 ping 을 켜고**(기본값 150초), 0.20 문서는 **재연결하는 resilient transport** 를 adaptable transport 의 하나로 문서화하며 typed remoc 채널이 그 위에 그대로 선다. 남는 참인 것은 **최초 byte stream 수립이 소비자 몫**이라는 것 하나다. `확실`(원문 대조)
- `tarpc` — transport 요건이 `Stream<Item=Request> + Sink<Response>` 뿐. serde 조차 optional feature(*"in-memory transports can be used, as well, so the price of serialization doesn't have to be paid when it's not needed"*). `확실`

**수명 쪽** — 재연결·heartbeat 를 해 준다. 대신 **타이핑을 하나도 안 한다.**
- `ezsockets` — *"Automatic reconnection of WebSocket Clients"*, *"Heartbeat mechanism to keep the connection alive"*, `client`/`server` feature 분리. 그런데 메시지 타입이 제네릭이 아니라 **구체 타입 고정**(`Utf8Bytes`/`Bytes`)이라 역직렬화는 전부 소비자 몫. `확실`

★**단 이 쏠림은 배타가 아니다 — 둘을 겹친 실물이 있다**★(`bevy_simplenet`·`tonic`, §0). 못 찾은 것은 거기에 **N-피어 레지스트리**까지 얹은 것뿐이다. `불확실`(부재 확인)

### 2-2. 근접한 것들이 치른 대가 — 전부 「축 하나 고정」

| 라이브러리 | 고정한 축 | 그래서 잃은 것 |
|---|---|---|
| `zbus`(D-Bus) | **wire 프로토콜** | 프로젝트 중립 아님. 단 seam 형태(`#[proxy]` trait / `#[interface]` impl 이원화)는 훔칠 값어치가 있다 |
| `jsonrpsee` | **wire 프로토콜**(JSON-RPC) | 같음 |
| `tonic` | **IDL + 코드생성** | build step 강제. 실패 모드가 나쁘다(codegen 실패가 `include!` 의 "No such file" 로만 드러남) |
| `irpc` | **전송(QUIC) + 직렬화(postcard)** | 문서가 자인 — *"이 라이브러리는 전송을 추상화하지 않습니다"*. ★**`quic-rpc` 와 묶어 읽지 말 것**★ — `irpc` 는 자기 문서에서 `quic-rpc` 와 **대비**해 자신을 설명한다 |
| `postcard-rpc` | **직렬화(postcard)** | no_std 타입 강제 + 3-crate 레이아웃 사실상 강제 |
| `bevy_replicon` | — | *"The library doesn't provide any I/O, so you need to add a messaging backend."* = 수명 포기 |

`확실`(각 원문 또는 매니페스트 대조 — 단 `postcard-rpc` 의 다운로드·라이선스 수치는 `모름`)

★**정정 — `quic-rpc` 는 이 표에 들지 않는다. 전송 축을 고정하지 않는다.**★ `Connector`/`Listener` trait 을 노출해 전송을 갈아끼우게 한다 — *"A Connector can be used to open bidirectional typed channels using Connector::open. A Listener can be used to accept bidirectional typed channels"* — 그리고 내장 전송이 넷이다(flume = 인메모리 · hyper = HTTP/2 · quinn = QUIC · iroh). ★**즉 「타입 있는 채널 + 갈아끼우는 전송 seam」의 성숙 선례가 실물로 있고, 이것이 아래 후보 D 의 근거다**★(§3 후보 D). `확실`(원문 대조)

### 2-3. 훔칠 만한 메커니즘 넷

1. ★**path + schema 해시를 라우팅 키로**★ — `postcard-rpc` 가 경로 문자열 + 타입 스키마를 해싱해 키를 만든다. 스키마가 어긋나면 **조용한 오해석 대신 라우팅 실패**가 난다. **우리와 토폴로지가 같다**(호스트 1 ↔ 디바이스 N). `가능성 높음`
   - ★**단 「값싼 버전 협상 수단」으로 읽지 말 것 — 버전 협상을 *대체하지 않는다*.**★ 해시가 **비암호학적 FNV1a** 이고 wire 키 폭이 1/2/4/8바이트로 선택 가능하며, 프로토콜은 **버전 필드를 따로** 예약해 둔다. 즉 ① 해시 불일치는 「다르다」만 알려 주지 협상을 *공급하지* 않고 ② **충돌하면 그 보증마저 깨진다.** 메커니즘 자체는 여전히 훔칠 값어치가 있다. `확실`(원문 대조)
2. ★**wire 메시지와 앱 명령을 타입으로 가르기**★ — `ezsockets` 의 `Call` 연관 타입: `on_text`/`on_binary`(와이어에서 옴)와 `on_call`(앱이 연결에 지시함)이 분리돼 있다. 우리 `agentClient` 구조에 그대로 대응된다. `확실`
3. **ALPN 문자열을 프로토콜 ID 겸 버전 협상으로** — `iroh`. 가장 값싼 핸드셰이크 형태. `가능성 높음`
4. **스키마를 build time 이 아니라 handshake time 에 배달** — Colyseus 가 입장 시 타입 정보 + 전체 상태를 함께 보낸다. `불확실`(라이선스·버전 미확인)

### 2-4. 재사용을 실제로 낳은 조건 — 예상과 달랐다

재사용된 라이브러리들의 공통점은 **「범용으로 설계했다」가 아니라 「강제하는 것이 적다」**였다.

| 속성 | 판정 | 근거 |
|---|---|---|
| async runtime 강제하지 않기 | `확실` | 런타임 결합이 생태계를 사일로로 가른다는 서술 + sans-IO 회고 |
| **I/O 를 안 갖기(sans-IO)** | `가능성 높음` | Firezone 프로덕션 회고. ★**실증**: Firezone 이 `str0m` 에서 **ICE agent 만 떼어다 썼다** — 원 프로젝트 밖 재사용이 실제로 일어난 사례★. ★**단 사례 하나는 성공한 조합 하나를 세울 뿐 시장 전체의 인과 규칙이 아니다**★ — 그래서 `확실`이 아니다 |
| 얇은 stable core crate 분리 | `확실` | `tracing-core`(*"the minimal, stable kernel of functionality"*) · `jsonrpsee-core` · `http` |
| 의존 표면 최소 | `가능성 높음` | 컴파일 시간 수치는 있으나 *채택* 근거는 아님 |
| `no_std` 지원 | `불확실` | 데이터 못 찾음 |

★**sans-IO 의 대가도 그 회고가 자인한다**★ — 순수 상태머신 작성 비용, 커뮤니티 채택 부족. 그리고 **function colouring** 문제를 피하려고 그렇게 한 것이다.

---

## §3 후보 아키텍처 — 사용자가 고를 것

네 후보 전부 **별도 crate 신설**(명제 4)이고 **기존 `net` 을 안 뜯는다**. 갈리는 축은 ★**「I/O 를 어디에 두나」**★ 와 ★**「피어 N 을 누가 소유하나」**★ 둘이다.

### 후보 A — 단일 crate · 안에서 2층(sans-IO 코어 + I/O 층) · 방향은 feature

**모양:** crate 하나 안에 순수 상태머신 층과 I/O 층을 모듈로 나눠 둔다. `client`/`server` feature 로 방향을 고른다(`default = []`).

| crate 안 | 소비자가 줄 것(seam) |
|---|---|
| 프레임 경계 처리 | **패킷 타입 정의**(serde derive) |
| 봉투 + 태그 상관(**태그 타입에 제네릭**) | codec 선택(제공 codec 중 택1 또는 자체 구현) |
| 핸드셰이크 **시퀀스**(모양은 소비자가 준 프레임) | **태그 타입** + 태그를 붙이고 읽는 방법 |
| 연결 수립(`connect`/`accept`) | 주소·인증 자료 |
| 재연결 + 백오프(정책은 값으로 주입) | 재연결 정책 값 |
| keepalive 개시·응답 | TLS 계층(원하면 바깥에서 감싼다) |
| 다중 연결 레지스트리 + 팬아웃 | 버전·능력 협상(ADR-0158 = 위층 소유) |
| 유계 큐 = 배압 **신호** | 배압 **정책**(막을지 버릴지) |

**다중 접속 표현:** 오래 사는 상위 객체가 연결 N 개를 id 로 소유하고 `get_or_connect` 형태로 준다. **핸들 = 배경 태스크로 가는 명령 채널**이라 재연결을 가로질러 살아남는다.

**장점:** 사용자가 원한 모양에 가장 가깝다. crate 하나라 ADR-0175 의 무게 조항에 안 걸린다. 유지보수 부담이 가장 낮다(대형 프로젝트가 별도 crate 에서 이 형태로 되돌아온 실물 선례가 있다).
**단점:** ★sans-IO 순수성이 한 crate 안에서는 컴파일러 강제 없이 관례로만 버틴다★(게이트를 따로 세워야 한다). 그리고 **feature unification 때문에 client 전용 소비자도 server 쪽 의존을 끌 수 있다**(§7).

### 후보 B — crate 둘 · 얇은 sans-IO 코어 + I/O 층

**모양:** `…-core`(sans-IO: 프레임 상태머신·핸드셰이크 시퀀스·상관·keepalive 판정, 워크스페이스 의존 0, 런타임 없음) + `…`(I/O: 전송·수명·재연결·레지스트리·팬아웃, feature 로 방향).

| crate 안(코어) | crate 안(I/O) | 소비자가 줄 것 |
|---|---|---|
| 프레임 상태머신 | `connect`/`accept` | 패킷 타입 정의 |
| 핸드셰이크 시퀀스(순수) | 재연결·백오프 | codec 선택 |
| 태그 상관(제네릭) | keepalive 타이머 | 태그 타입 |
| keepalive **판정**(시간을 인자로) | 다중 연결 레지스트리·팬아웃 | 주소·인증 자료·정책 값 |

**다중 접속 표현:** 후보 A 와 같되 레지스트리가 I/O crate 소유.

**장점:** ★재사용 조건(§2-4)을 실제로 만족하는 유일한 후보★ — 코어가 런타임·I/O 를 강제하지 않아 남의 프로젝트가 코어만 떼어 갈 수 있다. 시간을 인자로 넣어 테스트되므로 검증 하네스가 가장 강하다. 이 형태의 성숙 선례가 있다(`jsonrpsee` 하이브리드: 하부 별도 crate + 위에 umbrella 가 feature 로 당김).
**단점:** ★**ADR-0175 결정 6 에 정면으로 걸릴 수 있다**★ — *파일 하나짜리 lib 은 만들지 않는다 · lib 은 무게를 서로 비슷하게 맞춘다.* 코어가 얇게 나오면 그 조항이 기각 사유가 된다. **그 무게는 지금 모른다 — 설계 전에는 못 잰다.**

### 후보 C — 전송은 바이트 파이프 · 상관과 타이핑은 전부 위층

**모양:** crate 는 바이트/문자열만 나른다. 직렬화·상관·패킷 어휘를 하나도 모른다.

| crate 안 | 소비자가 줄 것 |
|---|---|
| 연결 수립·재연결·keepalive | **직렬화 전부** |
| 프레임 송수신(바이트) | **상관 전부**(pending map 포함) |
| 다중 연결 레지스트리·팬아웃 | 패킷 정의 + 그것을 바이트로 만드는 코드 |

**다중 접속 표현:** 레지스트리가 crate 소유. 핸들 = 명령 채널.

**장점:** engram 의 「어휘 격리는 타입 단위」 불변식(ADR-0129)과 **마찰이 0**이다. 성숙 선례가 정확히 이 모양이다 — 그 transport trait 은 메서드가 `send(String)`·`send_ping()`·`close()`·`receive()` 뿐이고 **id 를 한 글자도 언급하지 않는다**(원문 대조). 
**단점:** ★**사용자 요구에서 가장 멀다**★ — "패킷 정의만 주면 된다"가 아니라 "바이트만 나른다"가 된다. 셸의 `connection.rs` 에서 옮겨 갈 수 있는 양이 가장 적다.

### 후보 D — 타입 있는 다중 호스트 수명 엔진 · 전송은 `Connector`/`Listener` seam 으로 갈아끼움 · 핸들은 세대에 묶인다

**모양:** crate 가 패킷 타입 seam 과 연결 수명을 **둘 다** 소유하되, 실제 전송은 `Connector`/`Listener` trait 뒤에 두고 기본 어댑터(WS)를 함께 낸다. 호스트마다 감독 태스크가 하나씩 붙어 재연결·백오프·재구독을 지고, 인바운드는 **호스트 표식이 붙은 단일 병합 스트림**으로 올라온다. 핸들은 세대(generation)에 묶여서, 재연결 후 옛 핸들로 보내면 **조용히 성공하지 않고 「재연결됨」 오류로 거절**된다.

| crate 안 | 소비자가 줄 것(seam) |
|---|---|
| 패킷 seam | **패킷 타입 정의** |
| 태그 상관(제네릭) | 전송 어댑터(기본 제공분을 쓰면 0) |
| 연결 수립·수명 | 주소·인증 자료 |
| 재연결/백오프 · keepalive | 재연결 정책 값 |
| **호스트 N 레지스트리 + 호스트별 감독** | 세대 끊김 시 재시드(re-seed) 전략 |
| 호스트 표식 팬인 · **세대 경계 노출** · 팬아웃 | — |

**다중 접속 표현:** ★**1급이다.**★ 호스트 id 가 API 전면에 있고 이벤트가 호스트 표식을 달고 온다.

**장점:** 사용자 요구 넷(패킷 seam · 수명 소유 · 다중 접속 · 재사용)을 **유일하게 동시에 겨냥한다.** ★세대 경계를 노출하는 형태가 engram 의 화신 표식·replay 불변식과 같은 idiom 이라 §5-4 위험을 설계로 흡수한다★. 전송 seam 의 성숙 선례(`quic-rpc` 의 `Connector`/`Listener`)와 다중 호스트 감독의 선례(AHP `MultiHostClient`)가 각각 실물로 있다.
**단점:** ★**가장 크다**★ — 소유 범위가 넓어 무게가 크고, 팬아웃 계약(호스트별 공정성·부분 실패·느린 소비자 격리·재연결 유실)을 **명시적으로 설계해야** 하며 그것을 빠뜨리면 A·B 와 같은 결함을 더 큰 표면에서 반복한다. 그리고 「소비자는 패킷 정의만 준다」는 여전히 **완전히는 참이 아니다**(§9 C1).

### 후보 비교

| 축 | A(단일·2층) | B(코어+I/O) | C(바이트 파이프) | D(다중 호스트 엔진) |
|---|---|---|---|---|
| 「패킷만 주면 됨」 달성도 | 높음 | 높음 | 낮음 | **높음**(단 문자 그대로는 아님) |
| 프로젝트 중립 재사용 | 중간 | **높음** | 중간 | 중간(전송 seam 덕에 낮진 않다) |
| ADR-0175 무게 조항 | 통과 | **위험** | 통과 | ★**가장 위험**★ |
| ADR-0129 어휘 격리 마찰 | 낮음(태그 제네릭이면) | 낮음 | **없음** | 낮음(태그 제네릭 전제) |
| 독립 검증 하네스 강도 | 중간 | **높음** | 중간 | 중간(전송 seam 이 인메모리 어댑터를 허용) |
| 유지보수 부담 | **낮음** | 높음 | 낮음 | ★가장 높음★ |
| 다중 접속 1급 지원 | 부분 | 부분 | 부분 | ★**1급**★ |
| 옮겨 갈 수 있는 `connection.rs` 분량 | 많음 | 많음 | 적음 | **가장 많음** |

---

## §4 책임별 경계 — 선례가 어디에 두나

★**이 표는 후보와 독립이다**★ — 어느 후보를 고르든 각 항목이 안이냐 밖이냐는 같은 근거를 쓴다.

| 책임 | 통상 위치 | 근거(원문에서 읽은 것) | 확신도 |
|---|---|---|---|
| 바이트 프레이밍 | **안**(단 codec 은 주입) | 프레이밍은 라이브러리, **경계 규칙은 소비자 `Decoder`** | `확실` |
| 직렬화 | **갈림** | 고정형(gRPC·remoc) vs optional(tarpc — *"It's entirely optional"*) | `확실` |
| 연결 수립·핸드셰이크 | **갈림** | 안(tungstenite·zenoh) vs 밖(hyper — *"Connecting to a host, pooling connections, and the like are not handled at this level."*) | `확실` |
| **인증/토큰 교환** | **거의 밖** | RFC 6455 §10.5 — 프로토콜이 서버의 클라이언트 인증 방식을 **하나도 규정하지 않고**, 쿠키·HTTP 인증·TLS 인증처럼 **범용 HTTP 서버가 쓸 수 있는 아무 수단이나 허용한다**. ★옛 판(*"does not define client authentication mechanisms beyond what is provided by HTTP"*)은 **지어낸 의역이었다 — 인용 금지**★ | `가능성 높음` |
| **버전 협상** | **갈림 — 단 engram 은 결정됨** | ADR-0158 이 **위층 소유**로 못 박았다. 전송은 그 프레임을 불투명하게 나른다 | `확실`(내부 결정) |
| keepalive | **응답=안, 개시=갈림** | RFC 6455: ping 은 *MAY*, pong 응답은 *MUST* — ★**단 무조건이 아니다**★: §5.5.2 가 **이미 Close 프레임을 받은 엔드포인트를 면제한다.** gRPC A8 은 앱 레벨 keepalive 를 라이브러리 안에 둔다 | `확실` |
| **재연결 + 백오프** | **갈림 — RPC 프레임워크는 안** | gRPC 문서가 ★*"Proposed Backoff Algorithm"*★ 으로 제목을 달고 **대안을 허용한다 — 강제 규격이 아니다.** 상수(INITIAL 1s·MULT 1.6·MAX 120s·JITTER 0.2)는 **예시 기본값**으로만 쓴다. tower 는 **별도 미들웨어로 분리** | `확실` |
| **요청/응답 상관** | ★**전송 바로 위의 별도 계층**★ | §4-1 | `확실` |
| 순서/중복 제거 | **갈림** | remoc 는 아예 요구사항으로 밀어냄(*"ordered, reliable transport"* 필요) | `가능성 높음` |
| 배압 | **신호=안, 정책=밖** | Netty `isWritable()` 은 신호만, 쓰기 중단은 앱 책임 | `가능성 높음` |
| 팬아웃 | **갈림** | Netty `ChannelGroup` 은 안 / hyper·tungstenite·tokio-tower 는 **연결 1개만** 다룸 | `가능성 높음` |
| **TLS** | **거의 밖**(QUIC 만 예외) | hyper 비전 문서: ★*"We learned early that bundling TLS directly in hyper has problems"*★ — **한번 넣었다 뺀 이력** | `확실` |
| 주소 해석·discovery | **갈림** | hyper 는 1.0 에서 `client::connect` 모듈 자체를 제거해 `hyper-util` 로 내보냈다 | `확실` |
| graceful shutdown | **프리미티브=안, 오케스트레이션=밖** | GOAWAY·Close 는 프로토콜, `GracefulShutdown` 유틸은 hyper 가 아니라 `hyper-util` | `가능성 높음` |

### 4-1. 요청/응답 상관 — 우리 갈림길의 답

우리 문제(상관은 요청 id 가 필요한데 그 id 타입이 `protocol` crate 에 산다)를 **그대로 겪고 다르게 푼 진영이 다섯**이다.

**(A) 구체 id 를 프레임워크에 박제 → 폐기된 길.** `tokio-proto` 가 `pub type RequestId = u64;` 를 두고 transport 가 `(RequestId, Frame)` 을 내놓게 했다. 폐기됐고, 이슈에 남은 사유는 **복잡도**(*"It tries to provide many capabilities at the cost of significant complexity."*)·**오류 가시성**·**HTTP 버전 한계**다.

★**정정 — 「id 타입을 고정해서 안 쓰였다」는 교훈은 근거가 없다.**★ 인용된 논의 어느 것도 **id 타이핑을 폐기 원인으로 지목하지 않는다.** 남는 것은 인과가 아니라 **API 모양의 이동 관측**뿐이다 — 구체 `u64` → (다음 세대에서) 연관 타입. 이 이동을 원인으로 읽는 것은 **해석**이고, 그렇게 적은 출처는 없다. `불확실`

- **오류 인용도 정정한다** — *"Errors that are encountered reading, parsing, and writing are kind of just lost"* 는 그 이슈에서 **곧바로 이어서** 「그 오류들이 tokio-proto 까지 *도달은 하지만* 사용자가 얻어내기 어렵다」고 적는다. 「사라진다」로 잘라 인용하면 원문보다 세다. `불확실`

**(B) 태그 타입을 연관 타입으로 제네릭화 — `tokio-tower`.** 원문 대조 결과:
```rust
pub trait TagStore<Request, Response> {
    type Tag: Eq;
    fn assign_tag(self: Pin<&mut Self>, r: &mut Request) -> Self::Tag;
    fn finish_tag(self: Pin<&mut Self>, r: &Response) -> Self::Tag;
}
```
★**바운드가 `Eq` 하나뿐이다**★. 그리고 crate 문서: *"tokio-tower leaves the on-the-wire implementations of protocols to other crates … and instead operates at the level of Sinks and Streams."* pending map 은 **client 계층**이 소유하지 transport 가 아니다. `확실`(원문 대조)

**(C) transport 가 id 를 아예 모르게 — `jsonrpsee`.** transport trait 메서드가 `send(msg: String)` · `send_ping()`(기본 구현) · `close()` 뿐이고 **id 를 한 글자도 언급하지 않는다.** `Id` 타입은 별도 types crate 에 살고 상관은 core client 전담. ★**우리 배치와 가장 정확히 일치하는 선례다.**★ 부수 관측: *언제* ping 할지는 core 가 정하고 *어떻게* ping 하는지만 transport 가 안다. `확실`(원문 대조)

**(D) 프레임워크가 id 를 정의하되 transport 는 메시지 타입에 제네릭 — `tarpc`.** transport 는 `Request` 를 통째로 나르지 id 를 해석하지 않는다. 취소도 id 위에 얹혀 있다. `확실`

**(E) 세션 계층 프로토콜로 분리 — Finagle mux.** *"Mux is a pure session layer protocol"* — 태그 T/R 쌍·취소·liveness ping·윈도우 광고를 mux 가 지고 앱 프로토콜은 그 위에. `확실`

**(F) id 를 안 쓰는 우회 — NATS(요청마다 고유 inbox subject) / remoc(채널 수명이 상관을 표현).** `가능성 높음`

★**종합 판정: id 는 wire 어휘이지 transport 어휘가 아니다.** 다섯 진영 전부 id 를 프로토콜/세션 계층에 두고 transport 는 프레임만 나른다. **transport 가 id 를 배우는 배치는 조사 범위에서 못 찾았다.** pending map 이 client/session 계층 소유인 것도 일관된다.★

**그래서 우리 갈림길의 답:** ★**상관 「기제」는 새 crate 안에 두되, 태그 타입에 제네릭하게 짜고 `RequestId` 는 `protocol` 에 남긴다**★(후보 B 형). 그러면 새 crate 는 `protocol` 을 의존하지 않고도 상관을 소유한다. **이것이 ADR-0129 의 「어휘 격리는 타입 단위」와 충돌하지 않는 두 형태 중 하나다** — ★**「유일한」은 이 문서 자신의 §4-1 (C) 가 반증한다**★: `jsonrpsee` 처럼 **transport trait 이 id 를 아예 모르는** 형태가 두 번째이고, 그것이 후보 C 다.

**단 이 판정에는 해석이 섞여 있다** — 「제네릭하게 짜야 한다」고 명시한 maintainer 문장은 **못 찾았다.** `tokio-proto`(구체 `u64`) → `tokio-tower`(연관 타입) 의 이동은 두 소스를 대조해 얻은 관측이고, "그래서 제네릭으로 바꿨다"고 적힌 글은 없다. **사실과 해석을 갈라 읽을 것.** `가능성 높음`

---

## §5 다중 접속 소유 모델

### 5-1. 「N 개 서버」에는 두 종류가 있고 선례가 거기서 갈린다

★**이것이 이 조사의 두 번째 소득이다.**★

- **N 개가 서로 교체 가능한 한 논리 서비스**(DB 클러스터·메시지 브로커·gRPC 백엔드 풀)면 → **라이브러리가 집합을 소유하고 소비자는 값싼 clone 핸들 하나만 받는다.** 소비자는 개별 연결을 아예 못 본다. NATS·tonic·grpc-go·etcd·Cassandra/Scylla·MongoDB·reqwest 가 전부 이 모양. `확실`
- **N 개가 각자 다른 정체성·상태를 가진 별개 피어**(★**우리 경우**★ — 데몬마다 자기 에이전트 명부를 갖는다)면 → **Endpoint/Connection 2층 분리**로 갈리고, **그 진영에서는 재연결·페일오버가 소비자 몫으로 남는다.** quinn·iroh 가 그 정본. `확실`

★**정정 — 「정체성이 다른 피어면 재연결·페일오버가 *필연적으로* 소비자 몫」은 거짓이다.**★ 반례가 실물로 있다 — Microsoft Agent Host Protocol 의 Rust `MultiHostClient` 는 전송 N 개를 소유하고 **호스트마다 감독을 붙인다**(각 호스트가 자기 `HostRuntime` 태스크에서 돈다) · 백오프 · 취소 · **재연결을 가로지르는 재구독**(*"re-subscribes to known URIs across reconnects"*) · 그리고 이벤트가 `host_id` 를 달고 올라오는 **호스트 표식 팬인**까지 라이브러리가 진다. 즉 **2층 분리 진영이 그것을 소비자에게 넘기는 것은 그 라이브러리들의 성질이지 이 토폴로지의 필연이 아니다.** `확실`(원문 대조)

★**우리는 후자 토폴로지다. 그리고 그 토폴로지에서 수명을 라이브러리가 소유한 선례가 있다** — 그것이 후보 D 의 실물 근거다.★

### 5-2. 2층 모델의 알려진 부족 — 나중에 풀이 얹혔다

`iroh`(2층의 정본)가 나중에 `ConnectionPool` 을 추가했다. **명시된 사유(원문 대조):** *"opening a new connection every time you do a small exchange with a peer is very wasteful"*, *"Iroh connections are relatively lightweight, but even so you don't want to keep thousands of them open at the same time"*, 그리고 목적은 *"whenever you have a protocol that has to talk to a large number of endpoints while keeping an upper bound of concurrent open connections"*. API = `get_or_connect`(*"will try to get an existing connection from the pool. If there is none, it will create one and store it"*), 설정 = `idle_timeout`·`max_connections`·`connect_timeout`, 그리고 *"a drop-in replacement for endpoint.connect"*. `확실`(원문 대조)

★**정정 — 이 세션이 앞서 이것을 「소비자가 N 개 핸들을 들고 있으라는 게 부족하다는 자백」이라고 사용자에게 보고했는데, 그건 과장이다.**★ 원문이 든 사유는 **낭비와 연결 수 상한**이지 소유 모델의 실패가 아니다. 살아남는 관측은 더 좁다 — **id 로 조회해 없으면 만들어 주는 레지스트리가 2층 위에 결국 필요해졌다.**

### 5-3. API 모양을 실제로 가르는 축 — 「핸들이 무엇인가」

★**모델 1/2/3 보다 이 축이 결과를 더 크게 가른다.**★

- **핸들 = 소켓** → 재연결하면 **새 핸들**이고 소비자가 맵을 갱신해야 한다. (quinn·tokio-postgres·tokio-tungstenite)
- **핸들 = 배경 태스크로 가는 명령 채널** → **재연결을 가로질러 핸들이 그대로 살아남는다.** (NATS·redis `ConnectionManager`·gRPC `ClientConn`·tonic `Channel`)

★**정정 — 「그러니 핸들은 명령 채널이어야 한다」로 일반화하지 말 것. 반례가 있고, 그쪽은 의도적으로 반대로 간다.**★ AHP 의 `MultiHostClient` 는 **세대 무효화**를 고른다 — *"Any `HostClientHandle` you obtained from a previous connection refuses to dispatch on the new one and returns `HostError::HostReconnected`"*. 즉 재연결에 불사인 핸들 대신 **경계를 표면으로 올리는** 형태다. `확실`(원문 대조)

**그래서 이 축은 「옳은 한쪽」이 아니라 방어 가능한 양 끝이다:**
- **불사 명령 핸들** — 경계를 숨긴다. 소비자 코드가 짧아지는 대신 유실이 안 보인다(§5-4).
- **세대에 묶인 핸들** — 경계를 드러낸다. 소비자가 재시드 결정을 져야 하는 대신 조용한 유실이 안 생긴다.

★**후자가 engram 이 이미 하고 있는 것이다 — 화신 표식(epoch), ADR-0163/0164.**★ 그래서 이 저장소에서는 세대에 묶인 쪽이 idiom 에 맞는 끝이다.

### 5-4. ★우리 replay 불변식과 정면 충돌하는 위험★

우리와 형상이 가장 가까운 재연결 래퍼(desktop → WS 데몬)가 자기 문서에 적어 둔 것 — 원문 대조:
> *"By default, the library is re-transmitting pending calls and re-establishing subscriptions that were closed until it's successful"*
> ★*"subscriptions, which may lose a few notifications when it's re-connecting, **it's not possible to know which ones**."*★

`확실`(원문 대조)

★**독립 2차 확증 — 다른 계열에서도 같은 위험이 프로덕션 문서에 적혀 있다.**★ AHP 의 다중 호스트 클라이언트는 두 이벤트 원천이 **모두 `tokio::sync::broadcast` 기반**이라 버퍼가 차면 *"drop envelopes on slow consumers"* 하고, 그 결과가 *"A dropped (or missed-because-reconnected) envelope permanently desyncs the mirror for that `(host, channel)` until it's re-seeded from a fresh snapshot"* 다. ★즉 조용한 재연결 유실은 실제 프로덕션 위험이고, 그 문서가 **처방까지 이름 붙인다 — 새 스냅샷으로 재시드**★. `확실`(원문 대조)

★**이것이 이 설계의 최대 위험이다.**★ 재연결을 라이브러리 안에 숨기면 **구독 갭이 소비자에게 안 보이는 형태로 생긴다.** 그런데 engram 은 그 갭을 다루는 불변식을 이미 갖고 있다 — replay→live 경계, seq dedup, 화신 표식. ★**새 crate 가 재연결을 소유하려면 「무엇이 유실됐는지 소비자가 알 수 있는」 형태여야 한다**★(예: 재연결 사건을 소비자에게 사건으로 올리고 재구독 결정을 소비자가 하게, 또는 세대 경계 마커를 스트림에 끼워). **이 요구를 명시하지 않으면 후보 A·B 둘 다 이 결함을 그대로 물려받는다.**

부수 근거: etcd 클라이언트가 **불건강 엔드포인트를 직접 추적한 것이 실패 원인**이었고(고정 블랙리스트 탓에 복구된 노드가 계속 사용 불가), 결국 *"상태를 추적하지 말고 그냥 다음으로 넘겨라"* 로 단순화했다. `가능성 높음`

### 5-5. 모델 4(단일 이벤트루프 폴링)를 권하지 않는 근거

libp2p `Swarm` 은 연결 수락/거절 결정권을 독점해 사용자가 조건별 인바운드 제어를 못 했고 이슈가 났다. rumqttc 는 `poll()` 을 멈추면 전부 정지한다(*"NOTE Don't block this while iterating"*). `가능성 높음`

---

## §6 engram 제약 적합도

| 제약(정본) | A | B | C | D | 비고 |
|---|---|---|---|---|---|
| **ADR-0129 어휘 격리(타입 단위)** | ○(태그 제네릭 조건부) | ○ | ◎ | ○(태그 제네릭 조건부) | 태그를 제네릭으로 안 짜면 A·D 는 위반 |
| **ADR-0158 버전 협상 = 위층** | ○ | ○ | ○ | ○ | 넷 다 전송이 버전을 몰라야 한다 |
| **ADR-0036 단일 연결 불변식** | ★사용자 결정 필요★ | ★동★ | ★동★ | ★동 — 다만 D 는 N 을 API 전면에 올려 이 결정을 **정면으로** 건드린다★ | 후보와 무관하게 걸린다(§10-1) |
| **ADR-0175 무게 조항** | ○ | ★위험★ | ○ | ★★가장 위험★★ | 소유 범위가 가장 넓다 |
| **ADR-0151 「독립적으로 쓸 수 있는가」** | ○ | ◎ | ○ | ○(전송 seam 이 독립 실행을 살린다) | B 가 가장 강하게 통과 |
| **replay/seq 불변식** | ★조건부★ | ★조건부★ | ○ | ◎ | D 는 세대 경계 노출이 설계에 박혀 있다(§5-3) |
| **`default = []` 관례** | ○ | ○ | ○ | ○ | 사유 = *조용한 회귀보다 시끄러운 실패*(net Cargo.toml) |
| **CI 게이트 이름 접두** | ★함정★ | ★함정★ | ★함정★ | ★함정★ | §10-3 |

**추가로 즉시 살아나는 미결 하나** — `frame_port` 의 feature 소속이 ADR-0130 과 함께 보류돼 있는데, 그 주석이 *"재개 시 착수 전에 결정할 것 — 공개 API 변경이라 나중에 하면 비싸다"* 로 적혀 있다. ★**새 crate 가 프레임 포트 계약을 참조하려 하면 이 미결이 즉시 살아난다.**★

---

## §7 방향을 feature 로 가르기 — 된다, 단 조건 하나

### 7-1. 선례 — 우리가 하려는 것과 거의 같은 형태가 있다

`hyper` 1.x 매니페스트 원문(대조 완료):
```toml
[features]
# Nothing by default
default = []
full = ["client", "http1", "http2", "server"]
client = ["dep:want", "dep:pin-project-lite", "dep:smallvec"]
server = ["dep:httpdate", "dep:pin-project-lite", "dep:smallvec"]
```
★`full` 이 둘을 동시에 켠다 = **둘은 배타가 아니라 가산이다.** 이 점이 이 설계가 성립하는 유일한 이유다.★ `확실`

그리고 **반대 방향 실증** — `tokio` 는 별도 crate 를 버리고 단일 crate + feature 로 **이주**했다(2019). 사유: *"Maintaining a large number of crates comes with an increased maintainership burden… Users feel that large number of dependencies == bloat."* `가능성 높음`

`jsonrpsee` 는 하이브리드다 — 하부는 별도 crate, 위에 umbrella 가 feature 로 그것들을 optional dependency 로 당긴다. ★**후보 B 를 고른다면 이 형태가 참조 대상이다.**★ `확실`

### 7-2. ★넘을 수 없는 벽 — 「client 는 켜되 server 는 끈다」는 Cargo 가 표현 못 한다★

Cargo book 원문(대조 완료):
> *"When a dependency is used by multiple packages, Cargo will use the union of all features enabled on that dependency when building it."*
> *"Features should be additive. That is, enabling a feature should not disable functionality, and it should usually be safe to enable any combination of features."*
> *"There are rare cases where features may be mutually incompatible with one another."* → 제공되는 수단은 `compile_error!` 하나뿐 = **union 을 막는 게 아니라 union 이 일어난 뒤 빌드를 죽이는 것.**

상호배타 feature 요청은 2016년부터 열린 채다. `확실`

**단 「Cargo 가 표현 못 한다」는 stable 기준이다** — nightly 에는 `-Z feature-unification` 이 있어 **패키지별로 의존 feature 를 따로 해석**(빌드를 중복시켜)할 수 있다. stable Cargo 에 대한 위 결론은 그대로다. `확실`

★**그리고 union 은 우리가 통제할 수 없다**★ — 의존 그래프 안 아무 crate 하나가 `server` 를 켜면 전원이 켜진다. `resolver = "2"` 가 고치는 것은 target-specific·build-dep·dev-dep 세 축뿐이고 **평범한 normal dependency 그래프 안의 union 은 안 없어진다**(RFC 2957 이 스스로 자인: *"The new feature resolver does not address all of the enhancement requests"*). `확실`

### 7-3. 실제 사고 둘

1. **의존 누수 → 타깃 빌드 실패** — `default-features = false` 를 줬는데도 런타임이 딸려 와 wasm 타깃이 죽은 사례(tonic#1783). `가능성 높음`(해소 방식은 `모름`)
2. ★**거짓 컴파일 성공 — 우리 설계의 진짜 함정**★ — *"in practice `default-features = false` is almost impossible to use, because any dependency anywhere in the dependency tree that just includes `tokio = "1"` will silently bring all the default features back."* 소비자가 자기 feature 선언을 빠뜨려도 남이 켜 줘서 **일단 컴파일된다** → 남의 의존이 바뀌는 날 터진다. `가능성 높음`

**net 의 `default = []` 주석이 이미 같은 한계를 적어 뒀다** — *feature 합집합은 cargo 호출 1회 단위라, 루트 `cargo build` 는 데몬이 `server` 를 켜므로 net 을 여전히 한 번 server 로 빌드한다.* 즉 **이 저장소에서 빈 default 의 이득은 「소비자 쪽 실수를 시끄럽게 만드는 것」이지 「빌드 그래프에서 server 를 빼는 것」이 아니다.**

### 7-4. 테스트 부담

`cargo hack --each-feature` / `--feature-powerset --depth N`. 실무 권고 = feature 2개 이상 crate 는 `--each-feature` 를 CI 에, powerset 은 코어 crate·feature 8개 미만에만(지수). `가능성 높음`

★**「성숙 crate 들이 실제로 cargo-hack 을 CI 에서 돌린다」는 확인 못 했다 — 인용 금지.**★

---

## §8 grounding 결과 (메인이 1차 출처 직접 대조)

| 클레임 | 판정 | 확인 방법 |
|---|---|---|
| `hyper` 가 `default = []` + `client`/`server` + `full` | **지지** | 매니페스트 원문 fetch — 인용과 바이트 단위 일치 |
| Cargo feature union·additive·상호배타 미지원 | **지지** | Cargo book 원문 fetch, 세 인용 전부 확인 |
| `tokio-tower` `TagStore` 의 `type Tag: Eq` | **지지** | docs.rs 원문 fetch — 바운드가 `Eq` 하나임 확인 |
| tokio-tower 가 wire 구현을 남에게 맡긴다 | **지지** | crate 문서 원문 인용 확인 |
| `jsonrpsee` transport trait 이 id 를 모른다 | **지지** | docs.rs 원문 fetch — 메서드 셋 전부 id 미언급 확인 |
| 재연결 중 알림 유실 + 어느 것인지 알 수 없음 | **지지** | crate 문서 원문 fetch — 문장 그대로 확인 |
| `remoc` 가 전송을 구현하지 않는다 | **지지** | docs.rs 원문 fetch |
| iroh `ConnectionPool` 도입 사유 | **부분지지 → 이 세션의 과장을 정정** | 원문의 사유는 **낭비·연결 수 상한**이지 「소유 모델 실패의 자백」이 아니다(§5-2) |

**수집자가 스스로 「출처 없음」으로 플래그해 이 문서가 배제한 것:** rule of three 의 숫자 · `default = []` 의 crates.io 분포 · client/server 분할을 union 때문에 포기한 사례(**못 찾음 — 유사 사례를 이 사례로 바꿔 말하지 말 것**) · Leptos 를 maintainer 인정 사례로 쓰는 것(**날조가 됨**) · 성숙 crate 의 cargo-hack CI 사용 · Rails 추출 서사.

---

## §9 적대 리뷰 결과

**판정 = ★BLOCK★**(수정 후 재판정 대상) · 리뷰어 = cross-family, effort high, 다중 렌즈 + 후보 랭킹 · **적출 39건.** 아래 반영분은 전부 본문에 적용했고, ★**수정본에 대한 재판정은 안 돌렸다**★.

### 9-1. 결론을 바꾼 적출 (메인이 원문 대조로 확인함)

- ★**두 진영 명제 반증**★(`bevy_simplenet`·`tonic`) — §0 을 그래서 고쳤다. 타입 seam + 수명을 겹친 실물이 있다.
- ★**다중 호스트 수명을 라이브러리가 소유한 실물이 있다**★(AHP `MultiHostClient`) — §5-1 을 그래서 고쳤다. 「정체성이 다른 피어면 소비자 몫」은 필연이 아니다.
- ★**세대 무효화라는 반대편 설계가 있다**★(AHP `HostError::HostReconnected`) — §5-3 을 그래서 고쳤다. 「핸들은 명령 채널이어야 한다」는 일반화가 아니다.
- **전송 seam 의 실물 선례**(`quic-rpc` 의 `Connector`/`Listener`) — 후보 D 의 근거. §2-2 에서 `irpc` 와 분리했다.
- ★**후보 공간이 불완전했다**★ — **후보 D 누락.** §3 에 추가했다.
- ★**「소비자는 패킷 정의만 준다」가 A·B 에서도 문자 그대로 참이 아니다**★ — 이 보고서 **자신의 분할표**가 codec 선택 · 태그 타입 · 태그 접근자 · 주소 · 인증 자료 · 재연결 정책 · 배압 정책을 소비자 몫으로 적고 있다. **사용자 목표를 문면 그대로 달성하는 후보는 없다 — 정도의 차이다.** → §10 의 새 결정 항목으로 올렸다.

### 9-2. 인용 정확성 적출 (전부 반영함)

- `remoc` 에 재연결·keepalive 가 없다 → **거짓.** 자동 ping(`connection_timeout`) + 문서화된 reconnecting transport(§2-1).
- RFC 6455 §10.5 인용문 → **지어낸 의역.** 실제 조문으로 교체(§4).
- gRPC 백오프 상수 → **제안 알고리즘**이지 강제 규격이 아님(§4).
- RFC 6455 Pong *MUST* → **무조건이 아님.** Close 프레임 수신 후 면제(§4).
- tokio-proto 「id 타이핑이 폐기 원인」 → **출처가 그렇게 말하지 않는다.** 관측으로 격하 + `불확실`(§4-1 A).
- tokio-proto 「오류가 사라진다」 → 원문은 **도달하되 얻기 어렵다**로 이어진다(§4-1 A).
- `postcard-rpc` 해시 키 = 「값싼 버전 협상」 → **대체하지 않는다.** 비암호학 FNV1a · 가변 키 폭 · 별도 버전 필드(§2-3).

### 9-3. 각 후보의 미다룬 실패 모드 (리뷰 적출 · 이 보고서가 답을 안 갖고 있음)

- **후보 A** — 재연결과 **경합하는 送信**에 대한 세대 펜스·전달 계약이 없다. `미해결`
- **후보 B** — 순수 판정과 타이머·I/O 를 가르면 **원자적 teardown/취소의 소유**가 미명세로 남는다. `미해결`
- **후보 C** — 불투명 바이트 파이프는 **타입 있는 앱 핸드셰이크·준비 완료 경계를 소유할 수 없는데**, 그러면서 「패킷 정의만 주면 된다」를 주장할 수 없다. `미해결`
- **A·B·C·D 공통** — 팬아웃 계약(공정성 · 부분 실패 · 느린 소비자 · 재연결 유실) 미명세. `미해결`

### 9-4. 리뷰 랭킹과 메인의 처리

리뷰어 랭킹 = **D > A > B > C.**

★**메인은 이 랭킹을 채택하지 않는다.**★ 승자 선정은 사용자 몫이고, 리뷰어는 engram 내부 제약을 보지 않았다 — **ADR-0036 단일 연결 불변식 · ADR-0175 무게 조항 · `frame_port` 미결.** 랭킹은 **외부 설계 관점의 입력으로만** 기록한다.

### 9-5. 채택하지 않은 적출

- 리뷰어가 누락 피어로 `ws-bridge`·Lightyear·Naia·`bevy_connect` 를 들었으나 **메인이 원문 대조를 하지 않았다** → `미검`으로 기록하고 **결론 근거로 쓰지 않는다.**
- **tokio-tower 의 최신 안정 릴리스가 2021년**이라는 지적은 사실 관계를 확인하지 않았다 → `미검`. 단 §4-1 의 인용은 **API 문서 원문 대조를 거쳤으므로 인용 자체는 유효하다.**

---

## §10 사용자가 정해야 할 것

> ★굵은 설계라 세션이 임의 확정하지 않는다.★ 1 번이 나머지를 가른다.

1. ★**ADR-0036 과의 관계**★ — 셸이 데몬 N 대에 각각 1 연결을 쥐는 것이 그 결정의 **확장인가 개정인가.** 문서가 답하지 않는다. 이걸 정해야 새 crate 의 다중 접속 API 를 그릴 수 있다.
2. **후보 A/B/C/D 중 무엇인가** — I/O 를 어디에 두나 + 피어 N 을 누가 소유하나(§3).
3. ★**「패킷 정의만 주면 된다」를 어디까지 타협하나**★ — **어느 후보에서도 문자 그대로는 성립하지 않는다**(§9-1 마지막 항). codec 선택·태그 타입·주소·인증 자료·재연결 정책·배압 정책 중 **무엇까지 소비자가 주는 것을 받아들일지**가 후보 선택보다 먼저 정해져야 후보 비교가 뜻을 갖는다.
4. **재연결을 crate 가 소유할 때 유실 가시성을 어떻게 보장하나**(§5-4) — 재연결 사건을 소비자에게 올릴지, 세대 마커를 끼울지, 새 스냅샷으로 재시드할지.
5. **핸들이 소켓인가 명령 채널인가, 아니면 세대에 묶이나**(§5-3) — 재연결을 가로질러 핸들이 살아남는지가 여기서 갈린다.
6. **crate 이름** — ★프로젝트 중립 이름을 붙이면 CI 의존 상한 게이트들이 그 crate 를 조용히 안 본다★(멤버를 `engram-dashboard` 이름 접두로 식별). 같은 커밋에서 게이트 정규식을 고쳐야 한다.
7. **`frame_port` 의 feature 소속**(§6 말미) — 새 crate 가 그 계약을 참조하면 즉시 살아나는 미결.

---

## §11 거부 후보 → ADR 거부 대안 재료

- **기존 `net` 을 직접 확장한다** — 사용자 명제 4 가 배제(별도 crate 신설 → 독립 검증이 먼저). 또한 `net` 의 존재 이유가 공유가 아니라 **순환 끊기**였고 격리 게이트가 그 경계에 맞춰 서 있다.
- **IDL + 코드생성**(tonic 형) — build step 강제, 실패 모드가 나쁨(codegen 실패가 `include!` 오류로만 드러남). 이 저장소는 이미 `ts-rs` 생성물 sync 게이트를 지고 있어 축이 하나 더 늘어난다.
- **전송을 QUIC 로 고정**(★`irpc` 형 — `quic-rpc` 가 아니다★) — `irpc` 문서가 스스로 *"전송을 추상화하지 않습니다"* 라고 적고 `quic-rpc` 와 대비해 자기를 설명한다. 명제 1(방향은 고르는 능력)과 맞지 않는다.
  - ★**`quic-rpc` 는 거부 후보가 아니다 — 후보 D 의 선례다.**★ 전송을 `Connector`/`Listener` trait 뒤로 빼고 내장 어댑터를 넷(flume·hyper·quinn·iroh) 낸다(§2-2).
- **단일 이벤트루프 폴링 모델**(libp2p Swarm / rumqttc 형) — 수락/거절 결정권 독점 이슈, poll 정지 시 전면 정지(§5-5).
- **상관을 구체 `RequestId` 로 전송에 박제**(tokio-proto 형) — 폐기된 길이다. ★**단 「id 타입 고정이 폐기 원인」으로 적지 말 것**★ — 출처가 든 사유는 복잡도·오류 가시성·HTTP 버전 한계이고, 남는 것은 구체 `u64` → 연관 타입이라는 **API 모양의 이동 관측**뿐이다(§4-1 A). `불확실`

---

## §12 한계

- ★**「타입 seam + 수명을 합친 물건이 없다」는 반증됐다**★(§0). 남는 부재 확인은 더 좁은 것 하나다 — **거기에 N-피어 레지스트리와 피어별 정체성까지 갖춘 것을 못 찾았다.** 검색 축을 여럿 돌렸지만 전수는 아니다.
- **후보 B·D 의 무게는 지금 못 잰다** — ADR-0175 무게 조항 통과 여부는 설계를 그려 봐야 안다.
- **`connection.rs` 1,674줄이 실제로 얼마나 엉켜 있는지 안 셌다** — 파일 헤더·import 판독뿐이다. 후보를 고른 뒤 필요하다.
- **두 선행 분할 기준(`crate-granularity-survey` + `module-split-criteria`)을 하나로 합치는 작업은 여전히 안 했다** — 직전 세션이 남긴 숙제가 그대로다.
- ★**리뷰 적출 중 `미검`으로 남긴 둘**★(§9-5) — 누락 피어 후보 넷(`ws-bridge`·Lightyear·Naia·`bevy_connect`)과 tokio-tower 유지보수 상태. **원문 대조를 안 했으므로 결론 근거로 쓰지 않는다.**
- **재판정 미실행** — §9 적출은 반영했으나 수정본에 대한 2차 적대 리뷰는 안 돌았다.

---

## §13 출처

★**재현용 목록이다 — 본문이 근거로 쓴 URL 을 절 단위로 모았다.**★ 여기 없는 주장은 원문 대조를 거치지 않았다고 읽는다.

**§0 · §2-1 (타입 seam + 수명을 겹친 실물 · remoc 정정)**
- `bevy_simplenet` — https://docs.rs/bevy_simplenet/latest/bevy_simplenet/
- `tonic` 재연결 — https://docs.rs/tonic/latest/src/tonic/transport/channel/service/reconnect.rs.html
- `remoc` `chmux::Cfg::connection_timeout` — https://docs.rs/remoc/0.20.0/remoc/chmux/struct.Cfg.html
- `remoc` 0.20 adaptable/reconnecting transport — https://docs.rs/remoc/0.20.0/remoc/ · https://docs.rs/remoc/latest/remoc/

**§2-2 · §11 (전송 seam 선례)**
- `quic-rpc` `Connector`/`Listener` + 내장 전송 넷 — https://docs.rs/quic-rpc/latest/quic_rpc/transport/

**§2-3 (해시 라우팅 키)**
- `postcard-rpc` `Key`(FNV1a · 1/2/4/8바이트 · 별도 버전 필드) — https://docs.rs/postcard-rpc/latest/postcard_rpc/struct.Key.html

**§2-4 (sans-IO 재사용 사례)**
- Firezone sans-IO 회고 — https://www.firezone.dev/blog/sans-io

**§4 (책임별 경계)**
- RFC 6455 §10.5 인증 — https://www.rfc-editor.org/rfc/rfc6455.html#section-10.5
- gRPC 제안 백오프 알고리즘 — https://grpc.github.io/grpc/core/md_doc_connection-backoff.html

**§4-1 (상관 계층)**
- `tokio-tower` `TagStore` — https://docs.rs/tokio-tower/latest/tokio_tower/multiplex/client/trait.TagStore.html
- `tokio-tower` crate 문서 — https://docs.rs/tokio-tower/latest/tokio_tower/
- `jsonrpsee` transport trait — https://docs.rs/jsonrpsee-core/latest/jsonrpsee_core/client/trait.TransportSenderT.html
- tokio-proto 폐기 논의 — https://github.com/tokio-rs/tokio/issues/1318

**§5 (다중 접속 · 재연결 유실 · 세대 무효화)**
- AHP `MultiHostClient`(호스트별 감독 · 재구독 · `HostError::HostReconnected` · broadcast 유실과 재시드) — https://github.com/microsoft/agent-host-protocol/blob/main/clients/rust/MULTI_HOST.md
- 재연결 래퍼의 알림 유실 자인 — https://docs.rs/reconnecting-jsonrpsee-ws-client/latest/reconnecting_jsonrpsee_ws_client/
- iroh `ConnectionPool` 도입 — https://www.iroh.computer/blog/iroh-blobs-0-95-new-features
  - ★**주의: 이 `ConnectionPool` 은 `iroh-blobs` 에서 나왔고 지금은 `iroh-util` 에 산다 — 코어 `iroh` 가 아니다.**★

**§7 (feature 축)**
- `hyper` 1.x 매니페스트 — https://github.com/hyperium/hyper/blob/master/Cargo.toml
- Cargo feature 문서(union · additive) — https://doc.rust-lang.org/cargo/reference/features.html
- nightly `-Z feature-unification` — https://doc.rust-lang.org/cargo/reference/unstable.html#feature-unification
