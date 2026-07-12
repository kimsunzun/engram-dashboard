// ★잠정(interim) 구현 — 자체 경량 t()★: 지금은 한국어 단일 언어라 외부 i18n 라이브러리 없이 의존성 0으로
//   충분하다(리서치 결론 — 단일언어·내부툴엔 자체 t()가 정당, anti-pattern 아님). 실제 다국어(복수형·ICU·
//   날짜·번역가 TMS 등) 본격화 시, 아래 "안정 API 계약"의 seam 을 통해 공개 라이브러리(Lingui = 경량·Vite 공식 /
//   i18next = 최대 생태계)로 이 t() 뒤(백엔드)를 교체할 예정이다 — 호출부·key·시그니처 불변. 이 파일은 그때까지의
//   과도기 방편이다. 근거: /research medium 서베이(2026-07-12, ADR-0069 재확인).
//
// ADR-0069: `t(key, params?)` accessor — UI 문자열 중앙화의 안정 API.
//
// ★안정 API 계약★: 호출부는 `t('tab.close', { name })` 형태로만 문자열을 얻는다. 지금은 단일 언어(`ko`)만
//   반환하지만, 나중 로컬화는 이 함수 *내부*(백엔드)만 locale-aware 로 교체하면 되고 `(key, params)`
//   시그니처·모든 호출부·key 는 불변이다. 그래서 이 형태가 로컬화 base 다(ADR-0069 §영향/불변식).
//
// ★타입세이프 key(필수)★: `key` 타입을 `ko` 테이블에서 파생한 union 으로 만들어 오타·존재하지 않는 키를
//   tsc 가 컴파일 타임에 잡는다(대량 마이그레이션의 안전망 — ADR-0069 근거). 존재하지 않는 키 호출 = 컴파일 에러.
//
// ★타입세이프 params(nice-to-have, 구현함)★: 각 문자열 값 리터럴에서 `{placeholder}` 토큰을 template-literal
//   타입으로 추출해, placeholder 가 있는 키는 그 이름들을 필수 필드로 갖는 params 를 강제하고, 없는 키는
//   params 를 금지한다(잘못된/누락 param 을 tsc 가 잡음). ko.ts 의 `as const` 가 이 추론의 전제다.

import { ko, type KoTable } from './ko'

/** 유효한 문자열 키 유니온(예: `'tab.close' | 'tab.create' | 'common.emptySlot' | …`) — 두 단계 keyof 곱. */
export type StringKey = {
  [N in keyof KoTable]: `${N & string}.${keyof KoTable[N] & string}`
}[keyof KoTable]

/**
 * 평탄화 키 `K`("namespace.sub")로 `ko` 테이블에서 *정확한* 값 리터럴을 되짚는다.
 * ★per-key 정밀 인덱싱★: 키-remap 매핑 타입은 sub-값을 union 으로 뭉개므로(placeholder 유무가 섞임)
 *   쓰지 않는다. 대신 K 를 `${N}.${S}` 로 분해해 `KoTable[N][S]` 를 직접 인덱싱한다.
 */
type ValueOf<K extends StringKey> = K extends `${infer N}.${infer S}`
  ? N extends keyof KoTable
    ? S extends keyof KoTable[N]
      ? KoTable[N][S]
      : never
    : never
  : never

/**
 * ★문법 동기화(FIX A — 최우선)★: 아래 두 정의(타입 추출 `Placeholders` ⊕ 런타임 `WORD_PLACEHOLDER`)는
 *   **반드시 같은 문법**을 봐야 한다. 어긋나면 타입이 param 을 *요구*하는데 런타임은 치환을 *안 하는*(또는 그 반대)
 *   갈라짐이 생긴다(키는 안정 API 라 치명적). 채택 문법 = **비어있지 않은 word char(`[A-Za-z0-9_]`, 즉 `\w`) 토큰**.
 *   `{한글}`·`{first-name}`·`{ }`·`{}` 는 양쪽 모두 placeholder 로 *인식하지 않는다*(param 요구 안 함 + 원문 유지).
 *   런타임 매처(WORD_PLACEHOLDER)를 바꾸면 이 타입도 반드시 함께 바꾼다.
 */
type WordChar =
  | 'a' | 'b' | 'c' | 'd' | 'e' | 'f' | 'g' | 'h' | 'i' | 'j' | 'k' | 'l' | 'm'
  | 'n' | 'o' | 'p' | 'q' | 'r' | 's' | 't' | 'u' | 'v' | 'w' | 'x' | 'y' | 'z'
  | 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G' | 'H' | 'I' | 'J' | 'K' | 'L' | 'M'
  | 'N' | 'O' | 'P' | 'Q' | 'R' | 'S' | 'T' | 'U' | 'V' | 'W' | 'X' | 'Y' | 'Z'
  | '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '_'

/** `T` 가 **비어있지 않은** word char(`\w`) 만으로 이뤄졌는지 판정 — 런타임 `\w+` 와 문자 클래스 일치. */
type IsWord<T extends string> = T extends ''
  ? false
  : T extends `${WordChar}${infer R}`
    ? R extends ''
      ? true
      : IsWord<R>
    : false

/**
 * 문자열 값 리터럴 `S` 에서 `{name}` 스타일 placeholder 이름을 union 으로 추출한다.
 * `{...}` 안 토큰이 **word char(`\w`) 로만 이뤄진 비어있지 않은 토큰**일 때만 param 으로 인정한다
 * (런타임 `WORD_PLACEHOLDER = /\{(\w+)\}/g` 와 동일 문법 — FIX A). placeholder 가 없으면 `never`.
 * (예: `'탭 닫기: {name}'` → `'name'`, `'{a}{b}'` → `'a' | 'b'`, `'{한글}'`·`'no ph'` → `never`)
 *
 * ★비탐욕 매칭 주의★: `${string}{${infer P}}${infer Rest}` 는 TS 에서 `P` 를 첫 `}` 까지로 최소 매칭한다.
 *   `{ }`·`{first-name}` 처럼 `P` 가 word 검증에 실패하면 그 토큰은 버리되 `Rest` 를 계속 훑어 뒤쪽 유효 토큰은 살린다.
 */
type Placeholders<S extends string> = S extends `${string}{${infer P}}${infer Rest}`
  ? (IsWord<P> extends true ? P : never) | Placeholders<Rest>
  : never

/** 키 `K` 의 params 타입 — placeholder 각각을 string 필드로. placeholder 없으면 빈 객체(→ 인자 금지). */
type ParamsFor<K extends StringKey> = { [P in Placeholders<ValueOf<K>>]: string }

/**
 * 키 `K` 의 params 인자 튜플 — 가변 인자 자리에 펼쳐 params 를 필수/금지로 가른다.
 * placeholder 가 없으면 `[]`(두 번째 인자 금지), 있으면 `[ParamsFor<K>]`(placeholder 이름들이 필수 필드).
 * ★튜플 rest 방식★: `t<K>(key: K, ...rest: ParamsArg<K>)` 로 두면 K 는 첫 인자에서 그대로 추론되고
 *   (조건부 타입에 안 묻혀 추론 실패 없음), params 필수/금지는 rest 튜플 길이가 강제한다.
 * ★여분 필드 주의(FIX F)★: TS 의 excess-property 검사는 **인라인 객체 리터럴**을 넘길 때만 여분 필드를 막는다.
 *   미리 만든 객체 변수를 넘기면 구조적 타이핑상 여분 필드가 통과한다(TS 한계 — 완전 차단은 하지 않는다).
 *   런타임은 대응 토큰 없는 여분 필드를 무시하므로 무해하다(index.test.ts 「여분 param」 참조).
 */
type ParamsArg<K extends StringKey> = Placeholders<ValueOf<K>> extends never ? [] : [params: ParamsFor<K>]

/**
 * ★런타임 placeholder 문법(FIX A)★: 위 타입 추출 `Placeholders`/`IsWord` 와 **반드시 일치**하는 단일 매처.
 *   `\w+` = 비어있지 않은 word char(`[A-Za-z0-9_]`) 토큰만 placeholder 로 본다. 이걸 바꾸면 타입 쪽도 함께 바꾼다.
 *
 * ★타입↔런타임 grammar 계약(FIX③ — 중첩/짝 안 맞는 `{` 봉인)★: `{a{b}c` 처럼 여는 `{` 가 짝 없이 안에 또
 *   끼면 이 런타임 매처는 안쪽 `{b}` 를 치환하지만, 타입쪽 `Placeholders`(첫 `{` 를 여는 브레이스로 소비 →
 *   `a{b` 가 word 검증 실패)는 `never` 를 뽑는다 → **런타임만 몰래 치환**하는 갈라짐. 이건 순수 정규식으로
 *   막을 수 없다(짝 안 맞는 여는 브레이스 추적은 regular 하지 않다 — 고정폭 lookbehind 로 표현 불가).
 *   그래서 **계약으로 봉인한다**: *테이블 값의 placeholder 는 짝이 맞는 단일 `{\w+}` 뿐이며 중첩/짝 안 맞는
 *   `{` 를 넣지 않는다*(ko.ts 값 작성 규약). 이 계약은 index.test.ts 의 dev-guard(ko 전체 스캔)가 강제해,
 *   미래에 위반 값이 들어오면 테스트가 즉시 실패한다 → real 테이블 값에서 도달 가능한 silent 갈라짐이 없다.
 */
const WORD_PLACEHOLDER = /\{(\w+)\}/g

/** 테스트 seam(FIX①): 프로덕션 매처/보간을 그대로 노출 — 테스트가 로컬 복제본 대신 이 실물을 탄다. */
export const __test__ = { WORD_PLACEHOLDER, interpolate }

/**
 * ★런타임 조회 seam★: 지금은 `ko` 한 장에서만 값을 꺼낸다. 나중 로컬화는 여기(active locale 선택)만 바꾸면 된다 —
 *   시그니처·호출부 불변(ADR-0069). `namespace.key` 를 두 조각으로 나눠 중첩 테이블에서 값을 읽는다.
 *   반환은 `[value, found]` 튜플 — 호출부(t)가 미스일 때 보간을 건너뛰어 키를 loud 하게 유지한다(FIX②).
 */
function lookup(key: StringKey): [value: string, found: boolean] {
  const dot = key.indexOf('.')
  // ★no-dot 조기 반환(FIX②)★: 점이 없으면 slice(0,-1) 로 쓰레기 namespace 를 만들지 말고 즉시 키를 그대로 돌려준다.
  //   (unsafe-boundary 로 들어온 형식 밖 키 — 미스로 취급해 loud 하게 원문 유지.)
  if (dot < 0) return [key, false]
  const ns = key.slice(0, dot) as keyof KoTable
  const sub = key.slice(dot + 1)
  // ★unsafe-boundary backstop(FIX E)★: 정상 경로는 위 StringKey union 이 키 존재를 컴파일 타임에 보장한다.
  //   그러나 JS/역직렬화 경계(타입 게이트를 우회한 런타임 호출)에선 없는 키가 들어올 수 있다 — 그때
  //   `undefined.replace(...)` 로 터지지 않도록 키 문자열 자체를 fallback 으로 돌려준다(loud-visible, no-throw).
  const value = (ko[ns] as Record<string, string> | undefined)?.[sub]
  return value === undefined ? [key, false] : [value, true]
}

/**
 * `{name}` 스타일 placeholder 를 params 값으로 치환한다. params 에 없는 토큰은 원문 그대로 남긴다(loud-blank 방지).
 * 문법 = `WORD_PLACEHOLDER`(= 타입쪽 `Placeholders` 와 동기화). replace 콜백은 치환값을 재스캔하지 않으므로
 * 치환값 안의 `{token}`·정규식 특수문자(`$&`·`$1`·`\`)는 리터럴로 그대로 들어간다(백레퍼런스 해석 안 됨).
 */
function interpolate(template: string, params: Record<string, string>): string {
  return template.replace(WORD_PLACEHOLDER, (whole, name: string) =>
    // params 에 해당 키가 있으면 치환, 없으면 `{name}` 원문 유지(빈 문자열로 뭉개지 않는다).
    Object.prototype.hasOwnProperty.call(params, name) ? params[name] : whole,
  )
}

/**
 * 중앙 UI 문자열을 조회(+ 보간)한다. `key` 는 `ko` 테이블 파생 union — tsc 가 오타·미존재 키를 잡는다.
 * placeholder 있는 키는 params 필수, 없는 키는 params 금지(ParamsArg 튜플이 강제).
 */
export function t<K extends StringKey>(key: K, ...rest: ParamsArg<K>): string {
  const [template, found] = lookup(key)
  const params = rest[0] as Record<string, string> | undefined
  // ★미스는 loud 유지(FIX②)★: 키를 못 찾으면 fallback 인 raw 키 문자열은 **보간하지 않는다**. 안 그러면
  //   unsafe-boundary 로 들어온 `missing.{name}` 이 `missing.X` 로 치환돼 "없는 키"라는 신호가 지워진다.
  //   미스일 땐 원문(키 그대로)을 반환해 식별 가능하게 남긴다.
  if (!found) return template
  return params ? interpolate(template, params) : template
}
