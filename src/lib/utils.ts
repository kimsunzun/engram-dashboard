// ADR-0047: Tailwind 클래스 병합 유틸.
// clsx = 조건부/배열 클래스 평탄화, tailwind-merge = 충돌하는 Tailwind 유틸(p-2 vs p-4 등) dedupe.
// shadcn 관례 위치(src/lib/utils.ts)에 둔다 — 이후 컴포넌트가 cn() 으로 클래스 조합.
import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs))
}
