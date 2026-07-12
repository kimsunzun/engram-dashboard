// t() accessor 단위테스트(ADR-0069) — 중앙 문자열 조회 + 보간 런타임 동작.
//
// 검증: 키 조회가 올바른 한국어 반환 · placeholder 치환(전역·엣지) · 타입세이프 key/params 의 컴파일 타임 회귀 방어.
// 타입 단언은 런타임이 아니라 tsc 가 게이트한다 — 아래 「컴파일 타임 회귀 방어」 블록의 `@ts-expect-error` 가 그것.

import { describe, expect, it } from 'vitest'

import { __test__, t } from './index'
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
    // duplicatePreview = '{name} / {name}' (진짜 반복 토큰). 전역 치환이 깨져 첫 토큰만 바뀌면 이 단언이 실패한다.
    expect(t('common.duplicatePreview', { name: 'A' })).toBe('A / A')
    // 두 번째 등장까지 확실히 바뀌었는지 — 원문 토큰이 하나도 안 남아야 한다.
    expect(t('common.duplicatePreview', { name: 'X' })).not.toContain('{name}')
  })
})

describe('t() — 보간 엣지(값 재스캔·정규식 특수문자·문법)', () => {
  it('치환값이 토큰을 품어도 재스캔하지 않는다 — 값 안의 {index} 는 리터럴로 남는다', () => {
    // replace 콜백 반환값은 다시 매칭 대상이 되지 않으므로 '{index}' 는 그대로 출력된다(무한 확장·이중 치환 방지).
    expect(t('common.defaultTabName', { index: '{index}' })).toBe('View {index}')
  })

  it('정규식 특수문자($&·$1·\\)는 리터럴로 통과 — 치환 백레퍼런스로 해석되지 않는다', () => {
    // 함수형 replace 콜백은 반환 문자열을 그대로 넣는다($& 등 특수 패턴 미해석). 문자열 replacement 였다면 깨졌을 케이스.
    expect(t('tab.close', { name: '$&' })).toBe('탭 닫기: $&')
    expect(t('tab.close', { name: '$1' })).toBe('탭 닫기: $1')
    expect(t('tab.close', { name: 'a\\b' })).toBe('탭 닫기: a\\b')
  })

  it('placeholder 없는 문자열은 변경 없이 그대로 반환', () => {
    expect(t('tab.next')).toBe('다음 탭(순환)')
  })

  it('여분 param(값에 대응 토큰 없음)은 무시', () => {
    // 타입상 placeholder 없는 키엔 params 를 못 넘기지만, 런타임 안전성 확인을 위해 캐스팅해 우회 검증.
    const call = t as unknown as (k: string, p: Record<string, string>) => string
    expect(call('tab.create', { extra: '무시됨' })).toBe('새 탭')
  })

  it('누락 param(토큰은 있는데 값이 없음)은 {token} 원문을 유지(빈 문자열로 뭉개지 않음)', () => {
    const call = t as unknown as (k: string, p: Record<string, string>) => string
    expect(call('tab.close', {})).toBe('탭 닫기: {name}')
    expect(call('common.defaultTabName', {})).toBe('View {index}')
  })

  it('없는 키는 보간하지 않고 raw 키를 그대로 유지 — loud 하게 식별 가능(FIX②)', () => {
    // unsafe-boundary(타입 게이트 우회)로 없는 키가 들어와도 fallback 키 문자열은 보간되지 않는다.
    //   `missing.{name}` 이 `missing.X` 로 조용히 치환되면 "없는 키"라는 신호가 사라진다 — 그걸 막는다.
    const call = t as unknown as (k: string, p: Record<string, string>) => string
    expect(call('missing.{name}', { name: 'X' })).toBe('missing.{name}') // 미스 → 리터럴 유지
    expect(call('does.notexist', { name: 'X' })).toBe('does.notexist') // 미스 → 원문
    expect(call('nodots', { name: 'X' })).toBe('nodots') // 점 없는 키 — 조기 반환, 쓰레기 namespace 없음
  })

  it('문법 밖 중괄호(비-word 토큰·빈 중괄호)는 placeholder 로 보지 않아 원문 유지', () => {
    // ★FIX①: 프로덕션 실물(interpolate)을 그대로 탄다★ — 로컬 정규식 복제본이 아니라 index.ts 가 export 한
    //   __test__.interpolate 를 호출한다. 그래서 프로덕션 WORD_PLACEHOLDER 를 잘못된 브레이스 형태를
    //   받아들이도록 바꾸면 아래 단언이 실제로 깨진다(회귀 안전망 — 문법 단일화가 진짜로 강제됨).
    const interp = __test__.interpolate
    expect(interp('{}', { '': 'x' })).toBe('{}') // 빈 중괄호 — \w+ 는 1자 이상 요구
    expect(interp('{ }', { ' ': 'x' })).toBe('{ }') // 공백 — word char 아님
    expect(interp('{한글}', { 한글: 'x' })).toBe('{한글}') // 비-ASCII — \w 밖
    expect(interp('{first-name}', { 'first-name': 'x' })).toBe('{first-name}') // 하이픈 — word char 아님
    // 대조군: word 토큰은 정상 치환.
    expect(interp('{name}', { name: 'ok' })).toBe('ok')
    // 프로덕션 매처는 전역(g) 플래그여야 반복 토큰이 전부 치환된다 — seam 실물이 그 플래그를 갖는지 확인.
    expect(__test__.WORD_PLACEHOLDER.global).toBe(true)
  })

  it('중첩/짝 안 맞는 여는 중괄호는 grammar 계약 밖 — 런타임 raw 동작을 명시(FIX③)', () => {
    // '{a{b}c' 에서 런타임 /\{(\w+)\}/g 는 안쪽 {b} 를 치환하지만 타입쪽 Placeholders 는 never 를 뽑는다
    //   (타입↔런타임 갈라짐). 순수 정규식으로는 막을 수 없어(짝 안 맞는 { 추적은 regular 아님) *계약*으로
    //   봉인한다 — 테이블 값엔 이런 형태를 넣지 않으며, 그 계약은 아래 dev-guard 가 강제한다. 이 케이스는
    //   그 계약을 어긴 값이 들어왔을 때의 raw 동작을 기록해 둔 것(회귀 감지용, 정상 입력 아님).
    const interp = __test__.interpolate
    expect(interp('{a{b}c', { b: 'X' })).toBe('{aXc') // 계약 밖 raw 동작(문서화) — 실제 테이블 값엔 이런 형태 금지
    // 대조군: 계약을 지킨 짝 맞는 단일 {name} 은 정상 치환.
    expect(interp('pre {name} post', { name: 'ok' })).toBe('pre ok post')
  })

  // ★grammar 계약 dev-guard 분류기(FIX③ 강화)★: index.ts 의 grammar 계약("테이블 값의 placeholder 는
  //   짝이 맞는 단일 {\w+} 뿐 — 중첩/짝 안 맞는 브레이스 금지")을 강제한다. 짝 맞는 well-formed placeholder
  //   ({\w+})를 전부 제거한 뒤, 남은 문자열에 { 나 } 가 하나라도 있으면 = 짝 안 맞거나 중첩된 잘못된 브레이스.
  //   옛 스캔(/\{[^}]*\{/)은 중첩('{a{b}c')만 잡았고 unclosed('{name')·stray closing('}')·excess('{name}}')는
  //   놓쳤다 — 이 분류기는 그 전부를 잡는다(대량 마이그레이션 후 오타 브레이스가 깨진 라벨로 새는 걸 봉인).
  const hasMalformedBrace = (value: string): boolean => {
    const stripped = value.replace(/\{\w+\}/g, '') // 짝 맞는 well-formed placeholder 제거
    return stripped.includes('{') || stripped.includes('}') // 남은 브레이스 = 잘못된 형태
  }

  it('분류기가 잘못된/짝 안 맞는 브레이스를 전부 잡고 정상 값은 통과시킨다(FIX③ 강화)', () => {
    // must-fail: unclosed opening · stray/excess closing · nested — 전부 계약 위반으로 탐지되어야 한다.
    expect(hasMalformedBrace('{name')).toBe(true) // unclosed opening
    expect(hasMalformedBrace('안내 {')).toBe(true) // unclosed opening(트레일링 {)
    expect(hasMalformedBrace('}')).toBe(true) // stray closing
    expect(hasMalformedBrace('{name}}')).toBe(true) // excess closing
    expect(hasMalformedBrace('{a{b}c')).toBe(true) // nested(옛 스캔이 잡던 것 — 계속 잡혀야 함)
    // must-not-fail: 짝 맞는 단일/복수 placeholder · 브레이스 없는 평문 — 오탐 금지.
    expect(hasMalformedBrace('{name}')).toBe(false) // well-formed 단일
    expect(hasMalformedBrace('View {index}')).toBe(false) // well-formed + 주변 텍스트
    expect(hasMalformedBrace('{a}{b}')).toBe(false) // 인접 복수(둘 다 유효)
    expect(hasMalformedBrace('탭 닫기')).toBe(false) // 브레이스 없는 평문
  })

  it('ko 테이블 값에 잘못된/짝 안 맞는 브레이스가 없다(FIX③ 계약 dev-guard)', () => {
    // ★grammar 계약 강제★: 타입↔런타임 갈라짐이 real 테이블 값에서 도달 불가함을 보장한다.
    //   테이블 값의 placeholder 는 짝이 맞는 단일 {\w+} 뿐이어야 한다 — 미래에 누가 '{name'(닫힘 누락)·
    //   '{a{b}c'(중첩)·'{name}}'(잉여 닫힘) 같은 값을 넣으면 이 스캔이 즉시 잡는다(silent 갈라짐 원천 차단).
    for (const [ns, group] of Object.entries(ko)) {
      for (const [key, value] of Object.entries(group as Record<string, string>)) {
        expect(hasMalformedBrace(value), `${ns}.${key} 값에 잘못된/짝 안 맞는 브레이스가 있음: ${value}`).toBe(
          false,
        )
      }
    }
  })
})

describe('ko 테이블 — 런타임 불변(FIX E)', () => {
  it('deep-freeze 되어 있어 변조가 무시(또는 throw)된다', () => {
    // 비-strict 모드에선 대입이 조용히 무시되고 strict 에선 throw — 어느 쪽이든 값이 안 바뀌면 통과.
    const before = ko.tab.close
    try {
      // @ts-expect-error 런타임 변조 시도 — 읽기전용이라 타입상 거부되지만, 동결 여부를 실측하려 의도적으로 우회 시도.
      ko.tab.close = 'mutated'
    } catch {
      // strict 모드 TypeError — 예상된 동작.
    }
    expect(ko.tab.close).toBe(before)
    expect(Object.isFrozen(ko)).toBe(true)
    expect(Object.isFrozen(ko.tab)).toBe(true) // deep — 네임스페이스 객체도 동결
  })
})

// ─────────────────────────────────────────────────────────────────────────────
// 컴파일 타임 회귀 방어(FIX C) — 대량 마이그레이션의 load-bearing 안전망.
//
// 아래 각 `@ts-expect-error` 는 "이 줄은 반드시 tsc 에러여야 한다"는 단언이다. tsc 는 이 테스트 파일을
// 타입체크하므로(npx tsc --noEmit), StringKey 가 몰래 `string` 으로 넓어지거나 params 가 `any`/optional 로
// 퇴화하면 에러가 사라지고 → 사용되지 않은 `@ts-expect-error` 가 되어 tsc 가 *그 줄에서* 실패한다(안전망 발화).
// 런타임 비용 0(정적 검사 전용) — 그래서 영구 보존한다(이전 코더가 throwaway 로 지운 것을 되살림).
// eslint-disable-next-line @typescript-eslint/no-unused-vars -- 타입 게이트 검증용 스코프(호출은 실행 아님)
function __typeGate__(): void {
  // (1) 존재하지 않는 키 = 컴파일 에러.
  // @ts-expect-error bogus key — StringKey union 에 없음
  t('tab.doesNotExist')

  // (2) placeholder 있는 키에 params 누락 = 컴파일 에러.
  // @ts-expect-error tab.close 는 {name} 필수 — 두 번째 인자 없음
  t('tab.close')

  // (3) placeholder 없는 키에 params 전달(금지된 param) = 컴파일 에러.
  // @ts-expect-error tab.create 는 placeholder 없음 — 두 번째 인자 금지
  t('tab.create', { name: 'x' })

  // (4) param 값 타입 오류(string 기대인데 number) = 컴파일 에러.
  // @ts-expect-error name 은 string 이어야 함
  t('tab.close', { name: 123 })

  // (5) 잘못된 param 이름(요구 필드 name 을 안 주고 다른 이름) = 컴파일 에러.
  // @ts-expect-error 요구 필드 name 누락(other 는 미지의 필드)
  t('tab.close', { other: 'x' })
}
void __typeGate__ // 참조만 — 실제 호출 없음(런타임 무비용).
