//! 선언 매크로 — 이름 · 인자 모양 · 반환 · 오류를 한 자리에 적고 파생물을 자동으로 만든다(TRD §2-2).
//!
//! 매크로가 만드는 것 넷: ① 인자·반환 struct(직렬화 derive 포함) ② `CommandSpec` 정적 항목(링커 수집)
//! ③ JSON Schema 텍스트 ④ 선언 crate 의 `CATALOG_VERSION`.
//!
//! ★스키마 텍스트는 **컴파일 타임 리터럴**이어야 한다★ — 링커 수집 항목은 const 로 평가되므로
//! `&'static str` 밖의 자료형을 쓸 수 없다. 그래서 타입→스키마 사전을 블록마다 `macro_rules!` 로 찍고
//! `concat!` 이 사용처에서 그것을 펼쳐 리터럴로 접는다(`concat!` 은 인자 안의 매크로를 먼저 편다).

/// 명령을 **생산자 모듈 옆**에서 선언한다.
///
/// ```ignore
/// declare_commands! {
///     catalog_version: 1;
///
///     struct AgentRow { id: String, name: String, parent: Option<String> }
///
///     /// 에이전트를 띄운다(잠든 것 깨우기 포함).
///     #[effect(Write)] #[since(1)]
///     "agent.spawn" => args AgentSpawnArgs {
///         target: Option<String>,
///         cwd: Option<String>,
///     } -> ok AgentSpawnOk {
///         agent_id: String,
///     } errors [NOT_FOUND];
/// }
/// ```
///
/// ## 만들어지는 것 (호출한 모듈 안에)
///
/// - `CATALOG_VERSION: u32` — 이 crate 의 어휘 세대. **선언이 바뀌면 손으로 올린다**(§4-①).
/// - `COMMAND_SPECS: &[&CommandSpec]` — 이 블록이 선언한 전량. [`crate::CommandTable::new`] 이 받는
///   「자기 crate 선언 집합」이다.
/// - 인자·반환 struct 와 블록 안 선언 타입(serde + ts-rs derive).
/// - `<인자타입>::SPEC` 과 그 링커 수집 항목.
///
/// ## 허용 타입 알파벳 (그 밖은 컴파일 에러)
///
/// | 쓰는 법 | JSON 에서의 뜻 | 필드 생략 |
/// |---|---|---|
/// | `String`·`bool`·정수·실수 | 그 원시 타입 | **불가**(required) |
/// | `Vec<T>` | 배열 | **불가**(required) |
/// | `Option<T>`·`Option<Vec<T>>` | 값 또는 `null` | 가능(optional) |
/// | `Option<Option<T>>` | 값 또는 `null` — ★**부재·`null`·값 셋을 가른다**★ | **불가**(required) |
/// | 이 블록에서 선언한 struct/enum | 인라인으로 펼쳐진 객체 / 문자열 열거 | **불가**(required) |
///
/// ★`Option<Option<T>>` 가 있는 이유★: `null` 이 **적극적 지시**인 자리(예: 부모를 떼라)에서 평범한
/// `Option<T>` 은 「안 줬다」와 「null 을 줬다」를 같은 값으로 접는다 — 오타 필드 하나가 조용히 그 지시로
/// 실행된다. 바깥은 **실렸나**, 안쪽은 **값이 있나**를 뜻하고, 부재는 역직렬화 단계에서 반려된다.
///
/// 정수는 Rust 타입의 범위가 `minimum`/`maximum` 으로 함께 실린다 — **64비트도 싣는다.**
///
/// ★알려진 부정확(64비트만)★: 수치를 배정도 실수로 파싱하는 검증기에서는 `2^64`·`±2^63` 부근의 경계가
/// 가장 가까운 double 로 반올림돼 **한 칸 헐거워진다**. 그래도 싣는 이유는 **안 싣는 쪽이 더 넓게
/// 틀리기** 때문이다 — 상한이 없으면 `18446744073709551616` 이 광고상 허용인데 역직렬화는 반려하고,
/// `i64` 는 하한마저 없어진다. 정확한 거절은 어느 쪽이든 역직렬화가 한다.
/// `usize`·`isize` 는 **64비트 타깃 기준**으로 싣는다(이 저장소의 타깃).
///
/// ## 제약
///
/// - **모듈 하나에 블록 하나** — 타입→스키마 사전 매크로의 이름이 고정이라 같은 모듈에 두 블록이면
///   뒤 정의가 앞을 가린다.
/// - **재귀 타입 불가**(`struct Node { child: Node }`) — 스키마가 인라인 전개라 끝나지 않는다. 중첩
///   여덟 겹에서 컴파일 에러로 멈춘다.
/// - **raw identifier 필드 금지**(`r#type`) — serde 가 쓰는 wire 이름(`type`)과 스키마에 찍히는 이름
///   (`r#type`)이 갈리는데 매크로가 문자열을 다듬을 수단이 없다. [`crate::lint_spec`] 이 잡는다.
/// - 소비 crate 는 `serde`(derive) 와 `ts-rs` 를 의존해야 하고, `Option<Option<T>>` 를 쓰면
///   `engram_dashboard_command` 라는 이름으로 이 crate 를 의존해야 한다(serde 의 `deserialize_with` 가
///   경로를 **문자열**로 받아 `$crate` 를 못 쓴다).
///
/// ## 하지 않는 것
///
/// - **핸들러를 만들지 않는다.** 실물은 `make_table(deps)` 가 조립 때 주입한다(규칙 T-1).
/// - **필드 주석을 스키마에 싣지 않는다** — 리터럴 안에 다시 문자열을 끼우면 따옴표를 이스케이프할
///   방법이 매크로에 없다. 명령 단위 설명(`summary`)만 실린다(런타임 직렬화라 안전하다).
/// - **공통 오류를 적게 하지 않는다** — `INVALID_ARGUMENT`·`INTERNAL` 은 표가 내는 것이라
///   [`crate::CommandSpec::advertised_errors`] 가 자동으로 얹는다.
// ADR-0149
#[macro_export]
macro_rules! declare_commands {
    (
        catalog_version: $version:literal;
        $($body:tt)*
    ) => {
        /// 이 crate 의 명령 어휘 세대(TRD §4-①). 봉투의 `proto_ver` 로 실린다.
        pub const CATALOG_VERSION: u32 = $version;

        $crate::__engram_declare!(@types [] $($body)*);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __engram_declare {
    // ── 1단계: 블록 안 타입 선언을 먼저 모은다(사전을 찍으려면 전량이 있어야 한다) ──────────────
    (@types [$($seen:tt)*] $(#[doc = $doc:literal])* struct $name:ident { $($fields:tt)* } $($rest:tt)*) => {
        $crate::__engram_declare!(@types [$($seen)* (struct $name [$($fields)*] [$($doc)*])] $($rest)*);
    };
    (@types [$($seen:tt)*] $(#[doc = $doc:literal])* enum $name:ident { $($variants:tt)* } $($rest:tt)*) => {
        $crate::__engram_declare!(@types [$($seen)* (enum $name [$($variants)*] [$($doc)*])] $($rest)*);
    };
    (@types [$($seen:tt)*] $($rest:tt)*) => {
        $crate::__engram_declare!(@emit [$($seen)*] $($rest)*);
    };

    // ── 2단계: 타입 정의 + 타입→스키마 사전 + 명령들 ─────────────────────────────────────────
    (@emit [$( ($kind:ident $tname:ident [$($tbody:tt)*] [$($tdoc:literal)*]) )*] $($commands:tt)*) => {
        $( $crate::__engram_declare!(@typedef $kind $tname [$($tbody)*] [$($tdoc)*]); )*

        /// 이 블록의 타입→JSON Schema 사전. `concat!` 이 사용처에서 편다.
        /// 첫 인자는 중첩 깊이(`[+ + …]` 한 덩이) — 재귀 타입이 무한히 펼쳐지지 않게 여덟 겹에서 멈춘다.
        /// ★여기 안에서는 `$(...)*` 반복을 쓸 수 없다★ — 바깥 매크로가 그것을 자기 반복으로 읽어 터진다.
        /// 그래서 깊이는 **덩이 하나**로 받아 그대로 넘기고, 증가는 바깥(`@type_schema`)이 한다.
        macro_rules! __engram_command_schema_of {
            ([+ + + + + + + +] $deep:ident) => {
                compile_error!(concat!(
                    "declare_commands!: `", stringify!($deep),
                    "` 에서 타입 중첩이 여덟 겹을 넘었다 — 재귀 struct/enum 은 알파벳 밖이다",
                    "(스키마가 인라인 전개라 끝나지 않는다)."
                ))
            };
            $( ($d:tt $tname) => {
                $crate::__engram_declare!(@type_schema $d $kind [$($tbody)*])
            }; )*
            ($d:tt String) => { "{\"type\":\"string\"}" };
            ($d:tt bool) => { "{\"type\":\"boolean\"}" };
            ($d:tt u8) => { "{\"type\":\"integer\",\"minimum\":0,\"maximum\":255}" };
            ($d:tt u16) => { "{\"type\":\"integer\",\"minimum\":0,\"maximum\":65535}" };
            ($d:tt u32) => { "{\"type\":\"integer\",\"minimum\":0,\"maximum\":4294967295}" };
            // 64비트 경계는 double 파서에서 반올림된다(위 doc) — 그래도 없는 것보다 좁다.
            ($d:tt u64) => {
                "{\"type\":\"integer\",\"minimum\":0,\"maximum\":18446744073709551615}"
            };
            ($d:tt usize) => {
                "{\"type\":\"integer\",\"minimum\":0,\"maximum\":18446744073709551615}"
            };
            ($d:tt i8) => { "{\"type\":\"integer\",\"minimum\":-128,\"maximum\":127}" };
            ($d:tt i16) => { "{\"type\":\"integer\",\"minimum\":-32768,\"maximum\":32767}" };
            ($d:tt i32) => {
                "{\"type\":\"integer\",\"minimum\":-2147483648,\"maximum\":2147483647}"
            };
            ($d:tt i64) => {
                "{\"type\":\"integer\",\"minimum\":-9223372036854775808,\"maximum\":9223372036854775807}"
            };
            ($d:tt isize) => {
                "{\"type\":\"integer\",\"minimum\":-9223372036854775808,\"maximum\":9223372036854775807}"
            };
            ($d:tt f32) => { "{\"type\":\"number\"}" };
            ($d:tt f64) => { "{\"type\":\"number\"}" };
            ($d:tt $other:ident) => {
                compile_error!(concat!(
                    "declare_commands!: 허용 타입 밖의 필드 타입 `", stringify!($other),
                    "` — String·bool·정수·실수·Option<T>·Vec<T>·Option<Option<T>>·",
                    "이 블록에서 선언한 struct/enum 만 쓴다."
                ))
            };
        }

        $crate::__engram_declare!(@commands [] $($commands)*);
    };

    // ── 타입 정의 ───────────────────────────────────────────────────────────────────────────
    (@typedef struct $name:ident [$($fields:tt)*] [$($doc:literal)*]) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*] [] $($fields)* ,);
    };
    (@typedef enum $name:ident [$($variants:tt)*] [$($doc:literal)*]) => {
        $( #[doc = $doc] )*
        #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize, ::ts_rs::TS)]
        pub enum $name { $($variants)* }
    };

    // struct 본체 — 필드를 하나씩 옮겨 적는다(끝에 붙인 `,` 가 종결 신호).
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*]) => {
        $( #[doc = $doc] )*
        #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize, ::ts_rs::TS)]
        pub struct $name { $($out)* }
    };
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*] ,) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*] [$($out)*]);
    };
    // 바깥 Option = 실렸나 · 안쪽 = 값이 있나. `transmitted` 가 그 둘을 가른다.
    // ★`#[serde(default)]` 를 달지 않는 것이 이 필드의 계약 그 자체다★ — 달면 부재가 조용히 바깥 `None`
    //   으로 접혀 **역직렬화를 통과하고**, 스키마가 required 라고 광고한 것을 핸들러 손검사에만 맡기게
    //   된다(그 검사를 안 쓴 다음 명령이 광고를 어긴다). `deserialize_with` + default 없음 = 필드가
    //   빠지면 serde 가 `missing field` 로 반려한다.
    // ★`skip_serializing_if` 는 남긴다★ — 바깥이 `None` 인 값(코드가 직접 만든 경우)을 그대로 실으면
    //   `null` 이 되어 「루트로 떼라」는 지시로 되읽힌다. 빼고 내보내야 상대가 반려한다.
    // ★ts-rs 는 이 속성을 못 읽고 무시한다("failed to parse" 경고 — 실측)★: 무시되는 편이 맞다.
    //   `skip_serializing_if` 를 읽으면 TS 칸이 `field?:` 가 되는데 그건 「부재 허용」이라 이 칸의 계약과
    //   정반대다. 그래서 TS 는 필수 + nullable 로 나온다(`string | null | null` — TS 에서 `string | null`
    //   과 같은 타입이다. 매크로가 `#[ts(as = …)]` 로 다듬을 수 없다 — 그 속성은 리터럴 문자열을 받는데
    //   `macro_rules!` 는 타입 이름을 문자열 안에 끼워 넣지 못한다).
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < Option < $ty:ident >> , $($rest:tt)*) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*]
            [$($out)* $(#[doc = $fdoc])*
             #[serde(deserialize_with = "engram_dashboard_command::transmitted",
                     skip_serializing_if = "Option::is_none")]
             pub $field : Option<Option<$ty>>,] $($rest)*);
    };
    // `Option<Vec<T>>` 의 닫는 꺾쇠 둘은 `>>` 한 토큰으로 렉싱된다 — 매크로가 쪼갤 수 없어 그대로 받는다.
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < Vec < $ty:ident >> , $($rest:tt)*) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*]
            [$($out)* $(#[doc = $fdoc])* #[ts(optional = nullable)]
             pub $field : Option<Vec<$ty>>,] $($rest)*);
    };
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < $ty:ident > , $($rest:tt)*) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*]
            [$($out)* $(#[doc = $fdoc])* #[ts(optional = nullable)]
             pub $field : Option<$ty>,] $($rest)*);
    };
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Vec < $ty:ident > , $($rest:tt)*) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*]
            [$($out)* $(#[doc = $fdoc])* pub $field : Vec<$ty>,] $($rest)*);
    };
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : $ty:ident , $($rest:tt)*) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*]
            [$($out)* $(#[doc = $fdoc])* pub $field : $ty,] $($rest)*);
    };

    // ── 스키마 텍스트 ───────────────────────────────────────────────────────────────────────
    // 깊이 증가는 여기서 한다 — 사전 매크로 안에서는 반복을 쓸 수 없기 때문이다(위 사전 doc).
    (@type_schema [$($d:tt)*] struct [$($fields:tt)*]) => {
        $crate::__engram_declare!(@obj [$($d)* +] "" "" [] [] $($fields)* ,)
    };
    (@type_schema [$($d:tt)*] enum [$($variants:tt)*]) => {
        $crate::__engram_declare!(@enum_schema "" [] $($variants)* ,)
    };

    // `required` 는 **생략 가능한 필드만 뺀다** — 스키마가 「없어도 된다」고 광고했는데 역직렬화가
    //   실패하면 그 광고를 보고 인자를 채우는 호출자(LLM)에게 거짓말을 한 것이다(ADR-0150).
    (@obj [$($d:tt)*] $psep:literal $rsep:literal [$($props:tt)*] [$($req:tt)*]) => {
        concat!("{\"type\":\"object\",\"properties\":{", $($props)* "},\"required\":[", $($req)* "]}")
    };
    (@obj [$($d:tt)*] $psep:literal $rsep:literal [$($props:tt)*] [$($req:tt)*] ,) => {
        $crate::__engram_declare!(@obj [$($d)*] $psep $rsep [$($props)*] [$($req)*])
    };
    (@obj [$($d:tt)*] $psep:literal $rsep:literal [$($props:tt)*] [$($req:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < Option < $ty:ident >> , $($rest:tt)*) => {
        $crate::__engram_declare!(@obj [$($d)*] "," ","
            [$($props)* $psep, "\"", stringify!($field), "\":{\"anyOf\":[",
             __engram_command_schema_of!([$($d)*] $ty), ",{\"type\":\"null\"}]}",]
            [$($req)* $rsep, "\"", stringify!($field), "\"",] $($rest)*)
    };
    (@obj [$($d:tt)*] $psep:literal $rsep:literal [$($props:tt)*] [$($req:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < Vec < $ty:ident >> , $($rest:tt)*) => {
        $crate::__engram_declare!(@obj [$($d)*] "," $rsep
            [$($props)* $psep, "\"", stringify!($field), "\":{\"anyOf\":[{\"type\":\"array\",\"items\":",
             __engram_command_schema_of!([$($d)*] $ty), "},{\"type\":\"null\"}]}",]
            [$($req)*] $($rest)*)
    };
    (@obj [$($d:tt)*] $psep:literal $rsep:literal [$($props:tt)*] [$($req:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < $ty:ident > , $($rest:tt)*) => {
        $crate::__engram_declare!(@obj [$($d)*] "," $rsep
            [$($props)* $psep, "\"", stringify!($field), "\":{\"anyOf\":[",
             __engram_command_schema_of!([$($d)*] $ty), ",{\"type\":\"null\"}]}",]
            [$($req)*] $($rest)*)
    };
    (@obj [$($d:tt)*] $psep:literal $rsep:literal [$($props:tt)*] [$($req:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Vec < $ty:ident > , $($rest:tt)*) => {
        $crate::__engram_declare!(@obj [$($d)*] "," ","
            [$($props)* $psep, "\"", stringify!($field), "\":{\"type\":\"array\",\"items\":",
             __engram_command_schema_of!([$($d)*] $ty), "}",]
            [$($req)* $rsep, "\"", stringify!($field), "\"",] $($rest)*)
    };
    (@obj [$($d:tt)*] $psep:literal $rsep:literal [$($props:tt)*] [$($req:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : $ty:ident , $($rest:tt)*) => {
        $crate::__engram_declare!(@obj [$($d)*] "," ","
            [$($props)* $psep, "\"", stringify!($field), "\":",
             __engram_command_schema_of!([$($d)*] $ty),]
            [$($req)* $rsep, "\"", stringify!($field), "\"",] $($rest)*)
    };

    (@enum_schema $sep:literal [$($acc:tt)*]) => {
        concat!("{\"enum\":[", $($acc)* "]}")
    };
    (@enum_schema $sep:literal [$($acc:tt)*] ,) => {
        $crate::__engram_declare!(@enum_schema $sep [$($acc)*])
    };
    (@enum_schema $sep:literal [$($acc:tt)*] $(#[doc = $vdoc:literal])* $variant:ident , $($rest:tt)*) => {
        $crate::__engram_declare!(@enum_schema "," [$($acc)* $sep, "\"", stringify!($variant), "\"",] $($rest)*)
    };

    // ── 명령들 ──────────────────────────────────────────────────────────────────────────────
    (@commands [$($declared:ident)*]) => {
        /// 이 블록이 선언한 전량 — `CommandTable::new` 이 받는 「자기 crate 선언 집합」.
        pub const COMMAND_SPECS: &[&$crate::CommandSpec] = &[ $( &$declared::SPEC, )* ];
    };
    (@commands [$($declared:ident)*]
        $(#[doc = $summary:literal])*
        #[effect($effect:ident)]
        #[since($since:literal)]
        $name:literal => args $args:ident { $($afields:tt)* }
                      -> ok  $ok:ident   { $($ofields:tt)* }
                      errors [ $($code:ident),* $(,)? ] ;
        $($rest:tt)*
    ) => {
        $crate::__engram_declare!(@struct_def $args [] [] $($afields)* ,);
        $crate::__engram_declare!(@struct_def $ok [] [] $($ofields)* ,);

        impl $args {
            /// 이 명령의 정적 계약. ★핸들러는 여기 없다★ — 실물은 `make_table(deps)` 가 준다(T-1).
            pub const SPEC: $crate::CommandSpec = $crate::CommandSpec {
                name: $name,
                effect: $crate::Effect::$effect,
                since: $since,
                summary: concat!($($summary),*),
                args_schema: $crate::__engram_declare!(@obj [] "" "" [] [] $($afields)* ,),
                ok_schema: $crate::__engram_declare!(@obj [] "" "" [] [] $($ofields)* ,),
                errors: &[ $( $crate::__engram_declare!(@code $code) ),* ],
                // 파생물(TS 바인딩) 커버리지 테스트가 선언에서 파일 이름을 끌어내는 재료 —
                //   손 목록이면 선언을 늘리는 그 편집이 함께 고쳐 무장 해제된다(`CommandSpec::args_type`).
                args_type: stringify!($args),
                ok_type: stringify!($ok),
            };
        }

        $crate::inventory::submit! { $crate::LinkedSpec(&$args::SPEC) }

        $crate::__engram_declare!(@commands [$($declared)* $args] $($rest)*);
    };

    // 오류 코드 표기(SCREAMING) → 어휘. 목록 밖 코드는 여기서 컴파일 에러가 된다.
    (@code INVALID_ARGUMENT) => { $crate::ErrorCode::InvalidArgument };
    (@code UNKNOWN_COMMAND) => { $crate::ErrorCode::UnknownCommand };
    (@code OWNER_UNAVAILABLE) => { $crate::ErrorCode::OwnerUnavailable };
    (@code OUTCOME_UNKNOWN) => { $crate::ErrorCode::OutcomeUnknown };
    (@code TIMEOUT) => { $crate::ErrorCode::Timeout };
    (@code REQUEST_ID_CONFLICT) => { $crate::ErrorCode::RequestIdConflict };
    (@code ALREADY_APPLIED) => { $crate::ErrorCode::AlreadyApplied };
    (@code AUTH_FAILED) => { $crate::ErrorCode::AuthFailed };
    (@code PROTOCOL_MISMATCH) => { $crate::ErrorCode::ProtocolMismatch };
    (@code NOT_FOUND) => { $crate::ErrorCode::NotFound };
    (@code CONFLICT) => { $crate::ErrorCode::Conflict };
    (@code UNSUPPORTED) => { $crate::ErrorCode::Unsupported };
    (@code INTERNAL) => { $crate::ErrorCode::Internal };
}

/// `Option<Option<T>>` 필드의 역직렬화 — **실렸나**(바깥)와 **값이 있나**(안쪽)를 가른다.
///
/// serde 는 `Option` 필드가 빠지면 자동으로 `None` 을 넣으므로, 이 함수 없이는 「필드 부재」와
/// 「`null` 을 명시했다」가 같은 값이 된다. 선언 매크로가 `deserialize_with` 로 이것을 단다.
pub fn transmitted<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}
