// ADR-0055 / ADR-0011: 에이전트 command 어댑터 — register 로 agentClient(단일 제어 표면) 스폰 경로에
//   라우팅만 한다(새 상태 경로 0). import 부수효과로 등록되므로 단일 매니페스트(contributions.ts)에서
//   side-effect import 한다. 검증: window.__engramCmd.run('agent.spawn',{cwd:'C:/work'}).
//
// ★ADR-0064 슬롯 메뉴 기여 co-location★: agentlist.createAgent(폴더 다이얼로그 → reserved claude 프로필 등록)를
//   등록하고 agent_list 슬롯 메뉴에 기여한다 — 트리(agent_list) 콘텐츠 지식이 이 모듈에 응집(공통은 '*'이 소유).

import { open } from '@tauri-apps/plugin-dialog'

import { t } from '../i18n'
import { agentClient } from '../api/clientFactory'
import { useAgentStore } from '../store/agentStore'
import { refreshProfiles } from '../store/eventBus'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

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
  // ★ADR-0064★: agent_list(트리) 슬롯 pane 메뉴의 "에이전트 생성" — 네이티브 폴더 다이얼로그로 cwd 를 고른
  //   뒤 claude reserved(비활성) 프로필을 등록한다(스폰하지 않음). 트리에 예약 노드로 뜨고, 활성화(더블클릭/
  //   우클릭 활성화 → spawnProfile)에서 비로소 claude 를 spawn 한다.
  // ★동작 변경(WHY)★: 옛 흐름은 agent.spawn({cwd}) → SpawnByCwd 로 *즉시* 셸(cmd.exe) 에이전트를 띄웠다
  //   (kind='running', claude 아님·예약 아님) — 사용자 확정 의도(생성=claude reserved, 활성화=claude spawn)와
  //   어긋났다. 그래서 여기서 즉시 스폰 대신 createClaudeProfile 로 등록만 한다. agent.spawn 원시명령은
  //   그대로 둔다(LLM/cdp 즉시-스폰 프리미티브로 유효 — 별개 관심사).
  //   ★옛 AgentList 인-컴포넌트 bg 픽커를 대체★: 프리셋-리스트 기반 spawn 은 이 흐름에서 빠진다(후속으로
  //   프리셋 행 액션 "이 프리셋으로 생성" 추가 예정, ADR-0064). 취소(null)면 no-op. async 로 감싸 회수부
  //   (cdp/메뉴)가 await·catch 가능. 인자: name=cwd(옛 SpawnByCwd 관례 — 트리 표시명은 cwd basename 파생이라
  //   name 은 표시가 아님), extraArgs/env=[], autoRestore=false, outputFormat='StreamJson'(헤드리스
  //   stream-json — RichSlot 구조화 렌더; resume 지원됨, ADR-0044 후속 완료 spike-verified).
  //   예약 노드 반영은 CreateProfile 뒤이은 ProfileListUpdated broadcast → store.setProfiles(eventBus)
  //   + 유실 대비 생성 직후 명시 refetch(run body 아래 — broadcast 단독 의존 아님).
  run: async () => {
    const picked = await open({ directory: true, multiple: false, title: t('dialog.pickAgentCwd') })
    const cwd = typeof picked === 'string' ? picked : null
    if (!cwd) return // 취소 — no-op
    const profile = await agentClient.createClaudeProfile(cwd, cwd, [], [], false, 'StreamJson')
    // broadcast 는 유실 가능(ws 큐 포화, ws.rs:145)·구독이 레이아웃 초기화 이후(eventBus.ts)라, 생성 직후
    // 명시 refetch 로 예약 노드 표시를 보장한다(activateReserved 의 .then(refreshProfiles) 와 동형
    // belt-and-suspenders). 생성 프로필은 그대로 반환(회수부 cdp/메뉴가 계속 사용 가능).
    await refreshProfiles()
    return profile
  },
})

// ADR-0064: agent_list 슬롯 메뉴에 agentlist.createAgent 기여(group='content' — 공통 slot-ops 위에 렌더).
registerSlotMenu('agent_list', [{ commandId: 'agentlist.createAgent', group: 'content', order: 10 }])
