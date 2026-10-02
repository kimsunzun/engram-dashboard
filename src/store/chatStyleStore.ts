// 셸 설정 `chat.style.*` 을 :root CSS 변수에 붙이는 적용자 — 값의 권위는 셸 설정이고(ADR-0265), 여기는 받은 값을
//   화이트리스트(`CSS_VAR_BY_KEY`)로 변수 이름에 옮겨 setProperty 할 뿐이다. StructuredTextView/ChatRow/chat.css 는
//   var() 만 읽는다(ADR-0051 의 CSS 변수 레이어).
//
// ★기본값을 여기 두지 않는다★ — 정본은 셸 스키마 표(`src-tauri/src/settings/registry.rs`)이고, 값이 오기 전의 첫
//   페인트는 `theme.css` 의 `--chat-*` fallback 이 맡는다(TRD S21-storage §10 F6 (a) — 비동기).
// ★저장하지 않는다★ — 쓰기는 `settings_set`/`settings_reset` 뿐이고, 그 결과가 알림으로 돌아와 여기서 칠해진다.
//   localStorage 영속을 되살리지 말 것: 저장분이 셸 값과 갈려 두 출처가 된다.
// ★키 집합 대조가 없다(U2)★ — 셸 표의 11키와 `CSS_VAR_BY_KEY` 가 어긋나면 `settings.set chat.style.X` 가
//   `changed:true` 로 답하는데 화면은 그대로다(ADR-0265 「U2 의 대가」).
// ADR-0265

import {
  settingsClient,
  type ResetOutcome,
  type SetOutcome,
  type SettingsClient,
} from '../api/settingsClient'

/** 채팅 스타일 키(간격+폰트 세트). 값은 CSS 길이/숫자 문자열(예: '1rem', '13px', '1.55'). */
export type ChatStyleKey =
  | 'railRowPt' // rail 행 top-padding(행간 리듬)
  | 'plainRowPt' // 비-rail(user 버블·separator) 행 top-padding
  | 'userPy' // 유저 버블 세로 padding
  | 'userPx' // 유저 버블 가로 padding(userPy 와 대칭)
  | 'userMy' // 유저 버블 세로 margin(턴 덩어리 분리)
  | 'railGutter' // rail gutter 폭
  | 'railLineOffset' // 연결선 top 오프셋(위 행으로 이어짐 — railRowPt 와 커플링, 보통 음수)
  | 'railDotTop' // 점 마커 top(콘텐츠 첫 줄 근처)
  | 'fontSize' // 채팅 base font-size(chat.css 와 동기)
  | 'lineHeight' // 채팅 base line-height
  | 'waitStripH' // 입력창 위 대기 표시 줄 높이(사용자 결정 2026-10-01)

/** 설정 키 = 이 접두 + `ChatStyleKey`(셸 표의 마지막 마디가 이 철자다). */
const KEY_PREFIX = 'chat.style.'

// ADR-0051: StructuredTextView/theme.css/chat.css 가 이 변수들을 var() 로 읽는다.
const CSS_VAR_BY_KEY: Record<ChatStyleKey, string> = {
  railRowPt: '--chat-rail-row-pt',
  plainRowPt: '--chat-plain-row-pt',
  userPy: '--chat-user-py',
  userPx: '--chat-user-px',
  userMy: '--chat-user-my',
  railGutter: '--chat-rail-gutter',
  railLineOffset: '--chat-rail-line-offset',
  railDotTop: '--chat-rail-dot-top',
  fontSize: '--chat-font-size',
  lineHeight: '--chat-line-height',
  waitStripH: '--chat-wait-strip-h',
}

/** `chat.style.<k>` → 그 CSS 변수. 화이트리스트 밖이거나 챗 스타일 키가 아니면 `undefined`. */
function cssVarOf(settingKey: string): string | undefined {
  if (!settingKey.startsWith(KEY_PREFIX)) return undefined
  const key = settingKey.slice(KEY_PREFIX.length)
  // FIX-2: own key 로만 판정한다 — `key in CSS_VAR_BY_KEY` 는 프로토타입 체인을 타서 `chat.style.constructor` ·
  //   `chat.style.__proto__` 같은 키가 통과한다.
  if (!Object.prototype.hasOwnProperty.call(CSS_VAR_BY_KEY, key)) return undefined
  return CSS_VAR_BY_KEY[key as ChatStyleKey]
}

/** 설정 구독을 걸어 `chat.style.*` 을 :root 에 칠한다. 반환 = 구독 해제. document 가 없으면(SSR) 칠하지 않는다. */
export function installChatStyleApplier(client: SettingsClient = settingsClient): () => void {
  return client.subscribe(changes => {
    const root = globalThis.document?.documentElement
    if (!root) return
    for (const { key, value } of changes) {
      const cssVar = cssVarOf(key)
      if (cssVar) root.style.setProperty(cssVar, value)
    }
  })
}

/** 셸 설정에 쓴다 — 화면은 알림(또는 답)이 돌아온 뒤 바뀐다. 거절은 셸 오류 문자열로 reject. */
export function setChatStyle(
  key: ChatStyleKey,
  value: string,
  client: SettingsClient = settingsClient,
): Promise<SetOutcome> {
  return client.set(`${KEY_PREFIX}${key}`, value)
}

/** 한 키를, `key` 를 빼면 챗 스타일 전부를 기본값으로. */
export function resetChatStyle(
  key?: ChatStyleKey,
  client: SettingsClient = settingsClient,
): Promise<ResetOutcome> {
  return client.reset(key === undefined ? KEY_PREFIX : `${KEY_PREFIX}${key}`)
}
