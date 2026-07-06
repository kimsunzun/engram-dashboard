// ADR-0050: 구조화 채팅 렌더 dispatch — 우리 자체 채팅 leaf 컴포넌트(chat/*)에 우리 데이터 모델
//   (StructuredItem 스트림)을 먹인다. 벤치마크 룩 = Claude Code VSCode 확장(1차 근사치, 사용자가
//   스크린샷으로 후속 조정). 이전 라운드에서 도입했던 외부(Apache-2.0) 이식물은 전부 제거하고 자체
//   구현으로 대체했다. leaf: ./chat/{Markdown,ThoughtRow,CopyButton}.
//
// ★이 파일의 책임★: items 스트림을 종류별로 위 leaf 컴포넌트로 dispatch 하는 **순수 렌더**(구독/누적은
//   RichSlot 소관). props = { items, streaming } 만 받는다.
//
// ★레이아웃★: 각 행은 ChatRow 래퍼(relative pt-2.5 px-4)를 쓴다. assistant-side 행(text·thinking·tool·
//   generic·error + streaming tail)은 rail 모드 — 좌측 고정폭 gutter 에 muted 점 마커를 두고 콘텐츠를
//   그 오른쪽으로 들여쓴다(Claude Code VSCode 확장의 thread 룩 1차 근사치 — thinking+응답+도구를 한
//   가닥으로 묶는다). user 발화(확장 룩 버블 rounded-md border bg-elevated)와 separator 는 rail 없이 plain.
//   헤더는 작은 lucide 아이콘 + bold 제목. 도구/에러/generic 은 이 헤더 패턴, assistant text 는 헤더 없이
//   Markdown full-width, thinking 은 ThoughtRow.
//
// ★안전 파서 헬퍼(pretty/extractText/contentToText/parseToolResult/buildToolResultMap/shortArgs)★는
//   우리 데이터-어댑터 로직이며 **절대 throw 하지 않는다**(bad json 폴백).

import { useState, type ComponentType, type ReactNode } from 'react'
import {
  AlertTriangle,
  Braces,
  ChevronDown,
  ChevronRight,
  FileCode2,
  FileMinus2,
  FilePlus2,
  FolderOpen,
  Globe,
  List,
  Pencil,
  Search,
  SquareTerminal,
  Wrench,
} from 'lucide-react'

import { cn } from '@/lib/utils'
import type { StructuredItem } from './structuredAccumulator'
// ADR-0050: 우리 자체 채팅 leaf 들(chat/*). 상세는 각 파일 헤더 참조.
//   ★Markdown(전체 마크다운) 은 assistant text 에만 쓴다. 도구 IN/OUT·탈출구 json 은 신뢰할 수 없는
//   텍스트라 마크다운 파싱을 태우지 않고 InertCode(리터럴 <pre>)로만 그린다(FIX 2 — 아래 주석).
import { Markdown } from './chat/Markdown'
import { ThoughtRow } from './chat/ThoughtRow'

// ── 안전 파서 헬퍼(절대 throw 금지 — bad json 폴백) ────────────────────────────────

/** args_json/json 을 읽기 좋게 pretty-print(파싱 실패 시 원문 그대로). 절대 throw 하지 않는다. */
function pretty(json: string): string {
  try {
    return JSON.stringify(JSON.parse(json), null, 2)
  } catch {
    return json
  }
}

/**
 * json 문자열에서 텍스트를 추출한다. 절대 throw 하지 않는다.
 * - 'user': `.text` 우선, 없으면 `.thinking` · 'thinking': `.thinking` 우선, 없으면 `.text`.
 * - 실패/필드 부재 시 raw json 문자열.
 */
function extractText(json: string, mode: 'thinking' | 'user'): string {
  try {
    const parsed: unknown = JSON.parse(json)
    if (typeof parsed === 'string') return parsed
    if (parsed !== null && typeof parsed === 'object') {
      const obj = parsed as Record<string, unknown>
      if (mode === 'user') {
        if (typeof obj['text'] === 'string') return obj['text']
        if (typeof obj['thinking'] === 'string') return obj['thinking']
      } else {
        if (typeof obj['thinking'] === 'string') return obj['thinking']
        if (typeof obj['text'] === 'string') return obj['text']
      }
    }
    return json
  } catch {
    return json
  }
}

/**
 * Anthropic content 블록(문자열 | 블록 배열)에서 표시용 텍스트를 뽑는다. 절대 throw 하지 않는다.
 * content 가 문자열이면 그대로, 배열이면 `type === "text"` 블록의 `.text` 만 이어붙인다.
 */
function contentToText(content: unknown): string {
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    const parts: string[] = []
    for (const block of content) {
      if (typeof block === 'string') {
        parts.push(block)
      } else if (block !== null && typeof block === 'object') {
        const b = block as Record<string, unknown>
        if (b['type'] === 'text' && typeof b['text'] === 'string') parts.push(b['text'])
      }
    }
    return parts.join('\n')
  }
  if (content !== null && typeof content === 'object') {
    const b = content as Record<string, unknown>
    if (b['type'] === 'text' && typeof b['text'] === 'string') return b['text']
  }
  return ''
}

/** tool_result 페어(도구 결과 본문 + 에러 여부). tool_use_id 로 도구 호출과 짝짓는다. */
type ToolResult = { content: string; isError: boolean }

/**
 * structured item 의 json 이 tool_result 면 { toolUseId, result } 를, 아니면 null. 절대 throw 하지 않는다.
 */
function parseToolResult(json: string): { toolUseId: string; result: ToolResult } | null {
  try {
    const parsed: unknown = JSON.parse(json)
    if (parsed === null || typeof parsed !== 'object') return null
    const obj = parsed as Record<string, unknown>
    if (obj['type'] !== 'tool_result') return null
    const toolUseId = typeof obj['tool_use_id'] === 'string' ? obj['tool_use_id'] : ''
    if (!toolUseId) return null
    return {
      toolUseId,
      result: { content: contentToText(obj['content']), isError: obj['is_error'] === true },
    }
  } catch {
    return null
  }
}

/**
 * items 를 한 번 훑어 tool_use_id → tool_result 맵을 만든다(pre-scan). 절대 throw 하지 않는다.
 * 도구 호출(tool item)은 이 맵에서 자기 id 로 결과(OUT)를 찾아 함께 그린다.
 * 같은 tool_use_id 가 중복되면 last-write-wins(Map.set) — tool_use id 는 Anthropic 이 고유 보장하고
 * 상류(누산기)가 seq dedup 하므로 실전 중복은 없다. 있어도 마지막 결과로 덮는 것이 안전한 폴백.
 */
function buildToolResultMap(items: StructuredItem[]): Map<string, ToolResult> {
  const map = new Map<string, ToolResult>()
  for (const item of items) {
    if (item.kind !== 'structured') continue
    const hit = parseToolResult(item.json)
    if (hit) map.set(hit.toolUseId, hit.result)
  }
  return map
}

/** tool args JSON 의 첫 문자열 값을 잘라 1줄 힌트(무슨 파일/명령인지). 절대 throw 하지 않는다. */
function shortArgs(argsJson: string): string {
  try {
    const parsed: unknown = JSON.parse(argsJson)
    if (parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)) {
      const obj = parsed as Record<string, unknown>
      for (const val of Object.values(obj)) {
        if (typeof val === 'string' && val.length > 0) {
          return val.length > 64 ? val.slice(0, 64) + '…' : val
        }
      }
    }
    return ''
  } catch {
    return ''
  }
}

// ── 채팅 룩 프리미티브 ────────────────────────────────────────────────────────────

/**
 * 메시지 행의 바깥 컨테이너 — `relative pt-2.5 px-4`(세로 리듬 유지). StructuredTextView 컨테이너가
 * 이 행들을 그대로 담는다.
 *
 * rail 모드(기본 false — user 버블·하위호환): assistant-side 행에 좌측 thread 구조를 준다.
 *   flex 행 = [고정폭 gutter + 점 마커] + [콘텐츠 flex-1 min-w-0]. 점(dot)은 muted 원으로 콘텐츠 첫
 *   줄에 맞춰 상단 정렬(pt-2.5 + mt-[3px]). 콘텐츠 컬럼은 min-w-0 라 긴 토큰/wrap-anywhere 가 넘치지 않는다.
 *   1차 근사치라 dot+indent 골격만 — 점을 잇는 세로 thread 선은 후속 시각 조정에서.
 */
function ChatRow({ children, rail = false }: { children: ReactNode; rail?: boolean }) {
  if (rail) {
    return (
      <div className="relative pt-2.5 px-4 flex">
        {/* gutter — 고정폭. 점 마커는 콘텐츠 첫 줄 baseline 근처에 맞춘다(mt 미세조정).
            aria-hidden 은 점(span)에만 — gutter div 에 얹으면 separator 스페이서(div[aria-hidden])와
            셀렉터가 충돌한다. 점은 순수 장식이라 접근성 트리에서 뺀다. */}
        <div className="w-6 flex-none flex justify-center">
          <span aria-hidden className="mt-[3px] size-1.5 rounded-full bg-muted" />
        </div>
        {/* 콘텐츠 컬럼 — flex-1 min-w-0 로 긴 토큰/wrap-anywhere 오버플로 방지. */}
        <div className="flex-1 min-w-0">{children}</div>
      </div>
    )
  }
  return <div className="relative pt-2.5 px-4">{children}</div>
}

/** 행 헤더 클래스 — 작은 아이콘 + bold 제목. */
const HEADER_CLASSNAMES = 'flex items-center gap-2.5 mb-3'

type LucideIcon = ComponentType<{ className?: string }>

/**
 * 도구 헤더 아이콘 휴리스틱 — 우리 tool item 은 generic(name 만) 이라 도구 종류 판별자가 없다.
 * name 을 소문자로 보고 흔한 CC/claude 도구를 대응 아이콘(lucide)에 매핑한다. 미스는 Wrench 폴백.
 * (Pencil=edit, FilePlus2=create, FileMinus2=delete, FileCode2=read, FolderOpen=list, Search=search,
 *  SquareTerminal=bash, Globe=web/fetch.)
 */
function toolIconFor(name: string): LucideIcon {
  const n = name.toLowerCase()
  if (n.includes('multiedit') || n.includes('edit') || n.includes('write') || n.includes('replace'))
    return Pencil
  if (n.includes('create') || n.includes('new')) return FilePlus2
  if (n.includes('delete') || n.includes('remove') || n.includes('rm')) return FileMinus2
  if (n.includes('read') || n.includes('cat') || n.includes('view')) return FileCode2
  if (n.includes('glob') || n.includes('ls') || n.includes('list') || n.includes('dir'))
    return FolderOpen
  if (n.includes('grep') || n.includes('search') || n.includes('find')) return Search
  if (n.includes('bash') || n.includes('shell') || n.includes('exec') || n.includes('command'))
    return SquareTerminal
  if (n.includes('web') || n.includes('fetch') || n.includes('http') || n.includes('url'))
    return Globe
  if (n.includes('todo') || n.includes('task') || n.includes('plan')) return List
  return Wrench
}

/**
 * 헤더 행 — 작은 아이콘 + bold 제목(semantic color). 도구/에러/generic 에 공통.
 * 아이콘은 size-3.5(≈14px)로 우리 폰트 스케일에 맞춘다.
 */
function RowHeader({
  icon: Icon,
  title,
  tone = 'default',
}: {
  icon: LucideIcon
  title: ReactNode
  tone?: 'default' | 'error'
}) {
  return (
    <div className={HEADER_CLASSNAMES}>
      <Icon className={cn('size-3.5 flex-none', tone === 'error' ? 'text-red-500' : 'text-foreground')} />
      <span className={cn('font-bold', tone === 'error' ? 'text-red-500' : 'text-foreground')}>
        {title}
      </span>
    </div>
  )
}

// ── 이식 컴포넌트 어댑터 행 ──────────────────────────────────────────────────────────

/**
 * ★FIX 2 (fenced-code escape 방어)★: 도구 IN(args)/OUT(result)·탈출구 json 은 신뢰할 수 없는 텍스트다.
 *   이 콘텐츠를 마크다운 렌더러에 먹이면 내용에 삼중 백틱(```) 줄이 있을 때 펜스가 조기 종료돼 나머지가
 *   마크다운으로 파싱된다(활성 링크/이미지·heading 주입). 그래서 이 콘텐츠는 마크다운을 **절대 태우지 않고**
 *   리터럴 <pre><code> 로만 그린다 — React 텍스트 자식은 자동 이스케이프되므로 삼중 백틱·`# heading` 이
 *   있어도 태그로 승격되지 않는다(inert). 전체 마크다운은 assistant text(Markdown) 에만 허용.
 */
function InertCode({ code }: { code: string }) {
  return (
    <pre className="overflow-x-auto rounded-xs border border-border bg-surface px-2.5 py-2 text-xs">
      <code className="whitespace-pre-wrap break-words font-mono text-foreground">{code}</code>
    </pre>
  )
}

/**
 * 도구 호출 행 — 헤더(작은 아이콘 + bold 이름) + bg-surface rounded-sm border 박스.
 * 박스의 클릭 sub-header 를 눌러 IN(args)/OUT(result) 본문을 펼친다. IN/OUT 은 신뢰할 수 없는 텍스트이므로
 * InertCode(리터럴 <pre>)로만 렌더 — 마크다운 파싱 금지(FIX 2). 로컬 open state(itemId key 로 스트리밍
 * 리렌더 중에도 유지).
 */
function ToolItemRow({
  name,
  argsJson,
  result,
}: {
  name: string
  argsJson: string
  result: ToolResult | null
}) {
  const [open, setOpen] = useState(false)
  const hint = shortArgs(argsJson)
  const isErr = result?.isError === true
  const Icon = toolIconFor(name)
  return (
    <div>
      <RowHeader icon={Icon} title={name} tone={isErr ? 'error' : 'default'} />
      <div
        className={cn(
          'bg-surface rounded-sm overflow-hidden border',
          isErr ? 'border-red-500/60' : 'border-border',
        )}
      >
        {/* 클릭 sub-header — 펼침 토글(text-muted).
            aria-label 에 도구명을 실어 접근성 이름을 헤더와 일치시킨다(sub-header 텍스트는 인자 힌트라
            도구명이 없으므로, 스크린리더/테스트가 "어느 도구의 세부인지" 식별하게 name 을 명시). */}
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-label={name}
          className="flex w-full items-center gap-2 cursor-pointer select-none py-2 px-2.5 text-left text-muted"
        >
          {open ? (
            <ChevronDown className="size-3.5 flex-none" />
          ) : (
            <ChevronRight className="size-3.5 flex-none" />
          )}
          {hint ? (
            <span className="truncate font-mono text-xs">{hint}</span>
          ) : (
            <span className="truncate font-mono text-xs opacity-70">arguments</span>
          )}
          {isErr && (
            <span className="ml-auto flex-none rounded border border-red-500 px-1.5 text-[10px] text-red-500">
              Error
            </span>
          )}
        </button>
        {open && (
          <div className="space-y-2 border-t border-border px-2.5 py-2">
            <div>
              <div className="mb-1 text-[10px] uppercase tracking-wide text-muted">In</div>
              {/* args JSON 을 리터럴 <pre> 로 — 마크다운 파싱 없이 원문 그대로(FIX 2). */}
              <InertCode code={pretty(argsJson)} />
            </div>
            {result && (
              <div>
                <div
                  className={cn(
                    'mb-1 text-[10px] uppercase tracking-wide',
                    isErr ? 'text-red-500' : 'text-muted',
                  )}
                >
                  Out
                </div>
                {/* 결과 본문을 리터럴 <pre> 로 — 삼중 백틱이 있어도 inert(FIX 2). */}
                <InertCode code={result.content || '(빈 결과)'} />
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

/**
 * 탈출구(알 수 없는 label) 이벤트 — 도구 박스와 동형인 접힘 raw json 블록. label 을 muted 헤더로 얹고
 * (Braces 아이콘) bg-surface border 박스를 펼치면 json 을 InertCode(리터럴, FIX 2)로 그린다.
 */
function GenericItemRow({ label, json }: { label: string; json: string }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="bg-surface rounded-sm overflow-hidden border border-border">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 cursor-pointer select-none py-2 px-2.5 text-left text-muted"
      >
        {open ? (
          <ChevronDown className="size-3.5 flex-none" />
        ) : (
          <ChevronRight className="size-3.5 flex-none" />
        )}
        <Braces className="size-3.5 flex-none" />
        <span className="truncate font-mono text-xs">{label}</span>
      </button>
      {open && (
        <div className="border-t border-border px-2.5 py-2">
          {/* raw json 을 리터럴 <pre> 로 — 마크다운 파싱 없이 원문 그대로(FIX 2). */}
          <InertCode code={pretty(json)} />
        </div>
      )}
    </div>
  )
}

// ── 항목 dispatch ───────────────────────────────────────────────────────────────────

/** 한 item 렌더. key 는 여기서 부여(itemId). standalone tool_result 는 null 로 제외. */
function renderItem(item: StructuredItem, results: Map<string, ToolResult>): ReactNode {
  const k = item.itemId
  switch (item.kind) {
    case 'text':
      // assistant 본문 — 우리 자체 Markdown(react-markdown + remark/rehype). 헤더 없이 full-width.
      //   긴 토큰(URL·경로)이 컨테이너를 넘지 않게 행 컨테이너에 wrap-anywhere overflow-hidden.
      return (
        <ChatRow key={k} rail>
          <div className="wrap-anywhere overflow-hidden">
            <Markdown markdown={item.text} />
          </div>
        </ChatRow>
      )

    case 'structured':
      // ★FIX 1 (tool_result 흡수 — label 무관)★: json.type==='tool_result' 인 structured 는 label 이
      //   무엇이든(user 든 아니든) 매칭 도구의 OUT 에 흡수되므로 독립 렌더하지 않는다. 이 검사는 label
      //   분기보다 **먼저** 와야 한다 — 이전엔 user 분기 안에만 있어 다른 label 의 tool_result 가 standalone
      //   으로 새 나갔다(계약 위반). 매칭 tool 이 없어도 흡수 규칙은 동일(어디에도 안 그린다).
      if (parseToolResult(item.json)) return null

      if (item.label === 'user') {
        // 사용자 발화 — 확장 룩 버블(rounded-md border bg-elevated). whitespace-pre-line 으로 줄바꿈 보존.
        return (
          <ChatRow key={k}>
            <div className="rounded-md border border-border bg-elevated px-3 py-2 my-1 whitespace-pre-line break-words text-foreground">
              {extractText(item.json, 'user')}
            </div>
          </ChatRow>
        )
      }
      if (item.label === 'thinking') {
        // 추론 행 — ThoughtRow. 내용이 있으면 펼침 가능, 비어 있으면(opus 암호화 thinking) 비-인터랙티브
        //   "Thought" 라벨만 렌더해 "추론이 있었다"는 존재를 노출한다(빈 행을 걸러내지 않는다).
        const content = extractText(item.json, 'thinking')
        return (
          <ChatRow key={k} rail>
            <ThoughtRow content={content} />
          </ChatRow>
        )
      }
      // 기타 label(탈출구) — 접힘 generic 블록.
      return (
        <ChatRow key={k} rail>
          <GenericItemRow label={item.label} json={item.json} />
        </ChatRow>
      )

    case 'tool': {
      const result = item.id ? results.get(item.id) ?? null : null
      return (
        <ChatRow key={k} rail>
          <ToolItemRow name={item.name} argsJson={item.argsJson} result={result} />
        </ChatRow>
      )
    }

    case 'usage':
      // 토큰 사용량 — 메시지별 토큰 칩은 표시하지 않는다. 렌더 안 함.
      //   (누적 item 종류 자체는 유지 — 여기서 렌더만 생략.)
      return null

    case 'error':
      // 에러 — 헤더 패턴(에러 아이콘 + bold "Error", text-red-500) + 메시지 본문.
      return (
        <ChatRow key={k} rail>
          <RowHeader icon={AlertTriangle} title="Error" tone="error" />
          <div className="text-[13px] text-red-500 whitespace-pre-wrap break-words">
            {item.message}
          </div>
        </ChatRow>
      )

    case 'separator':
      // 턴 경계 — 점선 레일/구분선 없이 아주 옅은 세로 스페이서만(눈에 띄는 divider 지양).
      return <div key={k} aria-hidden className="h-3" />
  }
}

/**
 * ADR-0050: 구조화 채팅 렌더 — 세로 스택으로 항목별 dispatch. assistant-side 행(text·thinking·tool·
 * generic·error + streaming tail)은 좌측 dot-rail gutter 로 한 thread 처럼 묶고, user 버블·separator 는
 * rail 없이 plain.
 * items 를 한 번 pre-scan 해 tool_use_id → tool_result 맵을 만들고(도구 OUT 흡수), standalone tool_result
 * 는 제외. streaming(턴 활성)이면 스트림 끝에 ThoughtRow(pulse "Thinking…")를 붙인다.
 * 순수 렌더(props in, DOM out).
 */
export function StructuredTextView({
  items,
  streaming = false,
}: {
  items: StructuredItem[]
  streaming?: boolean
}) {
  const results = buildToolResultMap(items)
  // 스트리밍 라이브 신호는 콘텐츠가 있을 때만(빈 슬롯에 Thinking 신호 뜨는 오작동 방지 — 지시 명세).
  const hasContent = items.length > 0
  return (
    // text-[13px] leading-[1.45] — 채팅 루트 폰트/줄간격을 여기에만 스코프한다(트리·터미널 슬롯 등 앱
    //   나머지는 영향 없음).
    <div className="flex flex-col pb-3 font-sans text-foreground text-[13px] leading-[1.45]">
      {items.map((item) => renderItem(item, results))}
      {streaming && hasContent && (
        // ★streaming 라이브 신호★ — ThoughtRow streaming(pulse "Thinking…" 라벨). 스트림 끝에서만.
        //   ★FIX 3★: 안정 key — 없으면 streaming 토글 시 직전 실 item 이 이 행과 자리 매칭돼 remount 되며
        //   로컬 expand state 를 잃는다. 리스트 밖 고정 노드라 상수 key 로 정체성을 못박는다.
        <ChatRow key="__streaming__" rail>
          <ThoughtRow streaming label="Thinking…" />
        </ChatRow>
      )}
    </div>
  )
}
