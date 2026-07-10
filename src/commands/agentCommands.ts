// ADR-0055 / ADR-0011: 에이전트 command 어댑터 — register 로 agentClient(단일 제어 표면) 스폰 경로에
//   라우팅만 한다(새 상태 경로 0). import 부수효과로 등록되므로 부팅 경로(App.tsx)에서 side-effect
//   import 한다(presetCommands 미러). 검증: window.__engramCmd.run('agent.spawn',{cwd:'C:/work'}).

import { agentClient } from '../api/clientFactory'
import { useAgentStore } from '../store/agentStore'
import { register } from './registry'

register({
  id: 'agent.spawn',
  title: '에이전트 생성(spawn)',
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
