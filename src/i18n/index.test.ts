// Type assertions are gated by tsc, not the runtime — that is the `@ts-expect-error` in the 「컴파일 타임 회귀 방어」 (compile-time regression defense) block below.

import { afterEach, describe, expect, it, vi } from 'vitest'

import { en } from './en'
import { __test__, getLanguage, isLang, setLanguage, subscribeLanguage, t, type Translation } from './index'
import { ko } from './ko'

describe('t() — 중앙 문자열 조회', () => {
  it('placeholder 없는 키는 테이블 값을 그대로 반환', () => {
    expect(t('tab.create')).toBe('새 탭')
    expect(t('slot.setContent')).toBe('슬롯 콘텐츠 배치')
    expect(t('common.emptySlot')).toBe('- 비어있음 -')
  })

  it('반환값이 ko 테이블 원본과 일치(단일 소스 확인)', () => {
    expect(t('window.close')).toBe(ko.window.close)
    expect(t('agent.spawnInto')).toBe(ko.agent.spawnInto)
  })
})

describe('t() — 보간(interpolation)', () => {
  it('{name} placeholder 를 params 로 치환', () => {
    expect(t('tab.close', { name: '내 탭' })).toBe('탭 닫기: 내 탭')
  })

  it('index 를 넣어 기본 탭명 생성', () => {
    expect(t('common.defaultTabName', { index: '1' })).toBe('View 1')
  })

  it('같은 토큰이 여러 번 등장하면 전부(전역) 치환 — 첫 번째만 치환하는 회귀를 잡는다', () => {
    expect(t('common.duplicatePreview', { name: 'A' })).toBe('A / A')
    expect(t('common.duplicatePreview', { name: 'X' })).not.toContain('{name}')
  })
})

describe('t() — 보간 엣지(값 재스캔·정규식 특수문자·문법)', () => {
  it('치환값이 토큰을 품어도 재스캔하지 않는다 — 값 안의 {index} 는 리터럴로 남는다', () => {
    // The return value of the replace callback is not matched again (prevents infinite expansion · double substitution).
    expect(t('common.defaultTabName', { index: '{index}' })).toBe('View {index}')
  })

  it('정규식 특수문자($&·$1·\\)는 리터럴로 통과 — 치환 백레퍼런스로 해석되지 않는다', () => {
    // A function-form replace callback inserts the returned string as is ($& etc. special patterns are not interpreted). A case that would have broken with a string replacement.
    expect(t('tab.close', { name: '$&' })).toBe('탭 닫기: $&')
    expect(t('tab.close', { name: '$1' })).toBe('탭 닫기: $1')
    expect(t('tab.close', { name: 'a\\b' })).toBe('탭 닫기: a\\b')
  })

  it('placeholder 없는 문자열은 변경 없이 그대로 반환', () => {
    expect(t('tab.next')).toBe('다음 탭(순환)')
  })

  it('여분 param(값에 대응 토큰 없음)은 무시', () => {
    // By type, params cannot be passed to a key with no placeholder, but to confirm runtime safety we bypass with a cast and verify.
    const call = t as unknown as (k: string, p: Record<string, string>) => string
    expect(call('tab.create', { extra: '무시됨' })).toBe('새 탭')
  })

  it('누락 param(토큰은 있는데 값이 없음)은 {token} 원문을 유지(빈 문자열로 뭉개지 않음)', () => {
    const call = t as unknown as (k: string, p: Record<string, string>) => string
    expect(call('tab.close', {})).toBe('탭 닫기: {name}')
    expect(call('common.defaultTabName', {})).toBe('View {index}')
  })

  it('없는 키는 보간하지 않고 raw 키를 그대로 유지 — loud 하게 식별 가능(FIX②)', () => {
    // Even if a nonexistent key comes in through the unsafe-boundary (bypassing the type gate), the fallback key string is not interpolated.
    //   If `missing.{name}` were silently substituted into `missing.X`, the "nonexistent key" signal would vanish — this blocks that.
    const call = t as unknown as (k: string, p: Record<string, string>) => string
    expect(call('missing.{name}', { name: 'X' })).toBe('missing.{name}')
    expect(call('does.notexist', { name: 'X' })).toBe('does.notexist')
    expect(call('nodots', { name: 'X' })).toBe('nodots') // Key with no dot — early-return path
  })

  it('문법 밖 중괄호(비-word 토큰·빈 중괄호)는 placeholder 로 보지 않아 원문 유지', () => {
    // ★FIX①: it rides the real production implementation (interpolate) as is★ — not a local regex copy, it calls
    //   __test__.interpolate exported by index.ts. So if production WORD_PLACEHOLDER is changed to accept a malformed brace form
    //   the assertion below really breaks (regression safety net — grammar unification is truly enforced).
    const interp = __test__.interpolate
    expect(interp('{}', { '': 'x' })).toBe('{}')
    expect(interp('{ }', { ' ': 'x' })).toBe('{ }')
    expect(interp('{한글}', { 한글: 'x' })).toBe('{한글}')
    expect(interp('{first-name}', { 'first-name': 'x' })).toBe('{first-name}')
    // Control
    expect(interp('{name}', { name: 'ok' })).toBe('ok')
    // It must be the global (g) flag so repeated tokens are all substituted.
    expect(__test__.WORD_PLACEHOLDER.global).toBe(true)
  })

  it('중첩/짝 안 맞는 여는 중괄호는 grammar 계약 밖 — 런타임 raw 동작을 명시(FIX③)', () => {
    // For '{a{b}c' the runtime /\{(\w+)\}/g substitutes the inner {b} but the type-side Placeholders extracts never
    //   (type↔runtime split). It cannot be blocked with a pure regex (tracking an unmatched { is not regular), so we seal it with a *contract*
    //   — such a form is not put into table values, and that contract is enforced by the dev-guard below. This case
    //   records the raw behavior when a value that broke that contract comes in (for regression detection, not normal input).
    const interp = __test__.interpolate
    expect(interp('{a{b}c', { b: 'X' })).toBe('{aXc')
    // Control
    expect(interp('pre {name} post', { name: 'ok' })).toBe('pre ok post')
  })

  // ★grammar contract dev-guard classifier (FIX③ hardened)★: enforces index.ts's grammar contract ("a table value's placeholder is"
  //   only a matched single {\w+} — nested/unmatched braces forbidden"). After removing every matched well-formed placeholder
  //   ({\w+}), if even one { or } remains in the rest of the string = an unmatched or nested malformed brace.
  //   The old scan (/\{[^}]*\{/) caught only nested ('{a{b}c') and missed unclosed ('{name') · stray closing ('}') · excess ('{name}}')
  //   — this classifier catches all of them (seals typo braces leaking into broken labels after a mass migration).
  const hasMalformedBrace = (value: string): boolean => {
    const stripped = value.replace(/\{\w+\}/g, '')
    return stripped.includes('{') || stripped.includes('}')
  }

  it('분류기가 잘못된/짝 안 맞는 브레이스를 전부 잡고 정상 값은 통과시킨다(FIX③ 강화)', () => {
    // must-fail
    expect(hasMalformedBrace('{name')).toBe(true) // unclosed opening
    expect(hasMalformedBrace('안내 {')).toBe(true) // unclosed opening (trailing {)
    expect(hasMalformedBrace('}')).toBe(true) // stray closing
    expect(hasMalformedBrace('{name}}')).toBe(true) // excess closing
    expect(hasMalformedBrace('{a{b}c')).toBe(true) // nested (what the old scan caught — must keep being caught)
    // must-not-fail (no false positives)
    expect(hasMalformedBrace('{name}')).toBe(false) // well-formed single
    expect(hasMalformedBrace('View {index}')).toBe(false) // well-formed + surrounding text
    expect(hasMalformedBrace('{a}{b}')).toBe(false) // adjacent multiple (both valid)
    expect(hasMalformedBrace('탭 닫기')).toBe(false) // plain text with no braces
  })

  it('ko 테이블 값에 잘못된/짝 안 맞는 브레이스가 없다(FIX③ 계약 dev-guard)', () => {
    // ★grammar contract enforcement★: guarantees the type↔runtime split is unreachable from real table values.
    //   A table value's placeholder must be only a matched single {\w+} — if someone in the future puts in a value like '{name' (missing close) ·
    //   '{a{b}c' (nested) · '{name}}' (surplus close), this scan catches it at once (cuts off silent splits at the source).
    for (const [ns, group] of Object.entries(ko)) {
      for (const [key, value] of Object.entries(group as Record<string, string>)) {
        expect(hasMalformedBrace(value), `${ns}.${key} 값에 잘못된/짝 안 맞는 브레이스가 있음: ${value}`).toBe(
          false,
        )
      }
    }
  })

  it('en table values have no malformed or unmatched braces (FIX③ contract dev-guard)', () => {
    for (const [ns, group] of Object.entries(en)) {
      for (const [key, value] of Object.entries(group as Record<string, string>)) {
        expect(hasMalformedBrace(value), `${ns}.${key} has a malformed or unmatched brace: ${value}`).toBe(false)
      }
    }
  })
})

describe('en ↔ ko tables at run time', () => {
  // The type gate in ko.ts misses a `ko` value widened to `string` (a variable, a cast) when its `en` value has no token,
  //   and an `en` value that is itself widened. This scan holds whatever the values are.
  const tokens = (value: string): string[] =>
    [...new Set([...value.matchAll(__test__.WORD_PLACEHOLDER)].map(m => m[1]))].sort()

  it('every key has the same {word} token set in en and ko', () => {
    const koGroups = ko as unknown as Record<string, Record<string, string>>
    for (const [ns, group] of Object.entries(en as unknown as Record<string, Record<string, string>>)) {
      for (const [key, value] of Object.entries(group)) {
        const koValue = koGroups[ns]?.[key]
        expect(typeof koValue, `${ns}.${key} in ko`).toBe('string')
        expect(tokens(koValue ?? ''), `${ns}.${key}`).toEqual(tokens(value))
      }
    }
  })
})

describe('en table — runtime immutability (FIX E)', () => {
  it('is deep-frozen', () => {
    expect(Object.isFrozen(en)).toBe(true)
    expect(Object.isFrozen(en.tab)).toBe(true)
  })
})

describe('UI language (ADR-0372 · ADR-0373)', () => {
  // The setup file starts every test file in Korean; each case here leaves it that way.
  afterEach(() => {
    setLanguage('ko')
    document.documentElement.lang = 'ko'
  })

  /** A fresh copy of the module graph, booted from whatever `<html lang>` holds now. */
  async function bootFresh(): Promise<typeof import('./index')> {
    vi.resetModules()
    return import('./index')
  }

  it('boots in English when <html lang> holds nothing', async () => {
    document.documentElement.removeAttribute('lang')
    const fresh = await bootFresh()
    expect(fresh.getLanguage()).toBe('en')
    expect(fresh.t('tab.create')).toBe('New tab')
  })

  it('boots in English when <html lang> holds an unknown value', async () => {
    document.documentElement.lang = 'ko-KR'
    const fresh = await bootFresh()
    expect(fresh.getLanguage()).toBe('en')
  })

  it('boots in Korean when <html lang="ko"> is planted', async () => {
    document.documentElement.lang = 'ko'
    const fresh = await bootFresh()
    expect(fresh.getLanguage()).toBe('ko')
    expect(fresh.t('tab.create')).toBe('새 탭')
  })

  it('a module-scope t() right after import already resolves (synchronous init)', async () => {
    document.documentElement.lang = 'ko'
    vi.resetModules()
    const { MODULE_SCOPE_TITLE } = await import('./moduleScopeTitle.fixture')
    expect(MODULE_SCOPE_TITLE).toBe('새 탭')
  })

  it('setLanguage changes t() output, updates <html lang> and notifies once', () => {
    const listener = vi.fn()
    const unsubscribe = subscribeLanguage(listener)
    try {
      setLanguage('en')
      expect(getLanguage()).toBe('en')
      expect(t('tab.create')).toBe('New tab')
      expect(t('tab.close', { name: 'X' })).toBe('Close tab: X')
      expect(document.documentElement.lang).toBe('en')
      expect(listener).toHaveBeenCalledTimes(1)

      setLanguage('ko')
      expect(t('tab.create')).toBe('새 탭')
      expect(document.documentElement.lang).toBe('ko')
      expect(listener).toHaveBeenCalledTimes(2)
    } finally {
      unsubscribe()
    }
  })

  it('setLanguage with the current value does nothing', () => {
    const listener = vi.fn()
    const unsubscribe = subscribeLanguage(listener)
    try {
      document.documentElement.lang = 'marker'
      setLanguage('ko')
      expect(listener).not.toHaveBeenCalled()
      expect(document.documentElement.lang).toBe('marker')
    } finally {
      unsubscribe()
    }
  })

  it('setLanguage rejects a value that is not a known language and changes nothing', () => {
    const listener = vi.fn()
    const unsubscribe = subscribeLanguage(listener)
    const setAny = setLanguage as (lang: string) => boolean
    try {
      document.documentElement.lang = 'marker'
      expect(setAny('fr')).toBe(false)
      expect(setAny('KO')).toBe(false)
      expect(getLanguage()).toBe('ko')
      expect(t('tab.create')).toBe('새 탭')
      expect(document.documentElement.lang).toBe('marker')
      expect(listener).not.toHaveBeenCalled()

      expect(setLanguage('ko')).toBe(true) // The current value — accepted, nothing to do.
      expect(setLanguage('en')).toBe(true)
      expect(listener).toHaveBeenCalledTimes(1)
    } finally {
      unsubscribe()
    }
  })

  it('isLang accepts exactly en and ko', () => {
    expect(isLang('en')).toBe(true)
    expect(isLang('ko')).toBe(true)
    for (const other of ['', 'fr', 'KO', 'ko-KR', ' en', null, undefined, 1]) expect(isLang(other)).toBe(false)
  })

  it('a direct changeLanguage on the instance does not split the miss check and the lookup', () => {
    const { i18n } = __test__
    try {
      void i18n.changeLanguage('en')
      expect(t('tab.create')).toBe('새 탭')
    } finally {
      void i18n.changeLanguage('ko')
    }
  })

  it('an unsubscribed listener is not called', () => {
    const listener = vi.fn()
    subscribeLanguage(listener)()
    setLanguage('en')
    expect(listener).not.toHaveBeenCalled()
  })

  it('params named like i18next options (count, lng, context, defaultValue, ns) are plain values', () => {
    expect(t('chat.queuedMore', { count: '3' })).toBe('외 3개')
    const call = t as unknown as (k: string, p: Record<string, string>) => string
    expect(call('tab.close', { name: 'X', lng: 'en' })).toBe('탭 닫기: X')
    expect(call('tab.create', { context: 'c', defaultValue: 'd', ns: 'other', count: '2' })).toBe('새 탭')
  })

  it('<b>& in a param is not escaped', () => {
    expect(t('tab.close', { name: '<b>&' })).toBe('탭 닫기: <b>&')
  })

  it('a bare namespace or a key that is not exactly ns.key returns the key unchanged', () => {
    const call = t as unknown as (k: string) => string
    expect(call('tab')).toBe('tab')
    expect(call('tab.')).toBe('tab.')
    expect(call('.close')).toBe('.close')
    expect(call('tab.close.extra')).toBe('tab.close.extra')
    expect(call('tab.constructor')).toBe('tab.constructor')
  })

  describe('values no table holds (planted on the instance and restored)', () => {
    const { i18n } = __test__
    const original = ko.tab.create
    afterEach(() => {
      i18n.addResource('ko', 'translation', 'tab.create', original)
    })

    it('$t(...) in a value stays literal (no nesting)', () => {
      i18n.addResource('ko', 'translation', 'tab.create', 'pre $t(tab.closeCmd) post')
      expect(t('tab.create')).toBe('pre $t(tab.closeCmd) post')
    })

    it('an empty value returns an empty string', () => {
      i18n.addResource('ko', 'translation', 'tab.create', '')
      expect(t('tab.create')).toBe('')
    })
  })
})

describe('ko 테이블 — 런타임 불변(FIX E)', () => {
  it('deep-freeze 되어 있어 변조가 무시(또는 throw)된다', () => {
    // In non-strict mode the assignment is silently ignored and in strict it throws — either way, it passes if the value does not change.
    const before = ko.tab.close
    try {
      // @ts-expect-error runtime tampering attempt — it is read-only so the type rejects it, but we deliberately bypass to measure whether it is frozen.
      ko.tab.close = 'mutated'
    } catch {
      // strict mode TypeError — expected behavior.
    }
    expect(ko.tab.close).toBe(before)
    expect(Object.isFrozen(ko)).toBe(true)
    expect(Object.isFrozen(ko.tab)).toBe(true)
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// Compile-time regression defense (FIX C) — the load-bearing safety net of the mass migration.
//
// Each `@ts-expect-error` below is an assertion that "this line must be a tsc error". tsc type-checks this test file
// (npx tsc --noEmit), so if StringKey quietly widens to `string` or params degrades to `any`/optional, the error disappears →
// it becomes an unused `@ts-expect-error` and tsc fails *on that line* (the safety net fires).
// Runtime cost 0 (static check only) — so it is kept permanently (revives what a previous coder deleted as throwaway).
// eslint-disable-next-line @typescript-eslint/no-unused-vars -- scope for type gate verification (the calls are not executed)
function __typeGate__(): void {
  // @ts-expect-error bogus key — not in the StringKey union
  t('tab.doesNotExist')

  // @ts-expect-error tab.close requires {name} — no second argument
  t('tab.close')

  // @ts-expect-error tab.create has no placeholder — a second argument is forbidden
  t('tab.create', { name: 'x' })

  // @ts-expect-error name must be a string
  t('tab.close', { name: 123 })

  // @ts-expect-error required field name missing (other is an unknown field)
  t('tab.close', { other: 'x' })
}
void __typeGate__ // Reference only — no real call.

// ADR-0372: a translated table must have exactly the keys of `en` and, per key, the same placeholder set. Each case derives a
//   broken copy of the `ko` type; the control line shows the real `ko` passes, so the gate is not failing everything.
type Ko = typeof ko
type KoWithTab<Tab> = { readonly [N in keyof Ko]: N extends 'tab' ? Tab : Ko[N] }
type MissingKey = KoWithTab<Omit<Ko['tab'], 'close'>>
type ExtraKey = KoWithTab<Ko['tab'] & { readonly extra: '추가' }>
type RenamedPlaceholder = KoWithTab<Omit<Ko['tab'], 'close'> & { readonly close: '탭 닫기: {title}' }>
type DroppedPlaceholder = KoWithTab<Omit<Ko['tab'], 'close'> & { readonly close: '탭 닫기' }>
type AddedPlaceholder = KoWithTab<Omit<Ko['tab'], 'create'> & { readonly create: '새 탭 {name}' }>
type MissingNamespace = Omit<Ko, 'dialog'>
type ExtraNamespace = Ko & { readonly extra: { readonly a: '가' } }

// eslint-disable-next-line @typescript-eslint/no-unused-vars -- scope for type gate verification (never called)
function __translationGate__(
  missingKey: MissingKey,
  extraKey: ExtraKey,
  renamed: RenamedPlaceholder,
  dropped: DroppedPlaceholder,
  added: AddedPlaceholder,
  missingNs: MissingNamespace,
  extraNs: ExtraNamespace,
): void {
  ko satisfies Translation<Ko> // Control.

  // @ts-expect-error tab.close is missing
  missingKey satisfies Translation<MissingKey>
  // @ts-expect-error tab.extra is not in en
  extraKey satisfies Translation<ExtraKey>
  // @ts-expect-error tab.close has {title} instead of {name}
  renamed satisfies Translation<RenamedPlaceholder>
  // @ts-expect-error tab.close lost {name}
  dropped satisfies Translation<DroppedPlaceholder>
  // @ts-expect-error tab.create gained {name}
  added satisfies Translation<AddedPlaceholder>
  // @ts-expect-error the dialog namespace is missing
  missingNs satisfies Translation<MissingNamespace>
  // @ts-expect-error the extra namespace is not in en
  extraNs satisfies Translation<ExtraNamespace>
}
void __translationGate__ // Reference only — no real call.
