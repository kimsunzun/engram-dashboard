// ADR-0061: 이름을 저장하지 않고 cwd basename 을 쓴다.
//
// ★단일 출처★: 프리셋 표시명(PresetPalette)과 에이전트 행 표시명(AgentList)이 같은 규칙으로 이름을
//   파생해야 한다. 각자 복제하면 win/posix·root 엣지 처리가 갈려 표시가 어긋나므로 여기 한 곳에 둔다.

/** blank 라벨 방지용 안정적 placeholder. */
export const PATH_NAME_PLACEHOLDER = '(경로 없음)'

/**
 * ★반환값은 절대 blank(빈/공백-only) 가 아니다★: 상위(라벨)는 이 값 하나로만 그리므로 blank 면 행이 빈
 *   칸으로 보인다.
 *
 * root·drive-root 처럼 basename 이 없거나 misleading 한 엣지는 파생하지 않고 raw cwd 로 degrade 한다
 *   ("C:" 로 collapse 하면 오해 소지).
 */
export function basename(cwd: string): string {
  if (!cwd || cwd.trim().length === 0) return PATH_NAME_PLACEHOLDER
  const trimmed = cwd.replace(/[\\/]+$/, '')
  if (trimmed.length === 0) return cwd
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  const base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed
  if (base.length === 0 || /^[A-Za-z]:$/.test(base)) return cwd
  return base
}
