// AgentClient — 프론트가 의존하는 단일 제어 표면(S12, daemon-design §3-a).
//
// 컴포넌트·스토어는 invoke/Channel/WS 를 직접 부르지 않고 이 인터페이스만 의존한다.
// 단일 구현 ProtocolClient(프로토콜 의미론 1벌) + carrier(TauriTransport, ADR-0020) —
// embedded/InProc 표면은 제거됐다(ADR-0029: daemon-only).
// transport(Tauri Channel / WS binary frame)와 base64/디코딩은 transport 내부에 숨긴다 —
// 인터페이스는 "디코드된 바이트 청크"만 노출(§3-a 손발/두뇌 분리: 프론트=순수 I/O).

import type {
  AgentInfo,
  AgentProfile,
  AgentStatus,
  ClaudeOutputFormat,
  Preset,
  RestoreReport,
} from './types'

export type ConnectionState = 'connected' | 'reconnecting' | 'down'

/** 디코드된 출력 청크 — transport 무관(base64/binary frame 은 클라 내부에서 이미 풀림). */
export interface OutputChunk {
  /** core OutputCore 발급 seq(단조 증가). */
  seq: number
  /**
   * frame 종류(wsFrame.ts): 0=터미널 raw 바이트(xterm write) / 1=StructuredEvent JSON(ADR-0045 tag1).
   * 별도 콜백 대신 tag 필드로 통합한 이유: seq dedup·epoch 가드·pre-subscribe
   * 버퍼가 tag 를 몰라도 되게(한 seq 공간·한 배달 경로) — 콜백 이중화는 그 규율을 두 벌로 쪼갠다.
   */
  tag: number
  /** raw payload — tag0 이면 터미널 바이트, tag1 이면 StructuredEvent JSON UTF-8 바이트. */
  bytes: Uint8Array
}

/** 구독 해제 핸들 — 반드시 호출(unmount/재구독 시). 내부에서 transport 정리까지 수행. */
export interface OutputSubscription {
  unsubscribe: () => void
}

/**
 * 뷰 출력 국면 — `onState` 통지값이자 [`ViewOutputState.phase`] 의 값 집합(둘은 같은 어휘다).
 *
 * - `detached` — **붙을 에이전트가 없다**. 내는 자리는 둘뿐이다: 연결이 끊겼거나, 알려진 명부에 그 id 가
 *   없거나. 요청을 내지 않고 기다린다. 화면에 그려 둔 내용은 **지우지 않는다**("이 슬롯이 이 에이전트를
 *   본다"는 사용자 의도는 연결보다 오래 산다).
 *   ★"명부 자체를 아직 모른다" 는 이 값이 아니다★ — 부팅·웹뷰 리로드·명부 조회 실패가 그 구간이고,
 *   거기선 통지 없이 그냥 요청을 낸다(모름 ≠ 부재 — 합치면 조회 한 번 실패에 출력이 통째로 막힌다).
 *   그래서 **연결이 살아 있는 채로 이 값을 받았다면 명부가 "그 에이전트는 없다" 고 말한 것**이다.
 * - `buffering` — 축적 중(replay 경계 대기). ★비우면 안 된다★ — 무슨 이력이 올지 아직 모르는 구간이다.
 *   같은 화신이면 겹치는 앞부분을 dedup 이 흡수하고, 다른 화신이면 비우기는 `onReset` 이 따로 낸다.
 * - `live` — 직행 배달.
 * - `error` — 재요청 사다리 소진. `detached` 와 마찬가지로 **아무것도 오지 않는 상태**라, 소비자는 이걸
 *   화면에 드러내야 한다 — 안 그러면 명부에도 있고 연결도 붙은 슬롯이 신호 없는 빈 판으로 남는다.
 *   화면 내용은 여기서도 지우지 않는다(비우기는 `onReset` 단독).
 */
export type ViewPhase = 'detached' | 'buffering' | 'live' | 'error'

/**
 * 뷰 비우기 신호 — **다른 화신의 이력이 지금부터 배달된다**.
 *
 * 받는 쪽 의무: 그려 둔 것을 지우고 **자기 seq 가드도 함께** 되돌린다. 하나만 하면 겹쳐 그리거나(가드만
 * 되돌림) 새 프레임이 전부 탈락한다(화면만 지움 — 새 화신은 번호를 0 부터 다시 매긴다).
 *
 * ★요청이 아니라 **도착** 시점에 온다★ — 바로 뒤에 그 이력이 같은 틱으로 배달되므로 빈 화면이 비치는
 * 구간이 없다. 뒤집어 말하면 replay 가 거절·타임아웃되면 이 신호는 **오지 않고** 앞 화신의 화면이
 * 그대로 남는다(그게 의도다 — 데몬 재기동 직후가 정확히 그 경우다).
 */
export type ViewResetFn = () => void

/**
 * 뷰별 replay 상태 스냅샷(§5 LLM 제어 표면 — ADR-0046).
 * error(재요청 사다리 소진)·detached(붙을 에이전트 부재) 등을 LLM/자동화가 타입으로 발견·재구동 판단에
 * 쓴다. 최소 노출.
 */
export interface ViewOutputState {
  agentId: string
  phase: ViewPhase
  /** buffering 중 축적 프레임 수(디버그·관측). */
  buffered: number
  /** 재요청 사다리 시도 횟수(0=아직 재요청 안 함). */
  attempts: number
}

/**
 * 에이전트 제어/구독 단일 표면. 사람 UI 클릭과 (미래) LLM 호출이 같은 진입점을 거친다(§5).
 * 모든 side-effect 메서드는 idempotency·재시도 정책을 구현체가 책임진다.
 */
export interface AgentClient {
  // ── 연결 상태 ──────────────────────────────────────────────────────────────
  readonly connectionState: ConnectionState
  /**
   * 연결이 끊긴 **이유**(사람이 읽는 문장). 이유를 모르면 null.
   *
   * ★왜 상태와 같은 표면에 두나(ADR-0134)★: 데이터 폴더에 쓸 수 없어 데몬이 못 뜨는 실패는
   * 'down' 만으로는 원인 없는 시간 초과와 구분되지 않는다. 별도 핸들을 새로 파지 않고 이미 있는
   * 연결 상태 표면에 이유를 실어, 상태를 그리는 쪽이 같은 구독으로 함께 받는다(§5 LLM-우선 제어 —
   * 제어 표면은 하나다).
   */
  readonly connectionError: string | null
  /**
   * 상태 변화 구독. 반환은 해제 함수.
   * ★[`connectionError`] 가 바뀔 때도 통지된다★ — 상태 문자열이 그대로여도(예: 'down' 인 채 이유만
   * 밝혀져도) 구독자가 다시 그릴 수 있어야 한다.
   */
  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void
  /**
   * 연결 실패 이유를 기록한다(null = 지움). 다음 'connected' 전이에서 자동으로 지워진다.
   *
   * ★연결된 상태에서의 기록은 무시된다★: 지우는 계기가 'connected' 로의 *전이* 뿐이라, 이미
   * 연결된 채로 기록하면 지울 전이가 오지 않아 영구 고착된다. 그래서 connected 면 null 로 접는다.
   *
   * ★쓰는 곳은 지금 정확히 하나다★ — `DaemonControl.start()`. 부팅 bootstrap 과
   * `window.__ENGRAM_DAEMON__.start()` 가 그 한 곳을 지나므로 둘 다 덮인다.
   *
   * ★덮이지 **않는** 경로(알려진 공백 — 넓혀 적지 말 것)★:
   * - `AgentClient.connect()` — 공개 표면이고 내부적으로 spawn 까지 가지만 `DaemonControl` 을
   *   거치지 않는다. LLM·cdp 가 이걸 직접 부르면 실패 이유가 화면에 안 실린다.
   * - `DaemonControl.stop()` — 실패해도 기록하지 않는다(끄기 실패는 "연결 실패 이유"가 아니다).
   * - 네이티브 트레이의 "데몬 켜기" — Rust 가 `discovery::ensure_daemon` 을 직접 부르므로 프론트를
   *   아예 지나지 않는다(트레이는 자기 아이콘 상태로 표현한다).
   */
  reportConnectionError(reason: string | null): void

  /**
   * **명시 연결(spawn 허용)** — ADR-0021 §1. 부팅 1회 / 사용자 daemon_start 가
   * 부른다(DaemonControl.start). 데몬이 없으면 여기서만 spawn 한다. 명령 경로(ensureReady)는
   * attach-only 라 spawn 못 하므로, 데몬을 띄우는 유일한 의도적 진입점이다.
   * 재연결로 멈췄던 상태(closedByUser/attempt)를 리셋.
   */
  connect(): Promise<void>
  /**
   * **명시 연결 해제(재연결 중단, ADR-0021 note3)** — graceful daemon_stop
   * 후 부른다: closedByUser=true 로 즉시 'down' 정착해 5회 재연결 헛시도를 없앤다. ProtocolClient
   * 자체(구독 라우터/콜백 레지스트리)는 유지하므로, 이후 connect 로 다시 살릴 수 있다(close 와 다름).
   */
  disconnect(): void

  // ── 출력 구독 ──────────────────────────────────────────────────────────────
  /**
   * 뷰(slot) 단위 출력 구독(ADR-0046). viewId = 슬롯 id — 같은 agentId 를 N 뷰가 봐도 각자 독립 진도
   * (버그 B 구조 해소). onChunk 로 디코드된 바이트 전달. onState(옵션)는 국면 통지([`ViewPhase`]),
   * onReset(옵션)은 비우기 신호([`ViewResetFn`]) — 값별 소비자 의무는 각 타입에 있다. 반환 핸들의
   * unsubscribe 로 해제.
   */
  subscribeOutput(
    viewId: string,
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
    onState?: (state: ViewPhase) => void,
    onReset?: ViewResetFn,
  ): Promise<OutputSubscription>

  /**
   * 뷰(slot)별 replay 상태 조회(§5 LLM 제어 표면 — ADR-0046). error 소진(재요청 3회 실패)·buffering
   * 고착 등을 LLM/자동화가 관측·재구동 판단에 쓴다. 없는 viewId 면 null.
   */
  getViewOutputState(viewId: string): ViewOutputState | null

  // ── 상태/목록/복원 이벤트 ─────────────────────────────────────────────────────
  /** 권위 있는 에이전트 목록 교체(존재/제거 판정 기준). */
  onAgentListUpdated(cb: (agents: AgentInfo[]) => void): () => void
  /** 개별 status 갱신(목록 제거 안 함). */
  onStatusChanged(cb: (id: string, status: AgentStatus, epoch: number) => void): () => void
  /** 부팅 복원 결과(S9). */
  onRestoreResult(cb: (report: RestoreReport) => void): () => void
  /**
   * 프로필 목록 라이브 갱신(깡통/예약 에이전트 — ADR-0018 후속, §5).
   * 백엔드가 프로필 변경(create/delete/activate)을 broadcast 하면 store 미러를 갱신한다
   * (AgentEvent::ProfileListUpdated 라우팅).
   */
  onProfileListUpdated(cb: (profiles: AgentProfile[]) => void): () => void
  /**
   * 프리셋 목록 라이브 갱신(ADR-0061 — 프로필판과 동형, §5). 백엔드가 프리셋 CRUD(create/delete)를
   * broadcast 하면 store 미러를 갱신한다(AgentEvent::PresetListUpdated 라우팅).
   */
  onPresetListUpdated(cb: (presets: Preset[]) => void): () => void

  // ── 명령 ──────────────────────────────────────────────────────────────────
  spawnAgent(cwd: string): Promise<AgentInfo>
  killAgent(agentId: string): Promise<void>
  interruptAgent(agentId: string): Promise<void>
  writeStdin(agentId: string, data: Uint8Array): Promise<void>
  resizePty(agentId: string, cols: number, rows: number): Promise<void>
  getAgents(): Promise<AgentInfo[]>
  getSnapshot(agentId: string): Promise<unknown[]>
  /**
   * 데몬 graceful 종료(ADR-0021 §5). StopDaemon AgentCommand 전송 — 데몬이 자식 PTY 를 정리하고
   * 스스로 내려간다. force=false 면 실활성 에이전트가 있을 때 데몬이 거부(Error). DaemonControl.stop 이
   * 이걸 graceful 단계로 부르고, 실패/연결없음 시 daemon_stop(fallback kill)로 보강한다.
   */
  stopDaemon(force: boolean): Promise<void>

  // ── 프로필 CRUD ────────────────────────────────────────────────────────────
  listProfiles(): Promise<AgentProfile[]>
  /**
   * claude 프로필 생성. outputFormat 은 렌더 모드를 가른다(ADR-0044): 'Terminal'=PTY(xterm),
   * 'StreamJson'=헤드리스 NDJSON(RichSlot). 기본 'Terminal'(기존 호출자 동작 불변 — wire 는
   * `#[serde(default)]`). §5 제어 표면(cdp/console)이 이 인자로 json 에이전트를 스폰한다.
   */
  createClaudeProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
    outputFormat?: ClaudeOutputFormat,
  ): Promise<AgentProfile>
  deleteProfile(agentId: string): Promise<void>
  spawnProfile(agentId: string, resume: boolean): Promise<AgentInfo>
  setProfileAutoRestore(agentId: string, autoRestore: boolean): Promise<void>
  /**
   * 프로필 표시명 override 설정/해제(ADR-0061 리치화 — 트리 rename, §5). name=문자열 → override 저장,
   * null → 해제(cwd basename 파생 복귀). trim·빈문자열 거부·미변경 스킵은 호출부(UI)가 확정 직전 처리 —
   * 여기엔 유효 값 또는 명시 null 만 온다. 백엔드 reply=Ack(void), 표시명 반영은 뒤이은 ProfileListUpdated
   * broadcast(낙관 갱신 X). 없는 id 면 Error(setProfileAutoRestore 와 동형).
   */
  renameProfile(agentId: string, name: string | null): Promise<void>
  /**
   * 프로필 부모 재지정(ADR-0072 트리 계층, §5 LLM 제어 표면). childId 를 parentId 밑으로 이동
   * (null → 루트로 승격). 1단 중첩만 — 백엔드가 cycle/self/2단/존재하지 않는 parent 를 거부(Error).
   * renameProfile 과 동형 additive: reply=Ack(void), 계층 반영은 뒤이은 ProfileListUpdated broadcast
   * (낙관 갱신 X). 사람 드래그와 LLM 호출이 같은 진입점(§5 손발/두뇌 분리 — 트리 구성은 두뇌가 쥐는 핸들).
   */
  reparentProfile(childId: string, parentId: string | null): Promise<void>

  // ── 프리셋 CRUD(ADR-0061 — cwd 북마크, 스폰 안 함) ─────────────────────────────
  /** 저장된 프리셋 전체 조회. PresetList 전용 reply(request_id echo)로 회수. */
  listPresets(): Promise<Preset[]>
  /**
   * 프리셋 생성(cwd 북마크). ★이름은 넘기지 않는다★ — 백엔드는 {id,cwd}만 저장하고 표시명은 프론트가
   * cwd basename 으로 파생한다(ADR-0061). 백엔드 reply=Ack(void) — 생성된 Preset 은 뒤이은
   * PresetListUpdated broadcast 로 store 에 반영된다(프로필 CreateProfile 이 Created{profile} 를 돌려주는
   * 것과 다름 — 프리셋은 broadcast 로만 목록에 들어온다).
   */
  createPreset(cwd: string): Promise<void>
  /** 프리셋 삭제(에이전트는 무관하게 산다 — ADR-0061). 없는 id 면 no-op(Ack). */
  deletePreset(id: string): Promise<void>
  /**
   * 프리셋 표시명 override 설정/해제(ADR-0061 리치화, §5). name=문자열 → override 저장, null → 해제
   * (cwd basename 파생 복귀). trim·빈문자열 거부·미변경 스킵은 호출부(UI)가 확정 직전 처리. 백엔드
   * reply=Ack(void), 표시명 반영은 뒤이은 PresetListUpdated broadcast(낙관 갱신 X). 없는 id 면 no-op(Ack).
   */
  renamePreset(id: string, name: string | null): Promise<void>
}
