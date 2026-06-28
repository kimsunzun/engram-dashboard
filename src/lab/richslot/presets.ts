// 프리젠테이션 프리셋 — 이름붙은 표현 번들. 사용자(또는 LLM)가 통째로 swap 한다.
//
// ★§5 LLM-제어 표면★: 표현(레이아웃·테마·폰트크기)을 "프리셋 이름" 단위로 묶어,
// applyPreset(container, name) 하나로 바꾼다. CSS 변수만 만지므로 re-parse·remount 없이
// 브라우저가 즉시 reflow — 사람 클릭과 LLM 호출이 같은 핸들(applyPreset)을 흔든다.
//
// ★pure-DOM★: React 밖에서도 부를 수 있게 DOM 만 만진다(CDP/command 로 직접 구동 가능).

import type { LayoutKey } from './layouts'

/** 표현 모드 — color(일반) / eink(전자잉크 흑백, 현재 stub). */
export type PresetMode = 'color' | 'eink'

/** 프리셋 = {레이아웃, 모드, 폰트크기}. 필요시 자연스럽게 확장(지금은 최소). */
export interface Preset {
  layout: LayoutKey
  mode: PresetMode
  fontSize: number // px — --lay-font-size 로 주입
}

/** 프리셋 레지스트리 — 이름 → Preset. 사람·LLM 이 이름으로 고른다. */
export const PRESETS: Record<string, Preset> = {
  // 주력 — 대화형 + 컬러 + 14px. 열자마자 보이는 가독 결과.
  'color-chat': { layout: 'chat', mode: 'color', fontSize: 14 },
  // e-ink stub — data-theme 만 eink 로 박고, 실제 흑백 스타일링은 후속(지금 빌드 X).
  'eink-chat': { layout: 'chat', mode: 'eink', fontSize: 15 },
  // 기존 레이아웃 매핑 — 비교/대체 표현.
  'review-card': { layout: 'card', mode: 'color', fontSize: 13 },
  terminal: { layout: 'tlog', mode: 'color', fontSize: 12.5 },
}

export type PresetName = keyof typeof PRESETS

/**
 * 프리셋을 컨테이너에 적용 — data-theme(모드) + CSS 변수(--lay-font-size) 세팅.
 * 단일 핸들: 사람 UI 와 LLM 이 똑같이 부른다(§5). 선택된 layout key 를 반환해
 * 호출부(React)가 어떤 레이아웃 컴포넌트를 그릴지 동기화한다.
 * 알 수 없는 이름이면 null 반환(호출부가 무시 — robust).
 */
export function applyPreset(container: HTMLElement, name: string): LayoutKey | null {
  const preset = PRESETS[name]
  if (!preset) return null
  // data-theme: color 는 명시 안 함(기본=현 컬러 팔레트), eink 만 박는다(후속 스타일 훅).
  container.dataset.theme = preset.mode === 'eink' ? 'eink' : 'color'
  container.style.setProperty('--lay-font-size', `${preset.fontSize}px`)
  return preset.layout
}
