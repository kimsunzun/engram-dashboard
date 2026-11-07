# When to wrap a third-party library — summary of criteria

- **Status:** light · applied to the component decision for the preset-library slot (「Engram 에 옮기면」 (If we carry this to Engram) below) · a whole-codebase audit is a separate item
- **Date:** 2026-10-14
- **Method:** research light. 1 collector · main grounding (Cockburn original) · no adversarial review.
- **Trigger:** the user's 「라이브러리는 항상 브릿지로 감싸야 한다 — 단 표준 라이브러리까지 감싸는 건 쓸데없다, 확인해 봐라」 (a library must always be wrapped in a bridge — but wrapping even the standard library is pointless, check it).

## Conclusion

1. **What to wrap = external systems · I/O boundaries** (files · network · processes · vendor APIs). Fowler's Gateway — wrap a complex external API in a general object interface so that on replacement only the inside is fixed (original · collector check). **Certain.**
2. **The unit of wrapping is not the library but the 「목적」 (purpose).** Cockburn's hexagonal architecture — ports are split not by technology but by 「목적과 대화의 성질」 (the purpose and the nature of the conversation) (back-translated; for example, one conversation for the purpose of data lookup, where only the adapter is swapped, whether it is a SQL DB or an in-memory mock) · intent = let the app be developed · tested without real devices · a DB · the number of ports is by intuition (2 to 4) (main original check). → It is not one wrapper per library. **Certain.**
3. **What not to wrap = the language standard · foundations used everywhere (serde · tokio · React itself) · pure helpers used in only one place.** There is practically no chance of swapping them, and a wrapper only costs. A thin 1:1 pass-through wrapper leaks the details below as they are, so there is no gain (Spolsky 「사소하지 않은 추상화는 모두 어느 정도 샌다」 (back-translated: every non-trivial abstraction leaks to some degree) — original · collector). **Likely** (no explicit grounding in an authoritative document was found — conventional wisdom).
4. **If a public API exports another crate's type, it is tied to that crate's stability** (Rust API Guidelines C-STABLE — original · collector). It is weak inside the app, and bites on types that cross a crate boundary. **Certain.**
5. 「직접 소유한 타입만 흉내 내라」 (only mock types you own) (GOOS) — do not mock an external type directly; keep your own thin adapter (from memory — unchecked).

## If we carry this to Engram (session judgment — reported to the user)

| Library | Wrap? | Where |
|---|---|---|
| walkdir · serde-saphyr · `std::fs` | **with a single purpose port** — 「라이브러리 저장소」 (library store) (providing the asset list · asset read · asset write). No separate wrappers around walkdir and YAML | daemon · tests use an in-memory fake |
| serde · tokio | not wrapped (foundation) | — |
| react-arborist · TanStack Table/Virtual · react-selecto | put inside our screen components (`LibraryTree` · `LibraryList` — placeholders) — the component we write anyway is itself the boundary | frontend |
| React · Zustand | not wrapped (foundation) | — |
| TanStack Query (if adopted) | foundation in nature — only about a 「슬롯 데이터 받아 오기」 (fetch slot data) hook | TRD |

## Sources

- https://martinfowler.com/eaaCatalog/gateway.html (collector checked the original)
- https://alistair.cockburn.us/hexagonal-architecture/ (main checked the original)
- https://rust-lang.github.io/api-guidelines/necessities.html (collector checked the original)
- https://www.joelonsoftware.com/2002/11/11/the-law-of-leaky-abstractions/ (collector checked the original)

## Limits

- It is light, so no adversarial review. We did not find an authoritative document for React design-system wrapper practice · a direct critique of 「래퍼 과잉」 (wrapper excess) · 「std · serde 는 감싸지 않는다」 (std · serde are not wrapped).
