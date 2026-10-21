// 이 웹뷰의 첫 화면을 셸에 알린다(`webview_painted`) — 셸은 main · 팝아웃 창을 숨긴 채 만들고 이 알림에 창을 보인다
// (`src-tauri/src/first_paint.rs` — 알림이 오지 않으면 시한 뒤). 셸이 기다리지 않는 창(다시 읽기 · 예비 창 · `--hidden`
// 부팅)의 알림은 셸이 무시한다. 부르는 쪽은 탭 창의 `WindowLayout` 이다 — 탭을 받기 전의 바탕만 있는 화면이 아니라 첫
// 화면(탭바 + 활성 탭)이 커밋된 뒤에 부른다.
// 상태가 모듈 범위인 이유: 알림은 페이지 읽기 하나에 한 번이고, 다시 읽기가 모듈을 새로 실행하며 함께 초기화된다 —
//   StrictMode 의 effect 두 번 실행에도 한 번만 보낸다.

import { invoke } from '@tauri-apps/api/core'

// 보이는 페이지에서 프레임 둘을 기다리다 프레임이 오지 않을 때 보내는 상한 — 페이지가 보인다고 답하면서 프레임 콜백이
//   돌지 않는 경우(숨긴 창 안 페이지의 가시성 · 프레임이 어떤지 [미검])에 셸 시한(1500 ms)까지 끌려가지 않게. 숨은 페이지의
//   타이머는 조여질 수 있어 이 값보다 늦을 수 있다.
const FRAME_WAIT_CAP_MS = 100

let reported = false

/**
 * 보일 화면이 커밋된 뒤(effect)에 부른다. 페이지가 보이는 중이면 프레임 둘 뒤(그 화면이 그려진 뒤 — 프레임이 오지
 * 않으면 `FRAME_WAIT_CAP_MS` 뒤)에, 숨은 중이면 곧바로 보낸다 — 숨긴 창 안 페이지는 숨은 상태로 보여 프레임 콜백이 돌지
 * 않을 수 있고[미검 — WebView2 가 부모 창 숨김을 페이지 가시성에 싣는지], 그때는 DOM 이 커밋된 지금 보내 보인 뒤의 첫
 * 프레임이 그 DOM 을 그리게 한다.
 * 실패(셸 밖 · 명령 없음)는 경고로만 남긴다 — 셸은 시한 뒤 어차피 보인다.
 */
export function reportFirstPaint(): void {
  if (reported) return
  reported = true
  let sent = false
  const send = () => {
    if (sent) return
    sent = true
    invoke<void>('webview_painted').catch(err => console.warn('[firstPaint] webview_painted 실패:', err))
  }
  if (document.visibilityState === 'visible') {
    requestAnimationFrame(() => requestAnimationFrame(send))
    setTimeout(send, FRAME_WAIT_CAP_MS)
  } else {
    send()
  }
}

/** 테스트 전용 — 모듈 상태 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetFirstPaintForTest(): void {
  reported = false
}
