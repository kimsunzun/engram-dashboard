// 측정값 보고의 보내는 쪽 도우미 — 「한 번에 하나 · 같은 값 안 보냄 · 디바운스」(ADR-0227 · TRD S21-slot-docking §5-3). 창 캔버스
//   크기(`windowCanvasReport.ts`)가 쓴다. 셸에 남는 값 = 마지막으로 성공한 보고다.
// ★한 보고기의 invoke 는 한 번에 하나만 날린다★: 겹친 두 요청을 셸이 도착 역순으로 처리하면 셸의 값이 다음 보고까지 낡은 채
//   남는다. WebView2 가 겹친 요청을 JS 발행 순서대로 올리는지는 문서에 없다(모름). 하나씩이면 응답 순서가 곧 셸 적용 순서라,
//   취소된 작업의 성공도 셸의 지금 값으로 믿을 수 있다. 대가: 응답이 끝내 안 오면 그 보고기는 거기서 멈춘다.
// 날아가는 동안 온 값은 대기 칸 하나에 두고 새 값이 덮는다 — 응답 뒤 그 마지막 값만 보낸다.
// 실패는 스스로 유계 재시도(`retryAsync`)한다 — 측정은 바뀔 때만 오므로 다음 측정을 기다리면 영영 안 갈 수 있다. 그 사이 다른
//   값이 오면 옛 값의 재시도를 그친다.

import { retryAsync, RetryCancelledError } from './retryInvoke'

/** 측정 보고의 디바운스(ms) — 관측이 잦은 보고(캔버스 크기 · 창 틀)가 함께 쓴다. */
export const REPORT_DEBOUNCE_MS = 100

export interface LatestValueReporterOptions<T> {
  /** 셸에 보낸다 — 거절은 던진다. */
  send: (value: T) => Promise<unknown>
  same: (a: T, b: T) => boolean
  /** 로그 머리(`[log]`). */
  log: string
  /** 로그에 실을 보고 이름 — 예: `report_window_canvas(800x600)`. */
  describe: (value: T) => string
  /**
   * 셸이 `lastSent` 와 다른 값을 성공 응답해 `lastSent` 가 바뀐 뒤 부른다 — 실패 · 같은 값의 재성공에는 부르지 않는다.
   * ★던지지 말 것★ — 응답 처리 안에서 불려, 던지면 보고 실패로 셈해 같은 값을 다시 보낸다.
   */
  onAccepted?: (value: T) => void
}

export class LatestValueReporter<T extends object> {
  private readonly opts: LatestValueReporterOptions<T>
  /** 보낼 차례를 기다리는 값. 하나만 두고 새 값이 덮는다. */
  private pending: T | null = null
  private sent: T | null = null
  /** 지금 보내는 작업(재시도 대기 포함). 취소되면 null. */
  private job: { value: T; gen: number } | null = null
  /** invoke 가 응답을 기다리는 중인가 — 재시도의 backoff 대기와는 따로 센다. */
  private inAir = false
  /** 작업 세대 — 바뀌면 그 전 작업의 재시도는 더 시도하지 않는다. */
  private gen = 0
  private timer: ReturnType<typeof setTimeout> | null = null

  constructor(opts: LatestValueReporterOptions<T>) {
    this.opts = opts
  }

  /** 셸이 성공 응답한 마지막 값 — 같은 값의 재성공은 참조를 갈지 않는다. */
  get lastSent(): T | null {
    return this.sent
  }

  /**
   * 디바운스 전의 관측 하나 — 보내는 값 · 대기 값이 이 값과 다르면 디바운스를 기다리지 않고 거둔다. 기다리면 그 사이 옛 재시도나
   * 응답 뒤 차례가 낡은 값을 한 번 더 보낸다.
   */
  observe(value: T): void {
    if (this.job && !this.opts.same(this.job.value, value)) this.cancelJob()
    if (this.pending !== null && !this.opts.same(this.pending, value)) this.pending = null
  }

  /** 곧바로 보낼 차례에 올린다(디바운스 없음). `lastSent` 와 같으면 보내지 않는다. */
  submit(value: T): void {
    this.observe(value)
    this.pending = value
    this.pump()
  }

  /**
   * `delayMs` 뒤 `measure` 를 불러 그 값을 `submit` 한다 — 그 사이 다시 부르면 타이머를 다시 잡고 마지막 `measure` 만 쓴다.
   * 재는 것을 타이머 끝으로 미루므로 그때의 DOM 을 잰다. `measure` 가 `null` 을 주면 보내지 않는다.
   */
  schedule(measure: () => T | null, delayMs: number = REPORT_DEBOUNCE_MS): void {
    if (this.timer !== null) clearTimeout(this.timer)
    this.timer = setTimeout(() => {
      this.timer = null
      const value = measure()
      if (value !== null) this.submit(value)
    }, delayMs)
  }

  /**
   * 관측을 뗄 때 — 디바운스 · 대기 값을 버리고 재시도를 멈춘다. 날아가던 invoke 와 `lastSent` 는 남긴다 — 그 응답이 셸의 지금
   * 값을 맞추고, 다시 붙은 쪽도 그 응답을 기다려 한 번에 하나가 지켜진다.
   */
  release(): void {
    if (this.timer !== null) clearTimeout(this.timer)
    this.timer = null
    this.pending = null
    this.cancelJob()
  }

  private cancelJob(): void {
    this.gen += 1
    this.job = null
  }

  /** 대기 값을 내보낸다. 날아가는 invoke 가 있으면 그 응답 뒤(`start` 의 `finally`)에 다시 불린다. */
  private pump(): void {
    if (this.inAir || this.pending === null) return
    const value = this.pending
    this.pending = null
    if (this.job) {
      if (this.opts.same(this.job.value, value)) return
      this.cancelJob()
    }
    if (this.sent !== null && this.opts.same(this.sent, value)) return
    this.start(value)
  }

  private start(value: T): void {
    const gen = ++this.gen
    this.job = { value, gen }
    const attempt = async (): Promise<void> => {
      this.inAir = true
      try {
        await this.opts.send(value)
        // 같은 값이면 객체를 갈지 않는다 — 구독자가 읽는 값의 참조가 알림 없이 바뀌지 않게.
        if (this.sent === null || !this.opts.same(this.sent, value)) {
          this.sent = value
          this.opts.onAccepted?.(value)
        }
      } finally {
        this.inAir = false
        this.pump()
      }
    }
    void retryAsync(attempt, {
      isCancelled: () => this.gen !== gen,
      onRetry: (err, n) => {
        if (this.gen === gen) console.warn(`[${this.opts.log}] ${this.opts.describe(value)} 재시도 ${n}:`, err)
      },
    })
      .then(() => {
        if (this.gen === gen) this.job = null
      })
      .catch(err => {
        if (err instanceof RetryCancelledError || this.gen !== gen) return
        this.job = null
        console.error(`[${this.opts.log}] ${this.opts.describe(value)} 최종 실패(재시도 소진):`, err)
      })
  }
}
