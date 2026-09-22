// ADR-0055 / ADR-0011: agentClient(단일 제어 표면) 스폰 경로에 라우팅만 한다(새 상태 경로 0).
//
// ★ADR-0064 슬롯 메뉴 기여 co-location★: 트리(agent_list) 콘텐츠 지식이 이 모듈에 응집(공통은 '*'이 소유).

import { open } from '@tauri-apps/plugin-dialog'

import { t } from '../i18n'
import type { AgentProfile, AgentOutputFormat } from '../api/types'
import { agentClient } from '../api/clientFactory'
import { useAgentStore } from '../store/agentStore'
import { refreshProfiles } from '../store/eventBus'
import { matchDeclaredSpelling } from './enumArg'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

// ★ADR-0078★: 렌더 모드(Terminal=xterm PTY / StreamJson=headless NDJSON→RichSlot)는 생성 시점에 고정하고
//   이후 불변이다 — pane "에이전트 생성" 서브메뉴에서 모드를 골라 예약 프로필을 만든다(활성화-시점 override 는
//   거부됨: 활성화 행 메뉴는 단일 "활성화" 유지). claude reserved(비활성) 프로필을 등록만 한다(스폰하지 않음).
async function createReserved(make: (cwd: string) => Promise<AgentProfile>) {
  const picked = await open({ directory: true, multiple: false, title: t('dialog.pickAgentCwd') })
  const cwd = typeof picked === 'string' ? picked : null
  if (!cwd) return // 취소 — no-op
  const profile = await make(cwd)
  // broadcast 는 유실 가능(ws 큐 포화, ws.rs:145)·구독이 레이아웃 초기화 이후(eventBus.ts)라, 생성 직후
  // 명시 refetch 로 예약 노드 표시를 보장한다(activateReserved 의 .then(refreshProfiles) 와 동형
  // belt-and-suspenders).
  await refreshProfiles()
  return profile
}

async function createReservedProfile(outputFormat: AgentOutputFormat) {
  return createReserved(cwd => agentClient.createClaudeProfile(cwd, cwd, [], [], false, outputFormat))
}

async function createReservedCodexProfile(outputFormat: AgentOutputFormat) {
  return createReserved(cwd => agentClient.createCodexProfile(cwd, cwd, [], [], false, outputFormat))
}

// ★codex 생성 문 둘의 `humanOnly` 게이트는 2026-09-22 에 걷혔다★ — 그 게이트가 세워진 전제(「처음 보는
//   폴더에서 codex 가 신뢰 확인 모달을 띄우고 사람이 아닌 호출자는 못 지난다」)가 실측으로 뒤집혔기
//   때문이다: codex 의 `app-server` 경로는 한 번도 본 적 없는 폴더에 성공으로 답하고 그 폴더를 스스로
//   신뢰 목록에 적는다. 폴더 신뢰 모달은 TUI 전용 장치였다.
// ★사유·근거의 정본은 여기가 아니다★ — `engram-dashboard-agent` 의 `commands::LLM_BACKEND_POLICY` 의
//   codex 줄이고, 형제 문 둘(`agent.new` · `agent.spawnInto`)이 같은 표를 본다. 이 파일에 있던 사본
//   문자열(`CODEX_HUMAN_ONLY`)은 그 표가 codex 를 열면서 함께 지웠다 — 두 문장을 맞춰 주는 게이트가
//   없었으므로 남겨 두면 표와 어긋난 채 살아남는다.
// ★되돌리려거든 여기가 아니라 그 표부터★ — 이 문들을 다시 닫는 편집은 `humanOnly` 를 되살리기 전에
//   그 표의 codex 줄을 닫아야 한다(안 그러면 같은 백엔드를 `agent.new` 로는 만들고 이 문으로는 못
//   만드는 어긋남이 생기고, 그것을 재는 자리가 `agentCommands.test.ts` 와 `layout_apply.rs` 다).

// ★ADR-0078★: AgentOutputFormat 경계 검증기 — 컴파일타임 union 은 런타임 방어가 안 되므로 유효값
//   allowlist 로 좁힌다. 미지정(undefined/null)이면 'StreamJson' 기본(back-compat). 지정됐지만 두 유효값이
//   아니면 조용한 no-op·백엔드 전달 대신 명시 throw(잘못된 값 포함 — §5 LLM/cdp 디버깅).
// ★대소문자는 호출자 자유이고 나가는 값은 아니다★: 같은 낱말이 CLI 로는 되고 이 표면으로는 안 되는
//   어긋남을 없앤다(사용자 결정 2026-09-23 — Rust 쪽 `AgentBackendKind` 역직렬화·명령 버스 enum 접기와
//   같은 결정). 여기서 접은 뒤 나가는 것은 언제나 아래 선언 철자라 wire 값은 종전과 바이트 동일하다.
const VALID_OUTPUT_FORMATS: readonly AgentOutputFormat[] = ['Terminal', 'StreamJson']
function coerceOutputFormat(raw: unknown): AgentOutputFormat {
  if (raw === undefined || raw === null) return 'StreamJson'
  const matched = matchDeclaredSpelling(raw, VALID_OUTPUT_FORMATS)
  if (matched) return matched
  throw new Error(`agentlist.createAgent: 잘못된 outputFormat: ${String(raw)} (유효: 'Terminal' | 'StreamJson')`)
}

register({
  id: 'agent.spawn',
  title: t('agent.spawn'),
  category: 'agent',
  //   - preset(id) 주어지면 store.presets 에서 cwd 를 해소(프리셋 = cwd 북마크, ADR-0061). 최종적으로
  //     agentClient.spawnAgent(cwd)(데몬 SpawnByCwd)로 흐른다.
  //   - parent 는 SIGNATURE-ONLY: 중첩(부모 아래 스폰)은 아직 미지원 — 세팅되면 명시 throw(조용한
  //     무시 금지, LLM/cdp 디버깅 위해). 향후 트리 중첩 도입 시 여기서 배선한다.
  run: (args) => {
    const preset = args?.preset as string | undefined
    const rawCwd = args?.cwd as string | undefined
    const parent = args?.parent as string | undefined

    if (parent !== undefined && parent !== null) {
      throw new Error('agent.spawn: parent nesting 미지원')
    }

    let cwd: string | undefined
    if (preset) {
      const found = useAgentStore.getState().presets.find(p => p.id === preset)
      if (!found) {
        throw new Error(`agent.spawn: 알 수 없는 preset id: '${preset}'`)
      }
      cwd = found.cwd
    } else {
      cwd = rawCwd
    }

    if (!cwd || !cwd.trim()) {
      throw new Error(`agent.spawn: cwd 가 비어 있음: ${String(cwd)}`)
    }
    // ★이 문은 claude 만 낸다 — 2026-09-22 에 **사유가 갈렸다**★. 옛 사유(「codex 는 신뢰 확인 모달을
    //   띄우고 LLM 은 그것을 못 지난다」)는 실측으로 죽었다(그 근거의 자리 = 이 파일 머리 주석). 지금
    //   남는 사유는 **인자 표면을 늘리지 않는다**는 것 하나다: 이 command 에 backend 인자를 내면
    //   같은 id 의 백엔드 문(`engram-dashboard-agent` 의 `agent.spawn`)과 어휘가 갈린다 — 그쪽도 같은
    //   이유로 claude 에 박혀 있다(`commands.rs` 의 `create_and_start`).
    // ★codex 를 만드는 LLM 경로는 `agent.new` 다★ — backend 와 output_format 을 거기서 함께 고른다.
    return agentClient.spawnAgent(cwd.trim(), 'claude')
  },
})

register({
  id: 'agent.rename',
  title: t('agent.rename'),
  category: 'agent',
  // ADR-0061 리치화(§5 LLM 제어): 에이전트(프로필) 표시명 override set/clear.
  //   - name 문자열 → override 저장. name 이 null/빈문자열/미지정 → override 해제(cwd basename 파생 복귀).
  //   반영은 ProfileListUpdated broadcast(낙관 갱신 X). 없는 id 는 백엔드 Error. 이것이 rename 을 프론트
  //   전용이 아니라 백엔드 저장으로 두는 이유 — LLM 이 같은 표면(command)으로 트리 표시명을 바꿀 수 있다.
  run: (args) => {
    const id = args?.id as string | undefined
    if (!id || !id.trim()) {
      throw new Error(`[agent.rename] id 가 비어 있음: ${String(id)}`)
    }
    const raw = args?.name
    const name = typeof raw === 'string' && raw.trim().length > 0 ? raw.trim() : null
    return agentClient.renameProfile(id.trim(), name)
  },
})

register({
  id: 'agentlist.createAgent',
  title: t('agent.create'),
  category: 'agent',
  // ★ADR-0064★: agent_list(트리) 슬롯 pane 메뉴 "에이전트 생성" 계열의 파라미터형 §5 LLM 프리미티브 —
  //   폴더 다이얼로그로 cwd 를 고른 뒤 claude reserved(비활성) 프로필을 등록한다(스폰하지 않음). 트리에 예약
  //   노드로 뜨고, 활성화(더블클릭/우클릭 활성화 → spawnProfile)에서 비로소 claude 를 spawn 한다.
  // ★동작 변경(WHY)★: 옛 흐름은 agent.spawn({cwd}) → SpawnByCwd 로 *즉시* 셸(cmd.exe) 에이전트를 띄웠다
  //   (kind='running', claude 아님·예약 아님) — 사용자 확정 의도(생성=claude reserved, 활성화=claude spawn)와
  //   어긋났다. 그래서 여기서 즉시 스폰 대신 createReservedProfile 로 등록만 한다. agent.spawn 원시명령은
  //   그대로 둔다(LLM/cdp 즉시-스폰 프리미티브로 유효 — 별개 관심사).
  // ★ADR-0078★: outputFormat 은 args.outputFormat 으로 받되 미지정 시 'StreamJson' 기본(back-compat — 옛
  //   호출·테스트·LLM 참조가 인자 없이 부르면 종전 동작 유지). 사람 메뉴 경로는 아래 두 leaf command
  //   (createTerminal/createJson)가 모드를 명시 고정한다. command id 는 보존(하위호환).
  //   ★경계 검증(§5 LLM/cdp 프리미티브)★: outputFormat 은 외부 입력이라 무검증 캐스트 금지 — 런타임
  //   allowlist(AgentOutputFormat 은 컴파일타임 union 이라 런타임 enum 없음)로 걸러, 미지정이면 기본,
  //   잘못된 값이면 조용히 백엔드로 흘리지 않고 명시 throw(agent.spawn/rename 과 동일 fail-loud — LLM/cdp
  //   디버깅 위해 잘못된 값 포함).
  run: async (args) => createReservedProfile(coerceOutputFormat(args?.outputFormat)),
})

register({
  id: 'agentlist.createTerminal',
  title: t('agent.createTerminal'),
  category: 'agent',
  // ★ADR-0078★: 렌더 모드 Terminal(xterm PTY) 고정 생성 — 서브메뉴 leaf. 생성 시점에 모드 확정·이후 불변.
  run: async () => createReservedProfile('Terminal'),
})

register({
  id: 'agentlist.createCodex',
  title: t('agent.createCodex'),
  category: 'agent',
  // ★사람이 codex 대화형 TUI 를 고르는 문★ — 여기서 만들어진 예약 노드를 활성화하면 codex 가 그
  //   폴더에서 뜬다. ★첫 방문 폴더에서는 codex 자신의 신뢰 확인 모달이 화면에 그대로 뜨고 사람이
  //   지나간다(사용자 결정 2026-09-07 — 우리가 미리 신뢰를 심어 우회하지 않는다). 그 모달은 **TUI
  //   경로에만** 있다 — 그래서 이 문이 LLM 에게 닫혀 있을 이유가 되지 못했고, 2026-09-22 에
  //   `humanOnly` 를 걷었다(위 머리 주석이 근거의 자리를 가리킨다).
  run: async () => createReservedCodexProfile('Terminal'),
})

register({
  id: 'agentlist.createCodexJson',
  title: t('agent.createCodexJson'),
  category: 'agent',
  // ★사람이 codex 상주 JSON 서버(`codex app-server`)를 고르는 문★ — 형제 `createCodex` 와 같은
  //   백엔드이고 가르는 것은 출력 모드 하나다. 그 모드가 통로·입력 인코딩·출력 decoder 를 함께
  //   가르는 자리는 `engram-dashboard-agent` 의 `backend::codex::is_app_server` 다.
  // ★ADR-0078 과 같은 계약★: 모드는 생성 시점에 고정되고 이후 불변이다(활성화-시점 override 없음).
  // ★형제 `createCodex` 와 같은 날 `humanOnly` 를 걷었다(2026-09-22)★ — 두 문은 같은 게이트를 지므로
  //   한쪽만 열면 그 자체가 회귀다(그것을 재는 자리 = `agentCommands.test.ts`).
  run: async () => createReservedCodexProfile('StreamJson'),
})

register({
  id: 'agentlist.createJson',
  title: t('agent.createJson'),
  category: 'agent',
  // ★ADR-0078★: 렌더 모드 StreamJson(headless NDJSON→RichSlot) 고정 생성 — 서브메뉴 leaf. 생성 시점 확정·불변.
  run: async () => createReservedProfile('StreamJson'),
})

// ★ADR-0078★: agent_list pane 메뉴 "에이전트 생성"을 1단 서브메뉴 컨테이너로 — 렌더 모드를 *생성 시점*에
//   고르게(이후 불변). children 은 flat leaf(선언 순서 보존, ADR-0065) — 활성화-시점 override 는 거부됨.
// ADR-0064: group='content' — 공통 slot-ops 위에 렌더.
registerSlotMenu('agent_list', [
  {
    title: t('agent.create'),
    group: 'content',
    order: 10,
    children: [
      { commandId: 'agentlist.createJson', group: 'content', order: 10 },
      { commandId: 'agentlist.createTerminal', group: 'content', order: 20 },
      { commandId: 'agentlist.createCodex', group: 'content', order: 30 },
      { commandId: 'agentlist.createCodexJson', group: 'content', order: 40 },
    ],
  },
])
