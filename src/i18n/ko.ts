// ADR-0069: UI 문자열 중앙화 — 네임스페이스드 `key → 한국어 문자열` 테이블(단일 언어).
//
// ★역할★: 사용자 노출 UI 문자열의 단일 소스(command 제목·우클릭 메뉴 라벨·컴포넌트 텍스트·aria-label·
//   기본 탭명·빈상태 등). drift(aria-label ↔ 표시 텍스트 불일치) 방지 + §5(LLM 이 라벨을 한 곳에서 읽기/수정)
//   + 튜토리얼 문자열 소스. 나중 로컬화 = `t()` 백엔드를 locale-aware 로 교체(이 테이블이 그 base).
//
// ★시드 전용(이 커밋)★: 인프라 형태를 증명하는 대표 엔트리만 담는다. 기존 ~100개 인라인 문자열의
//   실제 마이그레이션은 후속 커밋(command → 컴포넌트 점진). 여기 나열된 것이 전부가 아니다.
//
// ★네임스페이스 = UI 도메인★: 도메인별 1단 그룹(tab/slot/agent/preset/common …). 키 경로는
//   `namespace.key` 로 평탄화돼 `t('tab.close', …)` 형태로 접근된다(index.ts 참조).
//
// ★보간★: 값 안의 `{name}` 같은 `{placeholder}` 토큰은 `t()` 호출 시 params 로 치환된다. placeholder
//   가 있는 값은 index.ts 의 타입이 params 를 필수로 강제한다(오타·누락은 tsc 가 잡음).

/**
 * 네임스페이스드 UI 문자열 테이블. 최상위 키 = UI 도메인(namespace), 그 아래 = 개별 문자열 키.
 * `as const` 로 값을 리터럴 타입으로 고정한다 — index.ts 가 placeholder 유무를 값 리터럴에서 추론해
 * params 타입을 만든다(값을 넓은 `string` 으로 두면 그 추론이 불가능하므로 `as const` 는 필수).
 */
export const ko = {
  /** 탭(View) 관련 — command 제목·라벨. */
  tab: {
    create: '새 탭',
    switch: '탭 전환',
    close: '탭 닫기: {name}', // 보간 시드 — ADR-0069 예시.
    next: '다음 탭(순환)',
    rename: '탭 이름 변경',
  },
  /** 슬롯(레이아웃 한 칸) 관련. */
  slot: {
    setContent: '슬롯 콘텐츠 배치',
  },
  /** 창(WebView2 윈도우) 관련. */
  window: {
    create: '새 창',
    close: '창 닫기',
  },
  /** 에이전트(claude 프로세스) 관련. */
  agent: {
    spawnInto: '스폰 + 배치',
  },
  /** 프리셋(cwd 프리셋) 관련. */
  preset: {
    create: '프리셋 추가',
  },
  /** 도메인 공통 — 기본명·빈상태 등. */
  common: {
    emptySlot: '- 비어있음 -',
    defaultTabName: 'View {index}', // 보간 시드 — 기본 탭명(예: "View 1").
    // 반복 placeholder 시드 — 같은 토큰 2회. 전역 치환(global replace) 회귀 검증용(index.test.ts). ADR-0069.
    duplicatePreview: '{name} / {name}',
  },
} as const

/** `ko` 테이블의 정적 타입(index.ts 가 키 유니온·params 추론에 쓴다). */
export type KoTable = typeof ko

// ★단일 소스 무결성(FIX E)★: `as const` 는 컴파일 타임 readonly 일 뿐 — JS 소비자(또는 역직렬화 경계)는
//   런타임에 `ko.tab.close = ...` 로 변조해 t() 백엔드를 오염시킬 수 있다. deep-freeze 로 런타임에도 잠근다.
//   (타입 추론은 위 `as const` 가 계속 담당 — freeze 는 값만 동결하고 타입엔 영향 없다.)
function deepFreeze<T>(obj: T): T {
  if (obj && typeof obj === 'object') {
    for (const value of Object.values(obj)) deepFreeze(value)
    Object.freeze(obj)
  }
  return obj
}
deepFreeze(ko)
