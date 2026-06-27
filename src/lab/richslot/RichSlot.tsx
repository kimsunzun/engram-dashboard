// RichSlot 렌더층 — 메시지 목록 → 구조화 뷰 (스파이크 골격).
//
// 입력은 RichMessage[] 인터페이스만. 소스가 mock fixture(랩)든 실제 데몬 스트림(통합 후)든
// 같은 형태면 그대로 동작한다(ADR-0012 격리 — 외부 의존을 seam 으로 끊음).

import type { RichMessage } from './types'
import { renderBlock } from './blocks'
import './richslot.css'

export function RichSlot({ messages }: { messages: RichMessage[] }) {
  return (
    <div className="rich-slot">
      {messages.map((m, i) => (
        <div key={i} className={`rs-msg rs-msg-${m.role}`}>
          {m.blocks.map((b, j) => renderBlock(b, j))}
        </div>
      ))}
    </div>
  )
}
