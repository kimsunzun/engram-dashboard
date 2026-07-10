// ADR-0064 §4: command + 슬롯 메뉴 기여 등록 매니페스트(단일 로딩점).
//
// ★역할★: 모든 command 어댑터·슬롯 메뉴 기여 모듈을 여기 한 곳에서 side-effect import 한다. 부팅(App.tsx)은
//   이 매니페스트 하나만 import 해 등록부가 일원화된다(옛 App.tsx 산발 import 4줄 제거). "새 콘텐츠 = 그
//   모듈 + 여기 한 줄" 규약(ADR-0064 불변식: 산발 import 금지).
//
// ★로드 순서 주의★: slotCommands / slotContentCommands / presetCommands / agentCommands 는 command 를
//   register 한 *뒤* registerSlotMenu 로 그 id 를 참조한다. buildSlotMenu 의 resolve 는 렌더 시점(우클릭)에
//   일어나므로 import 순서와 무관하지만, 매니페스트 로드 시점엔 모든 register 가 끝나 있어야 우클릭이 안전하다
//   — 이 파일이 전부 import 하므로 그 불변식이 성립한다.

import './themeCommands'
import './tabCommands'
import './slotCommands' // 공통 슬롯 ops(split/popout/empty/close) + '*' 기여
import './slotContentCommands' // empty fill-ops + agent 종료 기여(코어 콘텐츠 타입)
import './presetCommands' // preset CRUD + preset_palette 기여
import './agentCommands' // agent spawn + agent_list 기여

import { validateSlotMenuContributions } from './slotMenu'

// FIX-1(ADR-0064): 모든 command·기여 모듈이 import 된 뒤(=register 완료) 전수 검증. 기여한 commandId 가
//   registry 에 없으면 부팅 즉시 console.error 로 발각한다(우클릭 때 buildSlotMenu 의 skip+log 를 기다리지
//   않고). fail-loud 를 crash 없이 달성 — 이 시점엔 위 side-effect import 로 모든 register 가 끝나 있다.
validateSlotMenuContributions()
