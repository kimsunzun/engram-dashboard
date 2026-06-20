// AgentClient 팩토리 — 위치 모드(embedded/daemon) 선택 지점(ADR-0020 Stage 3).
// 부팅 시 1회 모드를 고른다 — 라이브 핫스왑 안 함.
//
// ★모드의 source of truth = Rust(ADR-0027 보강 요구1)★: Rust 가 부팅 모드를 결정해 페이지 로드 전
// window.__ENGRAM_MODE__ 로 주입한다(lib.rs engram-mode 플러그인 js_init_script, 모드 분기 밖에서 무조건).
// 따라서 프론트는 이 전역을 최우선으로 읽는다. localStorage 'engram_client_mode' 는 **Tauri 밖(vitest/
// 브라우저 프리뷰) fallback** 일 뿐이다 — Tauri WebView 안에선 dev/release 무관하게 주입이 항상 있어
// 이 경로는 도달하지 않는다("dev override" 아님). dev 모드 스위칭은 localStorage 가 아니라 --mode 인자나
// ENGRAM_MODE env 로 한다(Rust resolve_mode). 기본 'embedded'(둘 다 없을 때 — 순수 테스트 등).
//
// ★Stage 3 전환★: mode → transport(InProc/Ws) 선택 → new ProtocolClient(transport). 단일
// ProtocolClient(프로토콜 의미론 1벌) + carrier 2개. (Stage 4a: 옛 EmbeddedClient/DaemonClient/
// ptyApi·옛 Tauri command 삭제 완료 — 새 경로만 남음.)

import type { AgentClient } from './agentClient'
import {
  type DaemonControl,
  DaemonDaemonControl,
  EmbeddedDaemonControl,
} from './daemonControl'
import { InProcTransport } from './inProcTransport'
import { ProtocolClient } from './protocolClient'
import type { Transport } from './transport'
import { WsTransport } from './wsTransport'

type ClientMode = 'embedded' | 'daemon'

/**
 * 부팅 시 1회 모드 결정. Rust 가 주입한 전역(__ENGRAM_MODE__)이 source of truth 라 최우선(ADR-0027).
 * 주입이 없을 때만 localStorage(Tauri 밖 fallback) → 그래도 없으면 embedded(기본).
 */
function resolveMode(): ClientMode {
  // Rust 주입(source of truth) — 실배포에선 항상 존재(js_init_script, 페이지 로드 전).
  const fromGlobal = (window as unknown as { __ENGRAM_MODE__?: string }).__ENGRAM_MODE__
  if (fromGlobal === 'daemon' || fromGlobal === 'embedded') return fromGlobal
  try {
    const stored = window.localStorage.getItem('engram_client_mode')
    if (stored === 'daemon' || stored === 'embedded') return stored
  } catch {
    // localStorage 접근 불가(예: 일부 컨텍스트) — 기본값 fallback.
  }
  return 'embedded'
}

let instance: AgentClient | null = null
let daemonControlInstance: DaemonControl | null = null
let resolvedMode: ClientMode | null = null

/** 단일 AgentClient 인스턴스. 컴포넌트·스토어·(미래)LLM 이 모두 이걸 통한다. */
export function getAgentClient(): AgentClient {
  if (!instance) {
    const mode = resolveMode()
    resolvedMode = mode
    // mode → carrier 선택. 프로토콜 의미론은 ProtocolClient 한 곳(carrier 무관).
    const transport: Transport = mode === 'daemon' ? new WsTransport() : new InProcTransport()
    instance = new ProtocolClient(transport)
    // ADR-0021 §5: 데몬 lifecycle 제어 표면. daemon 모드만 실제 동작, embedded 는 no-op/에러.
    daemonControlInstance =
      mode === 'daemon' ? new DaemonDaemonControl(instance) : new EmbeddedDaemonControl()
    // §5 LLM-우선 제어: 제어 표면을 window 에 노출 — cdp.mjs eval / (미래) 백엔드측 LLM 이
    // 사람 클릭과 동일 진입점을 호출할 수 있게 한다(임시 경로, 정식 command 버스 전까지).
    ;(window as unknown as { __ENGRAM_AGENT__?: AgentClient }).__ENGRAM_AGENT__ = instance
    // 데몬 제어(start/stop/status)도 동일하게 노출 — 트레이(#2)·LLM·cdp 가 같은 핸들을 흔든다.
    ;(window as unknown as { __ENGRAM_DAEMON__?: DaemonControl }).__ENGRAM_DAEMON__ =
      daemonControlInstance
  }
  return instance
}

/** 단일 DaemonControl 인스턴스. getAgentClient 와 동일 시점에 구성된다(mode 1회 결정). */
export function getDaemonControl(): DaemonControl {
  if (!daemonControlInstance) getAgentClient() // 동시 초기화 보장.
  return daemonControlInstance!
}

/** 결정된 클라이언트 모드(부팅 1회). 부팅 ensure 가 daemon 모드만 명시 start 하도록 분기에 쓴다. */
export function getClientMode(): ClientMode {
  if (!resolvedMode) getAgentClient()
  return resolvedMode!
}

/**
 * 부팅 시 **명시 ensure 1회**(ADR-0021 §1: spawn=명시 시점만). daemon 모드일 때만 daemonControl.start()
 * 를 호출해 데몬을 띄운다(tmux attach 가 서버를 띄우는 것과 동치). 명령 경로(ensureReady)는 attach-only
 * 라 데몬을 못 깨우므로, 이 부팅 start 가 없으면 부팅 시 데몬이 안 뜬다. embedded 는 InProc 이 첫 명령
 * 시 Channel 을 등록하므로 no-op(EmbeddedDaemonControl.start 는 에러라 호출하지 않는다).
 * 멱등 — start 는 이미 connected 면 즉시 resolve. 실패(데몬 spawn 불가)는 삼켜 부팅을 막지 않는다.
 */
export async function bootstrapDaemonIfNeeded(): Promise<void> {
  if (getClientMode() !== 'daemon') return
  try {
    await getDaemonControl().start()
  } catch (err) {
    console.warn('[clientFactory] 부팅 daemon start 실패(수동 daemon_start 필요):', err)
  }
}

/** 모듈 로드 시점 싱글톤(대부분 컴포넌트는 이걸 import). */
export const agentClient = getAgentClient()
