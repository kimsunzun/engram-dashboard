// command 인자 가방으로 들어온 enum 낱말을 **선언된 철자**로 접는 한 곳.
//
// ★대소문자만 접는다 — 유효 집합은 넓히지 않는다★: 호출자는 아무 대소문자로 써도 되지만, 여기를 지나
//   나가는 값은 언제나 `declared` 에 적힌 철자 그대로다. 모르는 낱말은 여전히 `undefined` 로 떨어져
//   호출부가 원래 하던 거절을 그대로 한다(거절 문구·조건은 각 호출부 소관 — 이 모듈은 판정만 준다).

/**
 * `declared` 중 `raw` 와 대소문자 무시로 같은 항목을 돌려준다. 못 찾거나 문자열이 아니면 `undefined`.
 *
 * ★반환은 항상 `declared` 의 원소다 — 호출자가 준 문자열이 아니다★. 백엔드로 나가는 wire 값이 선언
 * 철자와 바이트 단위로 같아야 하므로, 이 반환을 그대로 실어 보내면 된다.
 *
 * ★완전 일치를 먼저 본다★ — `declared` 에 대소문자만 다른 두 철자가 들어오는 날에도 정확히 쓴 호출자는
 * 자기가 쓴 철자를 받는다(그 경우 나머지 해소는 선언 순서로 결정되며, 그런 집합은 애초에 만들지 않는다).
 *
 * 접기는 `toLowerCase`(로케일 비의존)로 한다 — `toLocaleLowerCase` 는 터키어 로케일에서 `I` 를 `ı` 로
 * 접어 같은 낱말이 기계마다 다르게 판정된다.
 */
export function matchDeclaredSpelling<T extends string>(
  raw: unknown,
  declared: readonly T[],
): T | undefined {
  if (typeof raw !== 'string') return undefined
  if (declared.includes(raw as T)) return raw as T
  const folded = raw.toLowerCase()
  return declared.find(d => d.toLowerCase() === folded)
}
