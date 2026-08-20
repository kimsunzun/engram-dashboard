import { useRef, useEffect, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebglAddon } from '@xterm/addon-webgl'
import '@xterm/xterm/css/xterm.css'
import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputSubscription, ViewPhase } from '../../api/agentClient'
import { useAgentStore } from '../../store/agentStore'
import { SlotUnavailableVeil } from './SlotUnavailableVeil'

interface TerminalSlotProps {
  /** 구독 키(ADR-0046) = 슬롯 id. 같은 agentId 두 슬롯도 이 값으로 독립 구독·독립 진도(버그 B 해소). */
  viewId: string
  agentId: string | null
}

export default function TerminalSlot({ viewId, agentId }: TerminalSlotProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const fitAddonRef = useRef<FitAddon | null>(null)
  // ADR-0056: WebGL 좌석은 "보이는 슬롯"에만. 숨김 시 반납(loseContext+dispose)하고 이 ref 를 null 로
  //   비운다. Terminal 인스턴스(버퍼/스크롤백)는 계속 살아있고 WebGL addon 만 붙였다 뗀다.
  const webglAddonRef = useRef<WebglAddon | null>(null)
  // ADR-0056: 좌석 결정적 반납용으로 attach 시점에 잡아둔 GL 컨텍스트. 언마운트 경로 방어의 핵심 —
  //   releaseWebgl 을 containerRef.current 에 의존시키면 안 되기 때문이다(아래 releaseWebgl 주석 참조).
  const glRef = useRef<WebGLRenderingContext | null>(null)
  const agentIdRef = useRef<string | null>(agentId)
  // 입력 가드가 읽는 부재 값 — onData 는 xterm 콜백 안에서 돌아 리렌더로 갱신되는 state 를 못 보므로
  //   막을 올리는 그 값(agentUnavailable)을 ref 로 실어 나른다. 아래 effect 가 매 판정마다 갱신한다.
  const unavailableRef = useRef(false)
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // 구독이 마지막으로 알린 국면 — 아래 오버레이의 근거. 명부·연결 상태를 따로 유도하지 않고 구독이
  //   판정한 사실을 그대로 받는다(제어 표면은 하나 — ADR-0046 §5).
  const [phase, setPhase] = useState<ViewPhase | null>(null)

  useEffect(() => {
    agentIdRef.current = agentId
  }, [agentId])

  const agents = useAgentStore(s => s.agents)
  const agent = agentId ? (agents.find(a => a.id === agentId) ?? null) : null
  // ADR-0148: 권위 명부를 받았나 — 받기 전(기동·재연결 직후)의 빈 명부를 "없어졌다" 로 오인하지 않기 위한 가드.
  const agentsLoaded = useAgentStore(s => s.agentsLoaded)
  // ★부재 = terminal 상태로 발견 ∪ 명부 수신 후에도 해석 안 됨(ADR-0148)★. 후자가 종료(kill)의 실제 결말이다
  //   — reaper 가 세션을 수거하며 명부에서 지우므로 status 로는 잡히지 않는다. 이걸 부재로 안 보면 죽은
  //   에이전트가 막도 입력 가드도 없는 슬롯으로 남는다(아래 agentUnavailable 의 한 항).
  const agentGone =
    (agentsLoaded && agentId != null && agent == null) ||
    (agent != null &&
      (agent.status.type === 'Exited' ||
        agent.status.type === 'Killed' ||
        agent.status.type === 'Failed'))

  // ADR-0148 결정 1 / ADR-0149 결정 4: 부재 판정은 세 슬롯이 같다(RichSlot 이 정본 형태) — 종료 ∪ 연결
  //   끊김 ∪ 구독이 출력을 못 내는 상태. 기존 연결 상태 표면(agentClient)을 **읽기만** 한다. 구독 콜백은
  //   등록 즉시 현재 상태로 1회 발화하므로 초기값 동기화가 따로 필요 없다.
  const [connected, setConnected] = useState(() => agentClient.connectionState === 'connected')
  useEffect(() => agentClient.onConnectionStateChange(s => setConnected(s === 'connected')), [])
  // ★'error' 도 부재다★: 재요청 사다리를 소진한 뷰는 아무것도 못 받는데, 이걸 빼면 명부에 살아 있고
  //   연결도 붙은 슬롯이 **아무 표시 없는 빈 판**으로 남는다(신호도 회복 안내도 없다).
  const subscriptionDown = phase === 'detached' || phase === 'error'
  const agentUnavailable = agentGone || !connected || subscriptionDown

  // ★입력을 막는 조건 = 막을 올리는 조건★: 막은 pointer-events-none 이라 아래 조작을 가리지 않는다
  //   (SlotUnavailableVeil) — 포커스가 남은 xterm 은 막이 떠 있어도 계속 키를 먹는다. 종료만 보고 막으면
  //   연결이 끊긴·구독이 죽은 슬롯에서 "없다" 고 적힌 화면 뒤로 입력이 계속 나가고, 그 실패는 조용히
  //   삼켜져 사용자에겐 무반응으로만 보인다.
  useEffect(() => { unavailableRef.current = agentUnavailable }, [agentUnavailable])

  useEffect(() => {
    if (!containerRef.current) return
    // WebGL/canvas 렌더러는 글리프를 canvas 2D(ctx.font)로 rasterize하는데 canvas 는 CSS var() 를
    // 해석 못 한다 → 'var(--font-terminal)' 을 그대로 넘기면 폰트 무효로 검은 화면. 생성 시점에
    // 실제 폰트 문자열로 해석해 넘긴다. (실측: canvas 가 '13px var(--font-terminal)' 거부→10px sans-serif)
    const fontFamily =
      getComputedStyle(document.documentElement).getPropertyValue('--font-terminal').trim() || 'monospace'
    const term = new Terminal({
      fontFamily,
      fontSize: 13,
      theme: { background: '#0a0a0a', foreground: '#e0e0e0', cursor: '#4a9eff' },
    })
    const fitAddon = new FitAddon()
    term.loadAddon(fitAddon)
    term.open(containerRef.current)
    // ADR-0056: WebGL 렌더러는 여기(마운트)서 안 붙인다 — 가시성 연동 effect 로 옮겼다. keep-alive 로
    //   숨은 탭도 Terminal 인스턴스는 살아있지만, WebGL 좌석(브라우저 하드 상한 16)은 보이는 슬롯에만
    //   준다.
    fitAddon.fit()
    terminalRef.current = term
    fitAddonRef.current = fitAddon

    const ro = new ResizeObserver(entries => {
      // ADR-0056: 숨김(탭 keep-alive = display:none) 이면 컨테이너가 0 크기로 붕괴하며 RO 가 발화한다.
      //   이때 fit() 하면 0 크기 위에서 cols/rows 가 최소값/쓰레기로 계산되고, resizePty 로 그 치수가
      //   PTY 에 전파돼 에이전트 레이아웃이 깨진다. hidden 신호로 조기 반환하고 대기 중 타이머도 지운다.
      //   신호는 offsetParent===null — display:none 이면(자신·조상 어느 쪽이든) null 이라 붕괴한 contentRect
      //   0 폭·높이보다 견고하다(1px·overflow 잔여 크기 오탐 회피).
      const hidden =
        containerRef.current?.offsetParent === null ||
        (entries[0]?.contentRect.width === 0 && entries[0]?.contentRect.height === 0)
      if (hidden) {
        if (resizeTimerRef.current) {
          clearTimeout(resizeTimerRef.current)
          resizeTimerRef.current = null
        }
        return
      }
      fitAddon.fit()
      const aid = agentIdRef.current
      if (!aid) return
      // debounce 50ms — 드래그 중 매 프레임 IPC 발사 방지
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current)
      resizeTimerRef.current = setTimeout(() => {
        resizeTimerRef.current = null
        // 거절 싱크 — 형제 호출부와 동형. 죽은/끊긴 슬롯이 마운트된 채 RO 가 살아 있으므로(ADR-0148 뷰
        //   유지) 분할선을 끌면 여기로 실패가 온다. Resize 는 fire-and-forget 이라 흡수만 한다.
        void agentClient.resizePty(aid, term.cols, term.rows).catch(() => {})
      }, 50)
    })
    ro.observe(containerRef.current)

    return () => {
      ro.disconnect()
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current)
      // ADR-0056: Terminal dispose 만으론 GC 전까지 GPU 좌석이 안 풀린다(아래 releaseWebgl 주석 참조).
      releaseWebgl()
      term.dispose()
      terminalRef.current = null
      fitAddonRef.current = null
    }
  }, [])

  // ADR-0056 ── WebGL 좌석 결정적 반납 ────────────────────────────────────────────────
  // ★load-bearing 불변식★: xterm 의 `WebglAddon.dispose()` 는 canvas/layer 를 DOM 에서 떼기만 하고
  //   `WEBGL_lose_context.loseContext()` 를 호출하지 않는다(소스 확인: WebglRenderer 에 loseContext 없음).
  //   즉 dispose 만 하면 GPU 컨텍스트 좌석은 GC 될 때까지(비결정적) 계속 점유된다 → 브라우저 하드 상한
  //   16 을 넘기면 오래된 컨텍스트가 소실(참조: PixiJS #8215 — destroy 후에도 GC 전까지 경고 지속).
  //   그래서 dispose *이전에* 우리가 직접 loseContext() 를 불러 좌석을 결정적으로 놓는다.
  // ★왜 attach 시점에 컨텍스트를 잡아두나(glRef)★: React 는 host ref(containerRef) 를 commit 의 mutation
  //   단계에서 null 로 비운다 — 이건 passive effect cleanup(언마운트 시 releaseWebgl 이 도는 지점) *이전*이다.
  //   그래서 언마운트 경로에서 releaseWebgl 이 containerRef.current 를 훑으면 이미 null → canvas 루프가
  //   통째로 스킵되고 loseContext 가 안 불려 좌석이 GC 까지 샌다(탭/슬롯 닫기 = 정확히 반납이 필요한 경로).
  //   그래서 컨테이너가 살아있고 보이는 attach 시점(attachWebgl, loadAddon 성공 직후)에 GL 컨텍스트를
  //   glRef 에 잡아두고, 여기선 그 잡아둔 컨텍스트로 loseContext 를 부른다 — ref-null·DOM 제거를 견딘다.
  //   (detach 된 canvas 의 컨텍스트라도 lose/dispose 전까지는 유효해 getExtension 호출이 가능하다. 방어차
  //   전 구간 옵셔널 체이닝 + try-catch — 컨텍스트를 못 얻어도 절대 throw 하지 않는다.)
  const releaseWebgl = () => {
    const addon = webglAddonRef.current
    if (!addon) return
    try {
      glRef.current?.getExtension('WEBGL_lose_context')?.loseContext()
    } catch {
      // loseContext 실패는 치명적 아님 — dispose 는 계속 진행(좌석은 GC 로라도 결국 풀림).
    }
    addon.dispose()
    webglAddonRef.current = null
    glRef.current = null
  }

  // ADR-0056 ── 가시성 연동 WebGL 라이프사이클 ──────────────────────────────────────────
  // 보이는 슬롯만 WebGL 좌석을 쥔다. keep-alive(탭 숨김=display:none, WindowLayout) 하에서 IO 는 숨은
  //   타겟을 not-intersecting 으로 보고하고, 전환 시 콜백을 발화한다. ★초기-숨김 주의★: 숨은 탭에서
  //   마운트되면 첫 observe() 콜백이 이미 not-intersecting 이므로, 첫 콜백이 보인다고 가정하면 안 된다 —
  //   숨은 채 마운트된 슬롯은 첫 "보임" 콜백 전까지 WebGL 을 만들지 않는다(좌석 절약).
  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const attachWebgl = () => {
      const term = terminalRef.current
      if (webglAddonRef.current || !term) return
      // WebGL 렌더러 — DOM 렌더러는 customGlyphs 미지원이라 블록/박스드로잉 문자를 폰트에 위임,
      // 분수 DPI(rowHeight 비정수)에서 첫 행 상단 픽셀이 깎인다. WebGL은 이 글리프를 직접 그려
      // 클리핑 제거(조사: xterm.js #2409/#3807/#967). 미지원/컨텍스트 소실 시 DOM 자동 폴백.
      // ★부분생성 누수 방어★: new WebglAddon() 이 이미 GL 컨텍스트를 잡은 뒤 loadAddon 등에서 throw 하면,
      //   catch 에서 이 addon 을 dispose 하지 않으면 좌석이 새고 다음 "보임"에서 두 번째 addon 이 붙는다.
      //   그래서 addon 참조를 try 밖에 두고 catch 에서 dispose 한다. ref 는 완전 성공 시에만 세팅.
      let webgl: WebglAddon | undefined
      try {
        webgl = new WebglAddon()
        // 컨텍스트 소실(좌석 축출 등) 시 반응형 DOM 폴백. addon 을 버리고 ref 를 비워
        //   다음 "보임" 때 재부착 가능하게 둔다.
        webgl.onContextLoss(() => {
          webgl?.dispose()
          if (webglAddonRef.current === webgl) {
            webglAddonRef.current = null
            glRef.current = null
          }
        })
        term.loadAddon(webgl)
        webglAddonRef.current = webgl
        // ADR-0056: 결정적 반납용 GL 컨텍스트를 지금(컨테이너 live·visible) 잡아둔다. 언마운트 시 releaseWebgl
        //   이 containerRef 없이도 loseContext 를 부를 수 있게 — 이유는 releaseWebgl 주석 참조. addon 이
        //   container 에 append 한 canvas 중 WebGL(2) 컨텍스트가 잡히는 것을 찾는다(layer class 이름 비의존
        //   버전-견고 경로; 2d atlas canvas 는 getContext('webgl2'|'webgl') 이 null 이라 자연히 건너뜀).
        const container = containerRef.current
        if (container) {
          for (const canvas of container.querySelectorAll('canvas')) {
            const gl =
              (canvas.getContext('webgl2') as WebGLRenderingContext | null) ??
              (canvas.getContext('webgl') as WebGLRenderingContext | null)
            if (gl) {
              glRef.current = gl
              break
            }
          }
        }
        // 새로 붙은 GPU 렌더러가 현재 버퍼를 즉시 그리도록: 보이는 상태이므로 fit()→PTY 크기 전파→refresh.
        //   (숨김 중엔 fit() 금지 — 측정 불가라 쓰레기 치수가 나온다.)
        fitAddonRef.current?.fit()
        const aid = agentIdRef.current
        if (aid) void agentClient.resizePty(aid, term.cols, term.rows).catch(() => {})
        // rows===0(측정 전 등)이면 refresh(0, -1) 이 되어 무의미/오작동 → rows>0 일 때만 refresh.
        if (term.rows > 0) term.refresh(0, term.rows - 1)
      } catch (e) {
        // WebGL 미지원/로드 실패 → DOM 렌더러 폴백. 부분생성된 addon 은 반드시 dispose(좌석 누수·중복부착
        //   방지). 무로깅이면 클리핑 픽스가 조용히 무효화돼도 모르니 경고.
        webgl?.dispose()
        console.warn('[TerminalSlot] WebGL 비활성 → DOM 렌더러로 폴백', e)
      }
    }

    const io = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const visible = entry.isIntersecting && entry.intersectionRatio > 0
        if (visible) attachWebgl()
        else releaseWebgl() // DOM 렌더러가 자동 인수(어차피 안 보여 그리지 않음).
      }
    })
    io.observe(container)

    return () => io.disconnect()
  }, [])

  useEffect(() => {
    const terminal = terminalRef.current
    if (!agentId || !terminal) return

    let sub: OutputSubscription | null = null
    let cancelled = false

    terminal.reset() // C2: StrictMode 중복 방지
    setPhase(null) // 새 구독의 국면은 그 구독의 통지가 다시 세운다.
    const lastSeq = { current: -1 } // T-2/G-2: seq dedup(컴포넌트 방어 — 클라도 내부 dedup)

    agentClient
      .subscribeOutput(
        viewId,
        agentId,
        chunk => {
          if (cancelled) return
          if (chunk.seq <= lastSeq.current) return
          lastSeq.current = chunk.seq
          // ★tag 게이트(S15/ADR-0045)★: 이 슬롯은 터미널 raw 바이트(tag0)만 xterm 에 write 한다. tag1
          //   (StructuredEvent JSON)이 오면 무시한다 — RichSlot 이 tag0 을 무시하는 것과 정확히 대칭.
          //   구조화 에이전트에 터미널 슬롯이 붙거나(renderModeOverride·다중 구독) 배선 버그로 tag1 이
          //   공유 스트림(한 seq 공간)으로 새면, 게이트가 없을 때 JSON 바이트가 그대로 xterm 에 찍혀
          //   화면이 오염된다. seq 는 위에서 이미 전진시켰으므로(tag 무관 한 seq 공간) tag1 을 건너뛰어도
          //   dedup 은 정합하다.
          if (chunk.tag !== FRAME_TAG_TERMINAL_BYTES) return
          terminal.write(chunk.bytes) // 디코드는 클라 내부에서 끝남(transport 캡슐화)
        },
        // 국면으로는 화면을 건드리지 않는다 — 'buffering' 에는 같은 화신 이어보기가 섞여 있어 거기서
        //   비우면 잠깐 끊겼다 온 슬롯이 매번 깜빡인다. 값은 부재 막 판정에만 쓴다.
        state => {
          if (cancelled) return
          setPhase(state)
        },
        // ★비우기 콜백은 세 슬롯 모두 반드시 넘긴다(정본 진술 — DomSlot·RichSlot 은 이 자리를 미러)★:
        //   라우터는 이 콜백을 부른 직후 같은 틱에 다른 화신의 전량 replay 를 배달하며 진도 커서를 버린다.
        //   안 넘기면 커서만 0 으로 돌아가고 화면은 앞 화신 내용을 든 채라, 전량이 그 위에 겹쳐 그려지고
        //   오류는 어디에도 남지 않는다. 화면과 이 로컬 가드를 **함께** 비우는 이유도 같다 — 가드만
        //   되돌리면 겹쳐 쌓이고, 화면만 지우면 0 부터 다시 매겨진 프레임이 전부 여기서 탈락해 빈 채 남는다.
        () => {
          if (cancelled) return
          terminal.reset()
          lastSeq.current = -1
        },
      )
      .then(handle => {
        if (cancelled) {
          handle.unsubscribe()
          return
        }
        sub = handle
        // Task1(ADR-0036 carry-forward): 구독 직후 초기 크기 1회 전파. ResizeObserver 는 크기
        // *변화* 시에만 발화하므로, 슬롯이 처음부터 최종 크기면 한 번도 안 울려 PTY 가 spawn 시
        // 기본값(80×24)에 고착된다 → claude welcome 박스가 80칸 기준으로 그려져 좁은 슬롯에서 깨짐.
        // 그 빈 "초기 1회"를 여기서 채운다(gotty 패턴; client-first(ttyd)는 데몬이 View 를 모르는
        // ADR-0035 구조라 불가). 보내기 직전 fit() 으로 allotment 지연 레이아웃까지 반영한 최신
        // cols/rows 를 보장한다. resizePty 는 fire-and-forget(Resize 는 request_id 없음) — 직전
        // kill 등으로 실패해도 흡수. carrier 는 Phase B(TauriTransport)에서도 이 call-site 그대로.
        fitAddonRef.current?.fit()
        void agentClient.resizePty(agentId, terminal.cols, terminal.rows).catch(() => {})
      })
      // 구독 실패(예: 직전 kill로 NotFound)는 unhandled rejection 방지용으로 흡수.
      .catch(() => {})

    return () => {
      cancelled = true
      // unsubscribe 내부가 transport 정리(#13133 delete onmessage) + 백엔드 구독 해제까지 수행.
      sub?.unsubscribe()
    }
    // ★화신(epoch)은 이 트리거에 넣지 않는다★: 화면을 비우는 일은 onReset 단독이 진다. 여기에 화신을
    //   섞으면 재spawn·데몬 재기동처럼 값이 흔들리는 구간마다 구독이 통째로 갈리면서 **replay 가 오기도
    //   전에** 이 위의 terminal.reset() 이 돌아 화면이 지워지고(데몬 ring 도 이미 없어 복구 불가),
    //   그 교체가 화신 표식까지 잃어버려 정작 회전 판정(protocolClient.observeRoster)이 못 선다.
    // viewId 포함 — 구독 키(ADR-0046)라 바뀌면 재구독(실무상 key=viewId 라 slot 교체는 remount).
  }, [viewId, agentId])

  useEffect(() => {
    const terminal = terminalRef.current
    if (!agentId || !terminal) return
    const disp = terminal.onData(data => {
      if (unavailableRef.current) return
      void agentClient.writeStdin(agentId, new TextEncoder().encode(data)).catch(() => {})
    })
    return () => disp.dispose()
  }, [agentId])

  return (
    <div style={{
      width: '100%',
      height: '100%',
      position: 'relative',
      boxSizing: 'border-box',
      padding: '4px 8px',        // 터미널 좌우 여백(wezterm 스타일). 여백만큼 inset → FitAddon이 그 크기로 cols/rows 계산.
      background: '#0a0a0a',     // 터미널 배경(Terminal theme)과 동일 → 여백이 seamless
    }}>
      <div ref={containerRef} style={{ width: '100%', height: '100%' }} />
      {agentUnavailable && <SlotUnavailableVeil phase={phase} />}
    </div>
  )
}
