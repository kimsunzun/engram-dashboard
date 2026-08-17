// ★유일한 레이아웃 렌더러★(Brick 1): 옛 프론트 전용 slotStore/LayoutRenderer(number id + content union)는
// 제거됐다. 이 렌더러는 wire LayoutNode(string UUID id + content: SlotContent, ADR-0060, src-tauri/bindings)만 그린다 —
// 사람 클릭(SlotContextMenu — 빈 슬롯은 좌클릭도)이든 LLM(window.__engramLayout)이든 같은 invoke→emit
// 권위 루프로 갱신된다.

import { useState } from 'react'
import { Allotment } from 'allotment'
import { Plus } from 'lucide-react'

import type { LayoutNode } from '../../api/layoutTypes'
import { useCurrentViewId, useViewStore } from '../../store/viewStore'
import { useAgentStore } from '../../store/agentStore'
import TerminalSlot from '../slot/TerminalSlot'
import RichSlot from '../slot/RichSlot'
import DomSlot from '../slot/DomSlot'
import PresetPalette from '../slot/PresetPalette'
import AgentList from '../agent/AgentList'
import { isContentSlot } from '../agent/selectOpenTarget'
import SlotContextMenu from '../slot/SlotContextMenu'
import { buildSlotMenu } from '../../commands/slotMenu'
import { defaultRenderMode } from '../slot/renderMode'
import { t } from '../../i18n'

export default function ViewLayoutRenderer({
  node,
  focusedSlotId,
  viewIdOverride,
}: {
  node: LayoutNode
  focusedSlotId: string | null
  // ★이 렌더러가 그리는 View id 오버라이드(선택).★ WindowLayout(main·팝업)이 각 탭 캔버스에 그 탭 view 를
  //   넘겨(ADR-0057) 내부 SlotContextMenu 의 액션 좌표를 그 탭 view 로 고정한다. 없으면 메뉴가
  //   useCurrentViewId(이 웹뷰 창의 active 탭) 폴백.
  viewIdOverride?: string | null
}) {
  const renderModeOverride = useViewStore(s => s.renderModeOverride)
  // ★M2 caps 분기(ADR-0044)★: agent 배정 슬롯의 렌더러는 그 agent 의 output caps 로 고른다. caps 는
  // AgentInfo 로 이미 wire 를 건너와 store 에 있다(M1) — 여기선 조회만(추가 배선 불필요).
  const agents = useAgentStore(s => s.agents)
  // ★우클릭 슬롯 메뉴 상태(§5)★: 슬롯 하나당 이 렌더러 인스턴스가 하나라(재귀 렌더) 여기 useState 는
  //   그 슬롯 전용 메뉴 좌표다. 열림 시 SlotContextMenu 를 이 렌더러 안에서 직접 마운트한다 — 옛
  //   LayoutRenderer→SlotPane 래핑 경로가 Brick 1 에서 삭제돼 메뉴가 캔버스에서 닿지 않던 갭을 메운다.
  //   ★hooks 무조건 호출★: split/slot 분기 이전에 부른다(조건부 호출 금지).
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null)
  // ★이 메뉴가 조작할 View 좌표(ADR-0064)★: 옛 SlotContextMenu 내부 폴백을 여기(ctx 조립처)로 끌어올렸다.
  const currentViewId = useCurrentViewId()
  const targetViewId = viewIdOverride ?? currentViewId

  if (node.type === 'slot') {
    const isFocused = node.id === focusedSlotId
    // ADR-0060: 슬롯 점유자 = SlotContent 태그드 유니온.
    const slotAgentId = node.content.type === 'agent' ? node.content.agent_id : null
    const agent = slotAgentId != null ? (agents.find(a => a.id === slotAgentId) ?? null) : null
    // ★caps 도착 후에만 구체 렌더러를 마운트한다(ADR-0041 replay 소유권)★: 데몬 replay 는 slot-assign
    //   델타((window,agent) 키)에서 단 1회만 발화하고, 컴포넌트 스왑(TerminalSlot→RichSlot)엔 재발화하지
    //   않는다. 그래서 caps 미도착 상태에서 TerminalSlot 을 먼저 띄웠다가 caps 도착 후 RichSlot 으로 갈아끼면,
    //   스왑된 RichSlot 이 빈 채로 마운트돼 스왑 전 바이트가 영구 유실된다. 대신 caps(=AgentInfo) 도착 전엔
    //   중립 플레이스홀더만 두고(아래 '에이전트 연결 중…'), 첫 구체 렌더러를 caps 확정 후 마운트해 assign
    //   시점 replay 를 온전히 받게 한다. (터미널 에이전트는 보통 assign 전에 AgentInfo 가 오므로 이 플레이스
    //   홀더는 일시적 엣지 상태다 — 터미널 replay 경로는 종전과 동일.)
    // 구조화 출력(NDJSON) = 라이브 RichSlot, 아니면 TerminalSlot(xterm) 분기 근거(ADR-0002/0044).
    const capsReady = slotAgentId != null && agent != null
    const mode = agent != null ? (renderModeOverride[node.id] ?? defaultRenderMode(agent)) : null
    // preset_palette·agent_list variant 도 슬롯을 100% 채우는 실 렌더러라 hasContent=true(중앙정렬
    //   플레이스홀더 스타일이 이들 레이아웃을 깨지 않게, ADR-0060/0061/0062).
    const isPresetPalette = node.content.type === 'preset_palette'
    const isAgentList = node.content.type === 'agent_list'
    const hasContent = capsReady || isPresetPalette || isAgentList
    return (
      <div
        style={{
          height: '100%',
          background: 'var(--bg)',
          // border 폭을 항상 1px 고정해 포커스 이동 시 layout shift 제거.
          border: '1px solid var(--border)',
          // ★포커스 링은 여기(래퍼)가 아니라 아래 absolute 오버레이로 그린다★: inset box-shadow 를 래퍼에
          //   직접 주면 overflow:hidden 슬롯에서 100% 채운 자식 컨텐츠(터미널 canvas·RichSlot)가 덮어 안
          //   보였다(빈 슬롯만 보이던 버그, 사용자 제보 2026-07-16). position:relative 로 오버레이 앵커만 잡는다.
          position: 'relative',
          boxSizing: 'border-box',
          // 콘텐츠(터미널/rich) 있을 때: 슬롯을 100% 채우도록 여백·정렬 제거(center 정렬 끼면 깨짐).
          // 빈 슬롯(empty): 플레이스홀더를 중앙정렬하는 flex 유지.
          ...(hasContent
            ? { overflow: 'hidden' }
            : {
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                justifyContent: 'center',
                color: 'var(--text-muted)',
                fontFamily: 'var(--font-ui)',
                fontSize: '12px',
                gap: '4px',
              }),
        }}
        // 슬롯 식별용 data 속성 — cdp eval 에서 DOM 으로 split 결과(슬롯 수)를 셀 수 있게.
        data-slot-id={node.id}
        // ADR-0066: click-to-focus — 슬롯 pane 클릭 시 이 슬롯을 포커스로 지정한다. viewStore.focusSlot →
        //   invoke(focus_slot) → emit(layout:updated) 단일 제어 표면(사람 클릭 = LLM = slot.focus command, §5).
        //   ★낙관 갱신 X★: 링(isFocused)은 백엔드 emit 스냅샷으로만 갱신된다(권위 = src-tauri, ADR-0035).
        //   ★버블 허용(stopPropagation/preventDefault 안 함)★: 내부 상호작용(터미널 포커스·AgentList 버튼 등)을
        //   가로채지 않는다 — pane 어디를 눌러도 내부 핸들러가 그대로 발화한다.
        //   ★제어 슬롯 포커스 제외 — allowlist(콘텐츠 슬롯만 포커스), ADR-0066 정제★: 콘텐츠 슬롯
        //   (empty/agent)일 때만 focusSlot 호출한다. 트리(agent_list)·팔레트(preset_palette) 등 제어 슬롯,
        //   그리고 앞으로 추가될 제어 variant(ADR-0060 FileTree/ControlPanel 등)는 자동으로 비포커스된다
        //   — 게이트 기준을 denylist(제어 나열)가 아니라 selectOpenTarget 와 공유하는 단일 분류기
        //   isContentSlot(allowlist)로 잡아 "열기" 대상 선택과 기준 이원화를 막는다.
        //   이 게이트가 없으면 트리 노드 좌클릭이 트리 슬롯 pane 까지 버블해 트리 슬롯이 포커스되고, 이어
        //   우클릭 "열기"가 그 트리 슬롯을 대상으로 잡아 트리를 에이전트 터미널로 덮어썼다(선존 UX 버그).
        //   targetViewId 미확정(부팅 직후 탭 상태 미도착)이면 no-op(잘못된 view 로 focus 유출 방지).
        //   ★빈 슬롯 좌클릭 = 메뉴(ADR-0142)★: 표적은 아이콘이 아니라 슬롯 전체이고, 우클릭과 같은
        //     setContextMenu 상태·같은 좌표를 쓴다. 콘텐츠 슬롯(터미널·트리·팔레트)의 좌클릭은 포커스
        //     이동뿐이다 — 자기 클릭 의미를 가진 렌더러들이라 메뉴를 열면 그 의미를 덮는다.
        //   ★이미 열린 메뉴는 재앵커하지 않는다★: SlotContextMenu 는 포털이 아니라 이 래퍼 안에 마운트돼
        //     서브메뉴 컨테이너 행·항목 사이 여백 클릭이 여기까지 버블한다. 재앵커하면 메뉴가 커서 밑으로
        //     점프해 앵커가 메뉴 사각형 안에 들어가고(SlotContextMenu 의 뒤집기 불변식이 막으려는 상태)
        //     이어지는 클릭이 커서 밑 항목을 실행한다. 슬롯 여백 클릭은 메뉴의 mousedown 바깥닫기가 먼저
        //     상태를 비우므로 이 가드에 걸리지 않는다(= 메뉴가 새 좌표로 옮겨 열린다).
        onClick={e => {
          if (node.content.type === 'empty' && contextMenu == null) {
            setContextMenu({ x: e.clientX, y: e.clientY })
          }
          if (!isContentSlot(node.content)) return
          if (targetViewId) void useViewStore.getState().focusSlot(targetViewId, node.id)
        }}
        // ADR-0035: 메뉴 액션(분할/닫기/배정)은 viewStore(=window.__engramLayout) 단일 제어 표면으로만
        //   흐른다(사람 클릭 = LLM 이 한 표면, §5).
        onContextMenu={e => {
          e.preventDefault()
          setContextMenu({ x: e.clientX, y: e.clientY })
        }}
      >
        {node.content.type === 'agent' ? (
          agent == null ? (
            <span>{t('agent.connecting')}</span>
          ) : (
            (() => {
              // ★viewId = node.id(slot id, ADR-0046)★: 슬롯이 자기 slot id 로 구독한다 — 같은 agentId 두
              //   슬롯도 독립 진도(버그 B 해소). key 도 slot id 로 두어(옛 agent_id 키는 같은 agent 두 슬롯이
              //   같은 React key 가 돼 remount 가 꼬였다) 슬롯 정체성을 slot 단위로 고정한다.
              switch (mode) {
                case 'dom':
                  // ★DOM 모드(§5 관측)★: 같은 출력 스트림을 평문 <pre> 로 그려 CDP eval/innerText 로 읽히게
                  // 한다(터미널 xterm 은 canvas 라 관측 불가).
                  return <DomSlot key={node.id} viewId={node.id} agentId={slotAgentId!} epoch={agent.epoch} />
                case 'rich':
                  // epoch 은 재spawn 재구독 트리거.
                  return <RichSlot key={node.id} viewId={node.id} agentId={slotAgentId!} epoch={agent.epoch} />
                case 'terminal':
                default:
                  return <TerminalSlot key={node.id} viewId={node.id} agentId={slotAgentId!} />
              }
            })()
          )
        ) : node.content.type === 'preset_palette' ? (
          // 목록/추가/삭제는 PresetPalette 내부에서 agentClient(단일 제어 표면)로 흐른다.
          <PresetPalette />
        ) : node.content.type === 'agent_list' ? (
          // 조작은 AgentList 내부에서 agentClient/viewStore(단일 제어 표면)로 흐른다(§5).
          <AgentList />
        ) : (
          // ★순수 그림(ADR-0142)★: 표적은 슬롯 컨테이너다. 아이콘에 핸들러·tabIndex·role 을 되붙이면 컨테이너
          //   좌클릭과 겹쳐 메뉴가 두 번 열리고, 키보드로 못 빠져나오는 메뉴에 닿는 경로가 되살아난다.
          //   pointer-events 를 끊어 아이콘 위 클릭도 슬롯에 그대로 닿는다 — 유틸리티 클래스가 아니라 인라인인
          //   이유는 이 끊음이 스타일 취향이 아니라 동작 계약이라서다(클래스 규칙으로 덮이지 않고, Tailwind 를
          //   적용하지 않는 테스트 환경에서도 계산된 값으로 검증된다).
          <Plus className="size-11 text-muted" style={{ pointerEvents: 'none' }} />
        )}
        {contextMenu && (
          // ADR-0064: 통합 슬롯 메뉴 — buildSlotMenu(content.type) 로 (콘텐츠 전용 ∪ 공통 '*') command 참조를
          //   결정적 정렬·resolve 해 항목을 만들고, ctx(viewId/slotId/agentId)를 넘겨 각 command.run 이 백엔드
          //   권위 경로(viewStore/agentClient)로 흐르게 한다(§5 단일 제어 표면). content 종류가 가시성 게이트.
          <SlotContextMenu
            x={contextMenu.x}
            y={contextMenu.y}
            items={buildSlotMenu(node.content.type)}
            ctx={{ viewId: targetViewId, slotId: node.id, agentId: slotAgentId }}
            onClose={() => setContextMenu(null)}
          />
        )}
        {isFocused && (
          // ★강도 65%★: 너무 약한 포커스 표시가 반복 UX 불만이라(VS Code #24586 등, /research) "은은하되
          //   확실히 식별"되게 65%(사용자 결정). 세 테마 모두 color-mix 자동 적응. 제어 슬롯(트리/프리셋)은
          //   애초 focusSlot 제외(isContentSlot 게이트)라 isFocused=false → 링 없음(요구: 트리/프리셋 제외).
          <div
            style={{
              position: 'absolute',
              inset: 0,
              pointerEvents: 'none',
              boxShadow: 'inset 0 0 0 1px color-mix(in srgb, var(--accent) 65%, transparent)',
              zIndex: 10,
            }}
          />
        )}
      </div>
    )
  }
  // ★ADR-0140 유일한 진실 경계★: dir='top_bottom' = 위/아래 → allotment 의 vertical(수직 스택). 여기가
  //   뒤집히면 메뉴·타입·테스트가 전부 맞는데도 화면만 반대가 된다.
  // ★ratio 초기 사이징(ADR-0063)★: node.ratio = a(왼/위) 자식의 비율. ★Allotment 의 `defaultSizes` 는
  //   비율이 아니라 *픽셀*이다★ — [0.2,0.8] 을 주면 0.2px/0.8px 로 먹어 split-view 가 ~1px 로 붕괴하고
  //   자식들이 흐름 밖으로 쌓인다(실측 스샷으로 확인한 회귀). 대신 첫 pane(a=왼/위)에 `preferredSize` 를
  //   *퍼센트 문자열*로 줘 컨테이너 대비 비율로 배치한다(b 는 나머지 채움). 0.2 → "20%" = 20/80,
  //   0.5 → "50%" = 50/50. 컨테이너 실측 픽셀을 몰라도 되고 높이는 Allotment 가 컨테이너로 채운다.
  //   ★초기 사이징만★: 드래그 리사이즈→백엔드 ratio 되쓰기는 이 슬라이스 범위 밖(ADR-0063).
  // ★Allotment.Pane key = 위치 고정(pane-a/pane-b), 콘텐츠 파생 금지★: 옛 key 는 nodeKey(node.a) 로
  //   *서브트리 구조*에서 파생됐다 — 어느 pane 안의 슬롯이 split 으로 재구조화되면 그 pane 의 nodeKey 가
  //   바뀌어 React 가 Pane 을 unmount+remount 했고, Allotment 는 pane 이탈+합류로 보아 전 pane 을 균등
  //   재분배(형제의 비율 소실 — 예: 왼 20% → 50% 점프)했다. split 은 항상 a/b 두 자식을 이 순서로만
  //   가지므로 위치 기반 안정 key("pane-a"/"pane-b")를 쓴다 → 콘텐츠 재구조화에도 Pane 이 마운트 유지 →
  //   Allotment 가 사이즈를 보존한다. (형제 2개 사이에서만 유일하면 됨 — 중첩 Allotment 는 각자 짝을 가짐.)
  //   preferredSize(=ratio 파생 초기 사이징 %)는 첫 pane(a)에만 — 마운트 시 1회 적용·이후 보존(ADR-0063).
  return (
    <div style={{ height: '100%' }}>
      <Allotment vertical={node.dir === 'top_bottom'}>
        <Allotment.Pane key="pane-a" preferredSize={`${Math.round(node.ratio * 100)}%`}>
          <ViewLayoutRenderer node={node.a} focusedSlotId={focusedSlotId} viewIdOverride={viewIdOverride} />
        </Allotment.Pane>
        <Allotment.Pane key="pane-b">
          <ViewLayoutRenderer node={node.b} focusedSlotId={focusedSlotId} viewIdOverride={viewIdOverride} />
        </Allotment.Pane>
      </Allotment>
    </div>
  )
}
