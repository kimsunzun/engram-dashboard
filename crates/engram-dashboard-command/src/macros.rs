//! Declaration macro — writes name · argument shape · return · errors in one spot and builds the derivatives automatically (TRD §2-2).
//!
//! Builds: ① argument·return structs (with serialization derives) ② `CommandSpec` static items (linker collection) ③ JSON Schema text ④
//!   the declaring crate's `CATALOG_VERSION`.
//!
//! ★The schema text must be a **compile-time literal**★ — linker-collected items are const-evaluated, so only `&'static str` is possible.
//!   The type→schema dictionary is stamped per block
//! with `macro_rules!` and `concat!` folds it into a literal at the use site (`concat!` expands macros in its arguments first).

/// Declares commands **next to the producer module**.
///
/// ```ignore
/// declare_commands! {
///     catalog_version: 1;
///
///     struct AgentRow { id: String, name: String, parent: Option<String> }
///
///     /// Launches an agent (including waking a sleeping one).
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
/// ## What gets built (inside the calling module)
///
/// - `CATALOG_VERSION: u32` — this crate's vocabulary generation. **Raise it by hand when a declaration changes** (§4-①).
/// - `COMMAND_SPECS: &[&CommandSpec]` — everything this block declared. It is the 「own crate's declaration set」
///   [`crate::CommandTable::new`] receives.
/// - argument·return structs and types declared inside the block (serde + ts-rs derive).
/// - `<ArgsType>::SPEC` and its linker-collected item.
///
/// ## Allowed type alphabet (anything else is a compile error)
///
/// | How to write | Meaning in JSON | Field omission |
/// |---|---|---|
/// | `String`·`bool`·integers·floats | that primitive type | **not allowed** (required) |
/// | `Vec<T>` | array | **not allowed** (required) |
/// | `Option<T>`·`Option<Vec<T>>` | a value or `null` | allowed (optional) |
/// | `Option<Option<T>>` | a value or `null` — ★**tells apart absent·`null`·value**★ | **not allowed** (required) |
/// | a struct/enum declared in this block | an inline-expanded object / string enumeration | **not allowed** (required) |
///
/// ## Agent reference field — `#[agent_ref]` (ADR-0308)
///
/// Put it on a top-level `String`·`Option<String>` field of the argument struct, **after** the doc comment:
///
/// ```ignore
///     "slot.focus" => args SlotFocusArgs {
///         /// The agent to pick — a name or an id.
///         #[agent_ref]
///         agent: Option<String>,
///     } -> ok SlotFocusOk { } errors [NOT_FOUND];
/// ```
///
/// The marked field names are carried through [`crate::CommandSpec::agent_args`] → `agent_args` of the registration packet; before
///   relaying someone else's command the daemon turns that field's string
/// into an agent id with its own resolver, so the receiving handler sees **only the id**. Putting it on another type or a type declared in
///   the block is a compile error. Schema · TS bindings ignore the mark.
///
/// ★Why `Option<Option<T>>` exists★: where `null` is an **active instruction** (e.g. detach the parent), plain `Option<T>` folds "not
///   given" and "given null" — one typo'd field silently runs as that instruction.
/// The outer means **was it carried**, the inner **is there a value**; absence is bounced at deserialization.
///
/// Integers carry the Rust type's range as `minimum`/`maximum` too — **64-bit is carried as well.**
///
/// ★Known imprecision (64-bit only)★: validators parsing numbers as doubles round the bounds near `2^64`·`±2^63` and **loosen by one
///   step**. They are carried anyway because **omitting them is wrong more widely**
/// (`18446744073709551616` advertised as allowed, `i64` losing its lower bound); exact refusal is done by deserialization. `usize`·`isize`
///   are carried **on a 64-bit target basis**.
///
/// ## Constraints
///
/// - **One block per module** — the type→schema dictionary macro name is fixed; with two blocks the later shadows the earlier.
/// - **No recursive types** — the schema is inline expansion; it stops with a compile error at eight levels of nesting.
/// - **No raw identifier fields** (`r#type`) — serde's wire name (`type`) and the schema's (`r#type`) diverge and the macro cannot trim
///   strings. [`crate::lint_spec`] catches it.
/// - The consuming crate must depend on `serde` (derive) and `ts-rs`, and for `Option<Option<T>>` on this crate under the name
///   `engram_dashboard_command` (serde's `deserialize_with` takes the path as a **string**, so `$crate` is unusable).
///
/// ## What it does not do
///
/// - **It does not build handlers.** The real ones are injected by `make_table(deps)` at assembly (rule T-1).
/// - **It does not carry field comments into the schema** — the macro cannot escape quotes when nesting strings in the literal. Only the
///   per-command `summary` is carried (runtime serialization).
/// - **It does not make you write common errors** — `INVALID_ARGUMENT`·`INTERNAL` come from the table via
///   [`crate::CommandSpec::advertised_errors`].
// ADR-0155
#[macro_export]
macro_rules! declare_commands {
    (
        catalog_version: $version:literal;
        $($body:tt)*
    ) => {
        /// This crate's command vocabulary generation (TRD §4-①). Carried as the envelope's `proto_ver`.
        pub const CATALOG_VERSION: u32 = $version;

        $crate::__engram_declare!(@types [] $($body)*);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __engram_declare {
    // ── Step 1: collect the type declarations inside the block first (stamping the dictionary needs all of them) ──────────────
    (@types [$($seen:tt)*] $(#[doc = $doc:literal])* struct $name:ident { $($fields:tt)* } $($rest:tt)*) => {
        $crate::__engram_declare!(@types [$($seen)* (struct $name [$($fields)*] [$($doc)*])] $($rest)*);
    };
    (@types [$($seen:tt)*] $(#[doc = $doc:literal])* enum $name:ident { $($variants:tt)* } $($rest:tt)*) => {
        $crate::__engram_declare!(@types [$($seen)* (enum $name [$($variants)*] [$($doc)*])] $($rest)*);
    };
    (@types [$($seen:tt)*] $($rest:tt)*) => {
        $crate::__engram_declare!(@emit [$($seen)*] $($rest)*);
    };

    // ── Step 2: type definitions + type→schema dictionary + commands ─────────────────────────────────────────
    (@emit [$( ($kind:ident $tname:ident [$($tbody:tt)*] [$($tdoc:literal)*]) )*] $($commands:tt)*) => {
        $( $crate::__engram_declare!(@typedef $kind $tname [$($tbody)*] [$($tdoc)*]); )*

        /// This block's type→JSON Schema dictionary; `concat!` expands it at the use site. The first argument is the nesting depth (one
        ///   `[+ + …]` lump) — it stops at eight levels so recursive types do not expand forever.
        /// ★`$(...)*` repetition cannot be used inside★ — the outer macro reads it as its own repetition and blows up; so the depth is
        ///   passed on **as one lump** and incremented outside (`@type_schema`).
        macro_rules! __engram_command_schema_of {
            ([+ + + + + + + +] $deep:ident) => {
                compile_error!(concat!(
                    "declare_commands!: `", stringify!($deep),
                    "` exceeded eight levels of type nesting — recursive struct/enum is outside the alphabet",
                    " (the schema is inline expansion, so it never ends)."
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
            // 64-bit bounds get rounded in a double parser (doc above) — still narrower than having none.
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
                    "declare_commands!: field type outside the allowed types `", stringify!($other),
                    "` — use only String·bool·integers·floats·Option<T>·Vec<T>·Option<Option<T>>·",
                    "struct/enum declared in this block."
                ))
            };
        }

        $crate::__engram_declare!(@commands [] $($commands)*);
    };

    // ── Type definitions ───────────────────────────────────────────────────────────────────────────
    (@typedef struct $name:ident [$($fields:tt)*] [$($doc:literal)*]) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*] [] $($fields)* ,);
    };
    (@typedef enum $name:ident [$($variants:tt)*] [$($doc:literal)*]) => {
        $( #[doc = $doc] )*
        #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize, ::ts_rs::TS)]
        pub enum $name { $($variants)* }
    };

    // struct body — copies the fields one by one (the trailing `,` is the termination signal).
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*]) => {
        $( #[doc = $doc] )*
        #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize, ::ts_rs::TS)]
        pub struct $name { $($out)* }
    };
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*] ,) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*] [$($out)*]);
    };
    // Outer Option = was it carried · inner = is there a value. `transmitted` tells the two apart.
    // ★Not attaching `#[serde(default)]` is this field's contract★ — with it, absence silently folds into the outer `None` and **passes
    //   deserialization**, leaving what the schema advertised as required to the handler's manual check.
    //   `deserialize_with` + no default = serde bounces a missing field with `missing field`.
    // ★`skip_serializing_if` stays★ — an outer `None` built directly would otherwise go out as `null` and be read back as "detach to
    //   root"; it must be left out for the counterpart to bounce it.
    // ★ts-rs cannot read this attribute and ignores it ("failed to parse" warning)★ — right: reading `skip_serializing_if` would make the
    //   TS field `field?:` ("absence allowed"), the opposite of this contract.
    //   So the TS is required + nullable (`string | null | null` = `string | null`; `#[ts(as = …)]` cannot trim it — it takes a literal
    //   string and `macro_rules!` cannot put a type name in one).
    (@struct_def $name:ident [$($doc:literal)*] [$($out:tt)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < Option < $ty:ident >> , $($rest:tt)*) => {
        $crate::__engram_declare!(@struct_def $name [$($doc)*]
            [$($out)* $(#[doc = $fdoc])*
             #[serde(deserialize_with = "engram_dashboard_command::transmitted",
                     skip_serializing_if = "Option::is_none")]
             pub $field : Option<Option<$ty>>,] $($rest)*);
    };
    // The two closing angle brackets of `Option<Vec<T>>` are lexed as one `>>` token — the macro cannot split it, so it takes it as is.
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

    // ── Schema text ───────────────────────────────────────────────────────────────────────
    // The depth increment is done here — because repetition cannot be used inside the dictionary macro (dictionary doc above).
    (@type_schema [$($d:tt)*] struct [$($fields:tt)*]) => {
        $crate::__engram_declare!(@obj [$($d)* +] "" "" [] [] $($fields)* ,)
    };
    (@type_schema [$($d:tt)*] enum [$($variants:tt)*]) => {
        $crate::__engram_declare!(@enum_schema "" [] $($variants)* ,)
    };

    // `required` **leaves out only the omittable fields** — if the schema advertised 「may be absent」 and deserialization
    //   then failed, it would have lied to the caller (LLM) that fills arguments by that advertisement (ADR-0156).
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

    // ── Commands ──────────────────────────────────────────────────────────────────────────────
    (@commands [$($declared:ident)*]) => {
        /// Everything this block declared — the 「own crate's declaration set」 `CommandTable::new` receives.
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
        // Strip the `#[agent_ref]` marks of the argument fields first — the list of stripped fields and the remaining declaration go to
        //   `@command`.
        $crate::__engram_declare!(@agent_refs
            [ [$($summary)*] $effect $since $name $args $ok [$($ofields)*] [$($code)*] ]
            [] [] $($afields)* ,);

        $crate::__engram_declare!(@commands [$($declared)* $args] $($rest)*);
    };

    // ── Agent reference marks (ADR-0308) ─────────────────────────────────────────────────────────
    // Copy the fields one by one, removing `#[agent_ref]` and collecting those field names (the trailing `,` is the termination signal).
    // ★The mark goes **after** the doc comment★ — before it, no arm matches (compile error).
    // ★Stripped only from top-level fields of the argument struct★ — a mark on a type declared in the block (taken by
    //   `@struct_def`·`@obj`) has no arm (compile error); the daemon only changes top-level fields, so a nested mark has no effect.
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]) => {
        $crate::__engram_declare!(@command $cmd [$($clean)*] [$($refs)*]);
    };
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*] ,) => {
        $crate::__engram_declare!(@command $cmd [$($clean)*] [$($refs)*]);
    };
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]
        $(#[doc = $fdoc:literal])* #[agent_ref] $field:ident : String , $($rest:tt)*) => {
        $crate::__engram_declare!(@agent_refs $cmd
            [$($clean)* $(#[doc = $fdoc])* $field : String ,] [$($refs)* $field] $($rest)*);
    };
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]
        $(#[doc = $fdoc:literal])* #[agent_ref] $field:ident : Option < String > , $($rest:tt)*) => {
        $crate::__engram_declare!(@agent_refs $cmd
            [$($clean)* $(#[doc = $fdoc])* $field : Option<String> ,] [$($refs)* $field] $($rest)*);
    };
    // The daemon changes only **string values** — a mark on another shape does nothing, so it is not accepted silently.
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]
        $(#[doc = $fdoc:literal])* #[agent_ref] $field:ident : $($rest:tt)*) => {
        compile_error!(concat!(
            "declare_commands!: `#[agent_ref]` goes only on `String`·`Option<String>` fields — `",
            stringify!($field), "`"
        ));
    };
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < Option < $ty:ident >> , $($rest:tt)*) => {
        $crate::__engram_declare!(@agent_refs $cmd
            [$($clean)* $(#[doc = $fdoc])* $field : Option < Option < $ty >> ,] [$($refs)*] $($rest)*);
    };
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < Vec < $ty:ident >> , $($rest:tt)*) => {
        $crate::__engram_declare!(@agent_refs $cmd
            [$($clean)* $(#[doc = $fdoc])* $field : Option < Vec < $ty >> ,] [$($refs)*] $($rest)*);
    };
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]
        $(#[doc = $fdoc:literal])* $field:ident : Option < $ty:ident > , $($rest:tt)*) => {
        $crate::__engram_declare!(@agent_refs $cmd
            [$($clean)* $(#[doc = $fdoc])* $field : Option < $ty > ,] [$($refs)*] $($rest)*);
    };
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]
        $(#[doc = $fdoc:literal])* $field:ident : Vec < $ty:ident > , $($rest:tt)*) => {
        $crate::__engram_declare!(@agent_refs $cmd
            [$($clean)* $(#[doc = $fdoc])* $field : Vec < $ty > ,] [$($refs)*] $($rest)*);
    };
    (@agent_refs $cmd:tt [$($clean:tt)*] [$($refs:ident)*]
        $(#[doc = $fdoc:literal])* $field:ident : $ty:ident , $($rest:tt)*) => {
        $crate::__engram_declare!(@agent_refs $cmd
            [$($clean)* $(#[doc = $fdoc])* $field : $ty ,] [$($refs)*] $($rest)*);
    };

    // One command — receives the argument fields with marks stripped and the stripped field names.
    (@command [ [$($summary:literal)*] $effect:ident $since:literal $name:literal $args:ident $ok:ident
                [$($ofields:tt)*] [$($code:ident)*] ]
        [$($afields:tt)*] [$($refs:ident)*]) => {
        $crate::__engram_declare!(@struct_def $args [] [] $($afields)* ,);
        $crate::__engram_declare!(@struct_def $ok [] [] $($ofields)* ,);

        impl $args {
            /// This command's static contract. ★The handler is not here★ — the real one is given by `make_table(deps)` (T-1).
            pub const SPEC: $crate::CommandSpec = $crate::CommandSpec {
                name: $name,
                effect: $crate::Effect::$effect,
                since: $since,
                summary: concat!($($summary),*),
                args_schema: $crate::__engram_declare!(@obj [] "" "" [] [] $($afields)* ,),
                ok_schema: $crate::__engram_declare!(@obj [] "" "" [] [] $($ofields)* ,),
                errors: &[ $( $crate::__engram_declare!(@code $code) ),* ],
                // Material from which the derivative (TS binding) coverage test derives file names from the declaration —
                //   with a hand list, the very edit that adds a declaration would fix it too and disarm it (`CommandSpec::args_type`).
                args_type: stringify!($args),
                ok_type: stringify!($ok),
                agent_args: &[ $( stringify!($refs) ),* ],
            };
        }

        $crate::inventory::submit! { $crate::LinkedSpec(&$args::SPEC) }
    };

    // Error code notation (SCREAMING) → vocabulary. A code outside the list becomes a compile error here.
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

/// Deserialization of an `Option<Option<T>>` field — tells apart **was it carried** (outer) and **is there a value** (inner).
///
/// serde puts `None` for a missing `Option` field, so without this function "absent" and "explicit `null`" become the same value. The
///   declaration macro attaches it with `deserialize_with`.
pub fn transmitted<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}
