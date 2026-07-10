// ADR-0061 / ADR-0055: 프리셋 command 어댑터 — register 로 agentClient 프리셋 메서드(ProtocolClient
//   seam, ADR-0011)에 라우팅만 한다(새 상태 경로 0 — store 반영은 PresetListUpdated broadcast 가 담당).
//   import 부수효과로 등록되므로 단일 매니페스트(contributions.ts)에서 side-effect import 한다.
//   검증: window.__engramCmd.run('preset.create',{cwd:'C:/work'}) → PresetListUpdated → store.presets.
//
// ★ADR-0064 슬롯 메뉴 기여 co-location★: 이 모듈이 preset.add command(폴더 다이얼로그 → createPreset) 를
//   등록하고 preset_palette 슬롯 메뉴에 기여한다 — 콘텐츠(프리셋) 지식이 한 모듈에 응집(공통은 '*'이 소유).

import { open } from '@tauri-apps/plugin-dialog'

import { agentClient } from '../api/clientFactory'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

register({
  id: 'preset.list',
  title: '프리셋 목록 조회',
  category: 'preset',
  // 읽기 전용 조회 — Preset[] 를 그대로 반환(cdp/호출부가 await). store 미러 교체는 안 한다(조회 표면).
  run: () => agentClient.listPresets(),
})

register({
  id: 'preset.create',
  title: '프리셋 생성',
  category: 'preset',
  // args.cwd 만 destructure(단일 객체 가방, ADR-0055). 이름은 안 넘긴다 — 백엔드가 {id,cwd}만 저장하고
  //   표시명은 프론트가 cwd basename 으로 파생한다(ADR-0061). 반환(Promise<void>)을 그대로 흘려보낸다.
  run: (args) => {
    const cwd = args?.cwd as string | undefined
    if (!cwd || !cwd.trim()) {
      throw new Error(`[preset.create] cwd 가 비어 있음: ${String(cwd)}`)
    }
    return agentClient.createPreset(cwd.trim())
  },
})

register({
  id: 'preset.delete',
  title: '프리셋 삭제',
  category: 'preset',
  // args.id 만 destructure. 없는 id 는 백엔드 no-op(Ack) — 프론트에서 존재 검증 안 함(ADR-0061).
  run: (args) => {
    const id = args?.id as string | undefined
    if (!id || !id.trim()) {
      throw new Error(`[preset.delete] id 가 비어 있음: ${String(id)}`)
    }
    return agentClient.deletePreset(id.trim())
  },
})

register({
  id: 'preset.add',
  title: '추가',
  category: 'preset',
  // ★네이티브 폴더 다이얼로그 → createPreset★(ADR-0061 / ADR-0064): 옛 PresetPalette 의 pane "추가" 메뉴
  //   로직을 command 로 승격한다. OS 폴더 선택 창(open({directory:true}))에서 고른 cwd(non-null)를 그대로
  //   agentClient.createPreset 로 넘긴다(store 반영은 PresetListUpdated broadcast — 낙관 갱신 X). 취소(null)면
  //   no-op. 다이얼로그는 네이티브 창(webview 밖)이라 cdp 검증 불가 — open→createPreset 배선만 단위테스트.
  run: async () => {
    const picked = await open({ directory: true, multiple: false, title: '프리셋 경로 선택' })
    const cwd = typeof picked === 'string' ? picked : null
    if (!cwd) return // 취소 — no-op
    return agentClient.createPreset(cwd)
  },
})

// ADR-0064: preset_palette 슬롯 메뉴에 preset.add 기여(group='content' — 공통 slot-ops 위에 렌더).
registerSlotMenu('preset_palette', [{ commandId: 'preset.add', group: 'content', order: 10 }])
