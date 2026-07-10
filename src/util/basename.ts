// 공용 basename 파생 — cwd(작업 디렉토리)의 마지막 경로 세그먼트(ADR-0061 이름 미저장 정책).
//
// ★단일 출처★: 프리셋 표시명(PresetPalette)과 에이전트 행 표시명(AgentList)이 같은 규칙으로 이름을
//   파생해야 한다(둘 다 "이름을 저장하지 않고 cwd basename 을 쓴다"). 각자 복제하면 win/posix·root
//   엣지 처리가 갈려 표시가 어긋나므로 여기 한 곳에 둔다.
//
// Windows(`\`)·POSIX(`/`) 구분자를 모두 다루고 후행 구분자는 무시한다(예: "C:/proj/" → "proj").

/** cwd 가 비었거나 파생할 세그먼트가 없을 때의 안정적 placeholder(blank 라벨 방지). */
export const PATH_NAME_PLACEHOLDER = '(경로 없음)'

/**
 * cwd 의 basename(마지막 경로 세그먼트)을 파생한다.
 *
 * ★반환값은 절대 blank(빈/공백-only) 가 아니다★: 상위(라벨)는 이 값 하나로만 그리므로 blank 면 행이 빈
 *   칸으로 보인다. 파생할 basename 도 raw cwd 도 없을 때는 placeholder 로 degrade 한다.
 *
 * 엣지 케이스(basename 이 없거나 misleading 할 때)는 파생하지 않고 raw cwd 로 degrade 한다:
 *   - 빈/공백-only 문자열 → "(경로 없음)" placeholder
 *   - drive-root "C:\\" / "C:/" → 원본 유지("C:" 로 collapse 하면 오해 소지)
 *   - posix root "/" · UNC "\\\\server\\share" 처럼 후행 구분자 제거 후 세그먼트가 없거나 원본과
 *     같아지는 경우 → raw cwd 반환(잘못된 세그먼트로 붕괴 방지).
 */
export function basename(cwd: string): string {
  // 빈/공백-only cwd: 파생할 basename 도 raw cwd 도 없음 → blank 라벨 대신 안정적 placeholder.
  if (!cwd || cwd.trim().length === 0) return PATH_NAME_PLACEHOLDER
  const trimmed = cwd.replace(/[\\/]+$/, '') // 후행 구분자 제거
  // 후행 구분자만 있던 root-like 경로("/", "C:\\", "\\\\srv\\share\\") → trim 후 비거나
  //   drive-only 로 남음. 이럴 땐 잘못된 세그먼트로 붕괴시키지 말고 raw cwd 로 degrade.
  if (trimmed.length === 0) return cwd
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  const base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed
  // base 가 비면(root 직후) 또는 drive-root("C:") 면 misleading — raw cwd 로 fallback.
  if (base.length === 0 || /^[A-Za-z]:$/.test(base)) return cwd
  return base
}
