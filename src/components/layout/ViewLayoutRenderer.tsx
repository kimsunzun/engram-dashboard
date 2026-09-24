// ★유일한 레이아웃 렌더러★(Brick 1): 옛 프론트 전용 slotStore/LayoutRenderer(number id + content union)는
// 제거됐다. 이 렌더러는 wire LayoutNode(string UUID id + content: SlotContent, ADR-0060, src-tauri/bindings)만 그린다 —
// 사람 클릭(SlotContextMenu — 우클릭 전용, ADR-0144)이든 LLM(window.__engramCmd)이든 같은 invoke→emit
// 권위 루프로 갱신된다.

import { Allotment } from 'allotment'

import type { LayoutNode } from '../../api/layoutTypes'
import LayoutLeaf from './LayoutLeaf'

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
  if (node.type === 'slot') {
    // key = slot id(ADR-0227): 이 자리에 다른 슬롯이 오면 잎을 새로 지어 슬롯별 기억·메뉴 상태가 딸려 가지 않는다.
    return (
      <LayoutLeaf
        key={node.id}
        node={node}
        focusedSlotId={focusedSlotId}
        viewIdOverride={viewIdOverride}
      />
    )
  }
  // ★ADR-0140 유일한 진실 경계★: dir='top_bottom' = 위/아래 → allotment 의 vertical(수직 스택). 여기가
  //   뒤집히면 메뉴·타입·테스트가 전부 맞는데도 화면만 반대가 된다.
  // ★ratio 초기 사이징(ADR-0063) = `defaultSizes`★: node.ratio = a(왼/위) 자식의 비율. allotment 는 defaultSizes 를
  //   합으로 나눠 비율로 정규화하고 칸마다 round(비율 × 컨테이너 크기)로 배치하므로 [ratio, 1-ratio] 를 그대로 준다.
  //   ★그 정규화는 `proportionalLayout` 기본값(true)에 기댄다★ — false 로 바꾸면 소수 값이 그대로 픽셀이 되어
  //   pane-a 가 minSize(30px)로 붕괴한다(리뷰 프로브 실측).
  //   ★빼지 말 것★: 이게 있어야 칸이 마운트 때 만들어져 첫 ResizeObserver 콜백 안에서 배치된다. 없으면 칸이 그
  //   콜백 뒤의 React 재렌더에서야 붙어, 새로 지은 split(닫기로 승격된 형제 포함)이 칸 크기 없는 한 프레임을
  //   그린다(쭈그러짐). 그 콜백이 새 split 의 첫 페인트보다 앞선다는 것은 조건부 정적 분석이고 WebView2 화면
  //   실측 전이다 — 조건·근거 = docs/research/split-layout-library-survey-2026-09-24.md F1·F2.
  //   ★「비율 defaultSizes 가 split-view 를 ~1px 로 붕괴시킨다」는 옛 실측(8dca0b9)은 원인 미상이다★ — 같은 커밋이
  //   allotment CSS import 누락도 함께 고쳐 둘이 따로 가려진 적이 없고, 정적 분석으로는 그 경로가 안 나온다(F2).
  //   다시 보이면 이 prop 이 첫 용의자다.
  //   pane-a 의 preferredSize(같은 ratio 를 정수 % 로)는 마운트 크기를 정하지 않는다 — allotment 는 그것을 새로 합류한
  //   pane 에만 먹이고 defaultSizes 로 지은 pane 은 합류로 치지 않는다. 남기는 이유 = sash 더블클릭 리셋이 pane-a 를
  //   이 값으로 되돌린다(없으면 50/50 균등 분배로 리셋된다). 정수 % 라 ratio 가 정수 % 가 아니면 마운트 크기와 리셋
  //   값이 조금 어긋난다(0.234 → 234/766 로 서고 230/770 으로 리셋) — 지금은 모든 split 이 0.5 로 태어나 안 보인다.
  //   ★초기 사이징만★: 드래그 리사이즈→백엔드 ratio 되쓰기는 이 슬라이스 범위 밖(ADR-0063).
  // ★Allotment.Pane key = 위치 고정(pane-a/pane-b), 콘텐츠 파생 금지★: 서브트리 구조에서 key 를 파생하면
  //   pane 안의 슬롯을 분할하는 순간 key 가 바뀌어 Pane 이 unmount+remount 되고, Allotment 는 이를 pane
  //   이탈+합류로 보아 전 pane 을 균등 재분배한다(형제의 비율 소실 — 예: 왼 20% → 50% 점프). split 은 항상
  //   a/b 두 자식을 이 순서로만 가지므로 위치 key 로 충분하다(중첩 Allotment 는 각자 짝을 가진다).
  // ★Allotment key = 이 split 의 id·dir★ (ADR-0223): allotment 는 방향과 defaultSizes 를 마운트 때 한 번만 읽는다
  //   (이후 `vertical` 은 CSS 클래스만 바꾼다) — 다른 split 에 옛 인스턴스를 쓰면 방향이 얼어 pane 이 0 으로
  //   접히거나(뷰가 빈 화면) 옛 split 의 드래그 픽셀 크기가 남는다.
  //   - id = split 노드의 정체(백엔드가 분할마다 새로 뽑고 형제 승격 때 노드와 함께 옮긴다). 닫기·팝업 분리로
  //     다른 split 이 이 자리에 올라오면 key 가 바뀌어 새로 짓고, 자식만 바뀐 같은 split 은 key 가 그대로라
  //     인스턴스(= 드래그한 크기)와 영향 없는 형제 서브트리가 남는다. 운영의 split 은 전부 ratio 0.5 로 태어나
  //     방향·비율로는 승격된 split 을 못 가른다.
  //   - dir 도 key 에 둔다 — 같은 split 의 방향이 제자리에서 바뀌어도 새로 지어야 한다.
  //   - ★ratio 는 key 에 넣지 않는다★: 드래그→백엔드 ratio 되쓰기가 생기면 드래그마다 모든 터미널이
  //     재마운트된다. 그 대가로 제자리 ratio 변경은 화면에 안 먹으므로(defaultSizes 는 마운트 때만 읽힌다)
  //     그 경로를 만들 땐 명령형 resize 가 필요하다.
  //   - ★첫 슬롯 id 로 key 를 파생하지 않는다★: 슬롯 하나를 닫으면 조상 split 의 key 가 바뀌어 영향 없는
  //     형제 서브트리까지 재마운트된다(터미널 재구독, ADR-0148 이 보존하던 죽은 에이전트 뷰 소실).
  return (
    <div style={{ height: '100%' }}>
      <Allotment
        key={`${node.id}:${node.dir}`}
        vertical={node.dir === 'top_bottom'}
        defaultSizes={[node.ratio, 1 - node.ratio]}
      >
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
