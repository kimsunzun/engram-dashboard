// ADR-0055 / ADR-0011: 에이전트 command 어댑터 — register 로 agentClient(단일 제어 표면) 스폰 경로에
//   라우팅만 한다(새 상태 경로 0). import 부수효과로 등록되므로 단일 매니페스트(contributions.ts)에서
//   side-effect import 한다. 검증: window.__engramCmd.run('agent.spawn',{cwd:'C:/work'}).
//
// ★ADR-0064 슬롯 메뉴 기여 co-location★: agentlist.createAgent(폴더 다이얼로그 → reserved claude 프로필 등록)를
//   등록하고 agent_list 슬롯 메뉴에 기여한다 — 트리(agent_list) 콘텐츠 지식이 이 모듈에 응집(공통은 '*'이 소유).

import { open } from '@tauri-apps/plugin-dialog'

import { t } from '../i18n'
import type { ClaudeOutputFormat } from '../api/types'
import { agentClient } from '../api/clientFactory'
import { useAgentStore } from '../store/agentStore'
import { refreshProfiles } from '../store/eventBus'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

// ★ADR-0078★: 렌더 모드(Terminal=xterm PTY / StreamJson=headless NDJSON→RichSlot)는 생성 시점에 고정하고
//   이후 불변이다 — pane "에이전트 생성" 서브메뉴에서 모드를 골라 예약 프로필을 만든다(활성화-시점 override 는
//   거부됨: 활성화 행 메뉴는 단일 "활성화" 유지). 세 생성 command 가 공유하는 헬퍼 — 폴더 다이얼로그로 cwd 를
//   고른 뒤 claude reserved(비활성) 프로필을 outputFormat 고정으로 등록한다(스폰하지 않음). 취소(null)면 no-op.
async function createReservedProfile(outputFormat: ClaudeOutputFormat) {
  const picked = await open({ directory: true, multiple: false, title: t('dialog.pickAgentCwd') })
  const cwd = typeof picked === 'string' ? picked : null
  if (!cwd) return // 취소 — no-op
  const profile = await agentClient.createClaudeProfile(cwd, cwd, [], [], false, outputFormat)
  // broadcast 는 유실 가능(ws 큐 포화, ws.rs:145)·구독이 레이아웃 초기화 이후(eventBus.ts)라, 생성 직후
  // 명시 refetch 로 예약 노드 표시를 보장한다(activateReserved 의 .then(refreshProfiles) 와 동형
  // belt-and-suspenders). 생성 프로필은 그대로 반환(회수부 cdp/메뉴가 계속 사용 가능).
  await refreshProfiles()
  return profile
}

// ★ADR-0078★: ClaudeOutputFormat 경계 검증기 — 컴파일타임 union 은 런타임 방어가 안 되므로 유효값
//   allowlist 로 좁힌다. 미지정(undefined/null)이면 'StreamJson' 기본(back-compat). 지정됐지만 두 유효값이
//   아니면 조용한 no-op·백엔드 전달 대신 명시 throw(잘못된 값 포함 — §5 LLM/cdp 디버깅).
const VALID_OUTPUT_FORMATS: readonly ClaudeOutputFormat[] = ['Terminal', 'StreamJson']
function coerceOutputFormat(raw: unknown): ClaudeOutputFormat {
  if (raw === undefined || raw === null) return 'StreamJson'
  if (VALID_OUTPUT_FORMATS.includes(raw as ClaudeOutputFormat)) return raw as ClaudeOutputFormat
  throw new Error(`agentlist.createAgent: 잘못된 outputFormat: ${String(raw)} (유효: 'Terminal' | 'StreamJson')`)
}

register({
  id: 'agent.spawn',
  title: t('agent.spawn'),
  category: 'agent',
  // 단일 객체 가방(ADR-0055): { preset?, cwd?, parent? }.
  //   - preset(id) 주어지면 store.presets 에서 cwd 를 해소(프리셋 = cwd 북마크, ADR-0061).
  //   - 아니면 raw cwd 사용. 둘 다 최종적으로 agentClient.spawnAgent(cwd)(데몬 SpawnByCwd)로 흐른다.
  //   - parent 는 SIGNATURE-ONLY: 중첩(부모 아래 스폰)은 아직 미지원 — 세팅되면 명시 throw(조용한
  //     무시 금지, LLM/cdp 디버깅 위해). 향후 트리 중첩 도입 시 여기서 배선한다.
  run: (args) => {
    const preset = args?.preset as string | undefined
    const rawCwd = args?.cwd as string | undefined
    const parent = args?.parent as string | undefined

    if (parent !== undefined && parent !== null) {
      throw new Error('agent.spawn: parent nesting 미지원')
    }

    // 프리셋 우선 — id 로 store.presets 조회해 cwd 해소. 없는 id 면 명시 throw(조용한 no-op 금지).
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
    return agentClient.spawnAgent(cwd.trim())
  },
})

register({
  id: 'agent.rename',
  title: t('agent.rename'),
  category: 'agent',
  // ADR-0061 리치화(§5 LLM 제어): 에이전트(프로필) 표시명 override set/clear. args={ id, name }.
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
  //   allowlist(ClaudeOutputFormat 은 컴파일타임 union 이라 런타임 enum 없음)로 걸러, 미지정이면 기본,
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
    ],
  },
])
