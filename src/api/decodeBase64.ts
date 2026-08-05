export function decodeBase64Bytes(b64: string): Uint8Array {
  const f = (Uint8Array as any).fromBase64
  if (f) return f(b64) as Uint8Array
  const bin = atob(b64)
  const arr = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i)
  return arr
}
