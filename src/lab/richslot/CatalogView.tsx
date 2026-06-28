// 스타일 카탈로그 — 각 콘텐츠 스타일을 라벨 달아 한 개씩 구분해 보여준다(대화 흐름 X, "스타일 견본").
// 텍스트·마크다운·코드·diff·bash·thinking·에러 각 1개. code/diff 토글을 바꾸면 해당 스타일만
// 다시 그려져 "VS Code 스타일 vs 자체" 같은 차이를 곧장 본다. 무엇이 어떻게 그려지는지 한눈에.
//
// LAYOUTS 셀렉터에 끼우려고 messages 를 받는 시그니처만 맞추고 내용은 무시한다(고정 견본).

import type { RichMessage } from './types'
import { Markdown } from './MarkdownView'
import './layouts.css'

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="cat-item">
      <div className="cat-label">{label}</div>
      <div className="cat-body">{children}</div>
    </section>
  )
}

const MD_SAMPLE = `## 마크다운 제목
**굵게** · *기울임* · \`inline code\` 와 목록:
- 첫째 항목
- 둘째 항목

1. 순서 항목
2. 순서 항목

> 인용 블록`

const CODE_SAMPLE = '```python\ndef peek(self):\n    if not self._items:\n        raise IndexError("peek from empty")\n    return self._items[0]\n```'

const DIFF_SAMPLE =
  '```diff\n class Stack:\n     def __init__(self):\n-        self._items = []\n+        self._items = deque()\n```'

export function CatalogView(_props: { messages: RichMessage[] }) {
  return (
    <div className="lay-catalog">
      <Section label="텍스트 (plain)">
        <Markdown text="큐에 peek() 메서드를 추가했습니다. 단계별로 진행할게요." />
      </Section>
      <Section label="마크다운 (md)">
        <Markdown text={MD_SAMPLE} />
      </Section>
      <Section label="코드 (code)">
        <Markdown text={CODE_SAMPLE} />
      </Section>
      <Section label="diff">
        <Markdown text={DIFF_SAMPLE} />
      </Section>
      <Section label="bash (명령 + 출력)">
        <div className="cat-bash">
          <div className="cat-bash-cmd">$ git diff --stat</div>
          <pre className="cat-bash-out">{' stack.py | 4 +++-\n 1 file changed, 3 insertions(+), 1 deletion(-)'}</pre>
        </div>
      </Section>
      <Section label="thinking (생각)">
        <div className="cat-thinking">stack.py 를 읽고 peek 을 추가한 뒤 git diff 로 확인하자.</div>
      </Section>
      <Section label="에러 결과 (error)">
        <pre className="cat-error">NameError: name 'deque' is not defined{'\n'}1 failed in 0.04s</pre>
      </Section>
    </div>
  )
}
