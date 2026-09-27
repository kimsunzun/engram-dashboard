// prints N lines; every `batch` lines sleeps `ms` milliseconds (so the flood lasts a while)
const [n = 3000, ms = 0, batch = 100] = process.argv.slice(2).map(Number)
const sleep = (t) => new Promise((r) => setTimeout(r, t))
;(async () => {
  for (let i = 1; i <= n; i++) {
    process.stdout.write(`flood line ${i} ${'x'.repeat(40)}\n`)
    if (ms && i % batch === 0) await sleep(ms)
  }
})()
