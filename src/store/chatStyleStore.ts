// ADR-0051: 채팅 렌더 간격·폰트 control surface — 프론트 전용 권위(Zustand slice)가 값을 소유하고
//   :root CSS 변수를 setProperty 로 갱신하며 localStorage 에 영속한다. 사람 UI 와 LLM(window.__engramChat)
//   이 **같은 store 액션**을 부른다(§5 단일 control surface). themeStore 패턴 계승 + 영속 추가
//   (themeStore 는 영속이 없어 새로고침 시 dark 로 리셋되는 함정이 있다 — 여기선 반복하지 않는다).
//
// ★권위 = 프론트★: 순수 렌더 프리퍼런스라 백엔드(데몬/settings.json/emit)를 안 태운다(ADR-0051 거부한
//   대안: 백엔드 영속). CSS 변수 레이어라 StructuredTextView/chat.css 는 var() 참조만 하고 값은 여기서만 쓴다.

import { create } from 'zustand'

/** 채팅 스타일 키(간격+폰트 세트). 값은 CSS 길이/숫자 문자열(예: '1rem', '13px', '1.55'). */
export type ChatStyleKey =
  | 'railRowPt' // rail 행 top-padding(행간 리듬)
  | 'plainRowPt' // 비-rail(user 버블·separator) 행 top-padding
  | 'userPy' // 유저 버블 세로 padding
  | 'userMy' // 유저 버블 세로 margin(턴 덩어리 분리)
  | 'railGutter' // rail gutter 폭
  | 'railLineOffset' // 연결선 top 오프셋(위 행으로 이어짐 — railRowPt 와 커플링, 보통 음수)
  | 'railDotTop' // 점 마커 top(콘텐츠 첫 줄 근처)
  | 'fontSize' // 채팅 base font-size(chat.css 와 동기)
  | 'lineHeight' // 채팅 base line-height

export type ChatStyleValues = Record<ChatStyleKey, string>

// ADR-0051: 기본값 — theme.css :root fallback 과 동기(둘 중 하나만 바뀌면 부팅 첫 프레임과 store 적용이
//   어긋난다). 이전 하드코딩(pt-3=0.75rem/1.45 등)보다 넉넉하게(사용자 지적: 우리 렌더가 좁고 산만).
export const CHAT_STYLE_DEFAULTS: ChatStyleValues = {
  railRowPt: '1rem',
  plainRowPt: '0.875rem',
  userPy: '0.5rem',
  userMy: '0.375rem',
  railGutter: '1.5rem',
  railLineOffset: '-1rem',
  railDotTop: '0.75rem',
  fontSize: '13px',
  lineHeight: '1.55',
}

// ADR-0051: store 키 → :root CSS 변수명. StructuredTextView/theme.css/chat.css 가 이 변수들을 var() 로 읽는다.
const CSS_VAR_BY_KEY: Record<ChatStyleKey, string> = {
  railRowPt: '--chat-rail-row-pt',
  plainRowPt: '--chat-plain-row-pt',
  userPy: '--chat-user-py',
  userMy: '--chat-user-my',
  railGutter: '--chat-rail-gutter',
  railLineOffset: '--chat-rail-line-offset',
  railDotTop: '--chat-rail-dot-top',
  fontSize: '--chat-font-size',
  lineHeight: '--chat-line-height',
}

const STORAGE_KEY = 'engram.chatStyle' // ADR-0051: localStorage 영속 키

/** localStorage 에서 저장된 값을 읽어 기본값 위에 병합한다. 부재/파싱 실패/타입 오류 → 기본값 fallback. */
export function loadChatStyle(): ChatStyleValues {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY)
    if (!raw) return { ...CHAT_STYLE_DEFAULTS }
    const parsed: unknown = JSON.parse(raw)
    if (parsed === null || typeof parsed !== 'object') return { ...CHAT_STYLE_DEFAULTS }
    const obj = parsed as Record<string, unknown>
    const merged: ChatStyleValues = { ...CHAT_STYLE_DEFAULTS }
    for (const key of Object.keys(CHAT_STYLE_DEFAULTS) as ChatStyleKey[]) {
      // 문자열 값만 채택(그 외 타입/누락은 기본값 유지) — 신뢰할 수 없는 저장값 방어.
      if (typeof obj[key] === 'string') merged[key] = obj[key]
    }
    return merged
  } catch {
    return { ...CHAT_STYLE_DEFAULTS }
  }
}

/** 값을 localStorage 에 저장. 실패(용량/비활성)해도 조용히 무시 — 스타일 프리퍼런스라 치명적이지 않다. */
function persist(values: ChatStyleValues): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(values))
  } catch {
    /* noop — 영속 실패는 무시(메모리 store 는 그대로 동작) */
  }
}

// ADR-0051 (FIX-2): control surface 는 런타임 무신뢰 경계다 — window.__engramChat.patch/set 은 LLM·외부
//   호출자가 부르고 TS 타입은 런타임 방어가 못 된다. 그래서 CHAT_STYLE_DEFAULTS 의 고정 키 목록을 단일
//   화이트리스트로 삼는다: applyToRoot 는 이 목록만 순회하고(values 에 낯선 키가 섞여도 setProperty 안 함),
//   set/patch 는 이 목록에 없는 키를 store·localStorage 진입 전에 걸러낸다.
const CHAT_STYLE_KEYS = Object.keys(CHAT_STYLE_DEFAULTS) as ChatStyleKey[]

/** 화이트리스트 판정 — 알려진 ChatStyleKey 인가(낯선 키는 store·CSS·저장 어디에도 못 들어간다). */
function isChatStyleKey(key: string): key is ChatStyleKey {
  // ADR-0051: 고정 배열 멤버십으로 판정한다. `key in CHAT_STYLE_DEFAULTS` 는 프로토타입 체인을 타서
  //   constructor·__proto__·toString 등 Object.prototype 상속 키가 true 로 통과 → store·localStorage 오염.
  //   CHAT_STYLE_KEYS(고정 9키)만 own key 로 인정해 프로토타입 오염을 원천 차단한다.
  return CHAT_STYLE_KEYS.includes(key as ChatStyleKey)
}

/**
 * 값을 :root CSS 변수에 적용(setProperty). document 부재(SSR/테스트) 시 no-op 방어.
 * ★고정 키 목록(CHAT_STYLE_KEYS)만 순회★ — Object.keys(values) 를 돌면 오염된 낯선 키가 잘못된
 *   setProperty 를 낳는다(FIX-2). values 는 화이트리스트로 이미 걸러진 것이지만 여기서도 고정 목록만 쓴다.
 */
function applyToRoot(values: ChatStyleValues): void {
  const root = globalThis.document?.documentElement
  if (!root) return
  for (const key of CHAT_STYLE_KEYS) {
    root.style.setProperty(CSS_VAR_BY_KEY[key], values[key])
  }
}

// ADR-0051 (FIX-1): 저장된 스타일 로드+적용을 데몬 부팅 경로에서 분리한다. chatStyle 은 프론트 전용
//   상태라 데몬 bootstrap 성공에 의존할 이유가 없다 — 예전엔 initEventBus(bootstrapDaemonIfNeeded 이후)
//   안에서 init() 을 불러, 데몬이 멈추면 저장값이 영영 적용 안 되고 정상 부팅에서도 첫 프레임이 기본값으로
//   깜빡였다. 이 함수를 main.tsx 최상단(첫 렌더 전)에서 부른다. document 부재(SSR/테스트)는 applyToRoot 가 방어.
export function loadAndApplyChatStyle(): void {
  const loaded = loadChatStyle()
  applyToRoot(loaded)
  useChatStyleStore.setState({ values: loaded })
}

interface ChatStyleState {
  values: ChatStyleValues
  /** 부팅 1회 — localStorage 로드 → CSS 변수 적용(값 부재 시 기본값). */
  init: () => void
  /** 단일 키 갱신 — CSS 변수 적용 + localStorage 저장. 사람 UI·LLM 공통 진입점. */
  setValue: (key: ChatStyleKey, value: string) => void
  /** 부분 병합 갱신(여러 키 한 번에). */
  patch: (partial: Partial<ChatStyleValues>) => void
  /** 기본값으로 초기화(+ 적용·저장). */
  reset: () => void
}

// ADR-0051: chat-style slice — 값의 유일 권위. 액션은 항상 (set → applyToRoot → persist) 3단을 함께 한다
//   (store·CSS·저장 3자 일관성). 초기 상태는 defaults(부팅 시 init() 이 localStorage 로 덮는다).
export const useChatStyleStore = create<ChatStyleState>((set, get) => ({
  values: { ...CHAT_STYLE_DEFAULTS },
  init: () => {
    const loaded = loadChatStyle()
    applyToRoot(loaded)
    set({ values: loaded })
  },
  setValue: (key, value) => {
    // FIX-2: 낯선 키(런타임 외부 호출)는 조용히 무시 — store·localStorage 오염 차단.
    if (!isChatStyleKey(key) || typeof value !== 'string') return
    const next = { ...get().values, [key]: value }
    applyToRoot(next)
    persist(next)
    set({ values: next })
  },
  patch: (partial) => {
    // FIX-2: 화이트리스트 밖 키·비문자열 값은 병합 전에 걸러낸다 — 낯선 키는 store·localStorage 에 못 들어간다.
    const clean: Partial<ChatStyleValues> = {}
    for (const [k, v] of Object.entries(partial ?? {})) {
      if (isChatStyleKey(k) && typeof v === 'string') clean[k] = v
    }
    const next = { ...get().values, ...clean }
    applyToRoot(next)
    persist(next)
    set({ values: next })
  },
  reset: () => {
    const next = { ...CHAT_STYLE_DEFAULTS }
    applyToRoot(next)
    persist(next)
    set({ values: next })
  },
}))
