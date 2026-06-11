export function decodeBase64Bytes(b64: string): Uint8Array {
  // Uint8Array.fromBase64 신규 API 우선, 없으면 atob fallback
  const f = (Uint8Array as any).fromBase64
  if (f) return f(b64) as Uint8Array
  const bin = atob(b64)
  const arr = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i)
  return arr
}
