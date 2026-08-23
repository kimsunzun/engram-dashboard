// 디스크의 UI 설정(`<data_dir>/ui-settings.json`)을 화면에 붙이는 배선 — 오늘 나르는 것은 테마 한 칸이다.
//
// ★값만 바꾼다★: 받은 테마를 themeManager 로 흘려 `data-theme` 하나를 갈아끼운다. 리마운트·키 교체·라우터
// 이동을 이 경로에 얹지 말 것 — 챗은 컴포넌트 상태라 슬롯이 다시 마운트되면 대화가 영구 소실된다(ADR-0149).
//
// ★invoke/listen 을 직접 부른다★: 「컴포넌트·스토어는 agentClient 에만 의존한다」의 예외로, 백엔드가 권위인
// 표면은 Tauri 를 직접 쓴다(레이아웃이 `viewStore` 에서 쓰는 그 예외 — 이 값의 주인은 데몬이 아니라 셸이라
// agentClient 가 나를 것이 아니다).
//
// ★쓰기 경로가 없다★: 화면의 테마 토글(`commands/themeCommands.ts`)은 인메모리로 남고 다음 `ui.refresh` 가
// 그것을 덮는다. 파일을 고치는 것은 밖의 에이전트다(사유 = `src-tauri/src/ui_settings.rs` 헤더).
// ADR-0149

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { type ThemeName } from '../store/themeStore'
import { retryAsync, RetryCancelledError } from '../util/retryInvoke'
import { themeManager } from './ThemeManager'

// 셸이 미는 알림. ★창마다 자기 값이 온다★ — 셸이 목적지를 지목해 보낸다(`src-tauri/src/commands/settings.rs`).
const EVT_UI_SETTINGS_UPDATED = 'ui:settings-updated'
// 부팅 조회. 미는 쪽과 같은 모양(`{theme, source}`)을 돌려준다 — 셸이 두 자리에 같은 페이로드 struct 를
// 쓴다. ★여기서 읽는 것은 `theme` 뿐이다★ — `source`(파일에서 왔나 / 기본값으로 접혔나)는 명령 답장을 보는
// 호출자 몫이고 화면은 값만 쓴다.
// ★창 인자를 안 넘긴다★ — 어느 창이 물었는지는 Tauri 가 셸 쪽에 넣어 준다(웹뷰가 밝히면 잘못 적힌 label
// 하나가 남의 창 테마를 가져간다).
const CMD_GET_UI_SETTINGS = 'get_ui_settings'

const THEMES: readonly ThemeName[] = ['dark', 'light', 'e-ink']
const FALLBACK: ThemeName = 'dark'

// ★셸이 이미 세 값 중 하나로 접어서 준다 — 이 검문은 그 계약이 깨졌을 때 화면이 무테마로 남지 않게 하는
//   그물이다★. 여기서 조용히 통과시키면 `data-theme` 에 없는 이름이 박혀 스타일이 통째로 안 붙는다.
function themeOf(payload: unknown, where: string): ThemeName {
  const raw = (payload as { theme?: unknown } | null | undefined)?.theme
  if (typeof raw === 'string' && (THEMES as readonly string[]).includes(raw)) {
    return raw as ThemeName
  }
  console.warn(`[uiSettings] ${where}: 쓸 수 없는 테마 ${JSON.stringify(raw)} — ${FALLBACK} 로 둔다`)
  return FALLBACK
}

/**
 * 부팅 조회 1회 + 갱신 구독을 건다. 반환값은 disposer(언마운트/HMR 시 리스너 중복 누적 방지).
 *
 * 창마다 한 번 부른다 — main·팝아웃·트리가 각자 자기 값을 당긴다.
 */
export function installUiSettings(): () => void {
  let disposed = false
  let unlisten: UnlistenFn | undefined
  // ★부팅 조회가 푸시를 덮지 못하게 하는 빗장★: 조회 왕복 중에 `ui.refresh` 가 들어오면 조회 답이 더 낡은
  //   값이다. 도착 순서로 이기게 두면 그 refresh 는 답만 성공이고 화면은 안 바뀐다 — 사유 없는 무반응이라
  //   호출자가 파일을 의심하며 헤맨다.
  let pushed = false

  void (async () => {
    // ★구독 등록이 끝난 **뒤에** 부팅 조회를 낸다 — 둘을 나란히 띄우지 말 것★.
    //   나란히 띄우면 등록이 아직 안 끝난 사이에 온 `ui.refresh` 알림은 이 창에 **도착조차 안 하고**
    //   (리스너가 없다) 그 뒤 늦게 온 조회 답이 옛 값을 칠한다. 위 `pushed` 빗장은 알림을 **받았을 때만**
    //   서므로 그 인터리브를 못 막는다 — 순서로만 막힌다.
    //   순서를 뒤집으면 잃는 것은 없다: 등록 전에 놓친 알림이 있어도 조회가 그 **뒤에** 파일을 읽으므로
    //   같은(또는 더 새) 값을 가져온다.
    // ★한 번 거절당하면 그 창은 영구히 귀머거리가 된다 — 그래서 유계 재시도를 건다★.
    //   조회는 성공하는데 구독만 실패하면 첫 값은 멀쩡히 칠해져 **화면이 건강해 보인다**. 그 뒤 모든
    //   `ui.refresh` 를 그 창만 놓쳐 다른 창과 영영 어긋나는데 아무 신호가 없다. 이벤트 평면 미준비는
    //   부팅 직후의 일시 실패라 재시도로 낫는 종류다(같은 판단으로 `App.tsx` 의 부팅 pull 이 이 헬퍼를
    //   쓴다 — ADR-0102).
    //   재시도까지 소진되면 **조용히 넘기지 않고 error 로 표면화한다** — 그 뒤로는 이 창이 갱신을 못 받는
    //   것이 확정이라 warn 이 아니다.
    // ★재시도가 걸리는 것은 **구독뿐**이다★ — 아래 부팅 조회는 맨 `invoke` 다. 한 번 실패하면 그대로
    //   기본 테마로 가고, 다음 `ui.refresh` 가 값을 맞춘다(구독은 그때 이미 서 있다).
    // ★이 재시도가 덮는 것은 **거절**뿐이다 — 무응답(hang)은 못 덮는다★(알려진 잔여). 타임아웃이 없어
    //   `listen()` 이 영영 안 풀리면 재시도 루프가 첫 시도에서 멈춰 있고, 그 아래 부팅 조회까지 함께 막힌다
    //   (순서 종속 — 바로 위 항목). 「재시도가 있으니 등록은 결국 된다」로 읽지 말 것.
    try {
      // ★★`target` 을 빼지 말 것 — 그러면 이 창이 **남의 창 값까지** 받아 마지막 것을 칠한다★★
      //   `listen()` 의 기본 타깃은 `Any` 이고, Tauri 는 `Any` 로 등록된 리스너를 **필터와 무관하게 전부**
      //   깨운다(`match_any_or_filter`). 셸은 창마다 `emit_to(label, …)` 로 자기 값을 보내므로, 기본
      //   등록이면 창 셋짜리 refresh 한 번에 이 창이 값 셋을 받고 **먼저 온 것들이 무의미해진다**.
      //   ★label 은 Tauri 가 준다★ — 셸도 같은 값을 쓴다(`commands/settings.rs` 의 `Window::label()`).
      const off = await retryAsync(
        () =>
          listen<unknown>(
            EVT_UI_SETTINGS_UPDATED,
            e => {
              // ★정리된 뒤에 온 것은 버린다★: dispose 가 `listen()` 대기 중에 돌면, 등록은 이미 끝났는데
              //   아래 `off()` 에는 아직 못 닿은 창이 생긴다. 그 틈에 배달되면 **죽은 인스턴스**
              //   (StrictMode 첫 회 · HMR 이전 판)가 살아 있는 창의 테마를 덮는다. `off()` 만으로는
              //   그 한 건을 못 막는다.
              if (disposed) return
              pushed = true
              themeManager.apply(themeOf(e.payload, EVT_UI_SETTINGS_UPDATED))
            },
            { target: getCurrentWindow().label },
          ),
        {
          isCancelled: () => disposed,
          onRetry: (err, attempt) => {
            console.warn(`[uiSettings] ${EVT_UI_SETTINGS_UPDATED} 구독 재시도 #${attempt}:`, err)
          },
        },
      )
      // 등록이 끝나기 전에 dispose 가 돌면 핸들을 아직 못 받아 unlisten 을 못 건다 — 받은 자리에서 바로 푼다
      // (viewStore 의 같은 조항).
      if (disposed) {
        off()
        return
      }
      unlisten = off
    } catch (e) {
      // 구독이 죽어도 부팅 조회는 낸다 — 갱신은 못 받아도 첫 값은 맞아야 한다.
      if (!(e instanceof RetryCancelledError)) {
        console.error(
          `[uiSettings] ${EVT_UI_SETTINGS_UPDATED} 구독 실패 — 이 창은 이후 ui.refresh 를 못 받는다:`,
          e,
        )
      }
    }

    let theme: ThemeName = FALLBACK
    try {
      theme = themeOf(await invoke<unknown>(CMD_GET_UI_SETTINGS), CMD_GET_UI_SETTINGS)
    } catch (e) {
      console.warn(`[uiSettings] ${CMD_GET_UI_SETTINGS} 실패 — ${FALLBACK} 로 간다:`, e)
    }
    if (disposed || pushed) return
    themeManager.apply(theme)
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = undefined
  }
}
