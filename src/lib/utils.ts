// ADR-0047: shadcn 관례 위치(src/lib/utils.ts)에 둔다.
import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs))
}
