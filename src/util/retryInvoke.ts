// ADR-0102: 부팅 pull 용 유계 재시도(backoff) 헬퍼 — 조기 invoke 레이스의 프론트측 방어망.
//
// ★왜 필요한가★: main 창은 이벤트 복구 경로가 없다(window:tabs-updated 는 탭 *변형* 시에만 발화 —
//   부팅 직후 정적 상태엔 안 온다). 그래서 부팅 시 list_tabs/get_view pull 이 한 번 실패하면(예: Tauri v2
//   부팅 레이스로 managed state 가 아직 준비 전인 순간) 그걸로 끝이고 화면이 로딩 플레이스홀더에 영구
//   고착된다. LayoutState 는 이제 pre-build manage 로 레이스가 구조적으로 사라졌지만(근본 수정), pull 은
//   여전히 one-shot 이면 안 된다 — 다른 조기 transient(DaemonClient 등 런타임 의존 상태의 순간적 미준비,
//   IPC 초기화 지연)에도 스스로 회복해야 한다.

/** 기본값은 부팅 pull 에 맞춘 보수적 값(짧고 몇 번만 — 부팅 UX 를 오래 막지 않음). */
export interface RetryOptions {
  attempts?: number
  baseDelayMs?: number
  /** 기본 2 → 총 대기 ~1s(부팅 체감 상한). */
  factor?: number
  onRetry?: (err: unknown, attempt: number) => void
  /** unmount 가드용. */
  isCancelled?: () => boolean
}

/** 정상 실패와 구분하는 sentinel — 호출부가 조용히 무시한다. */
export class RetryCancelledError extends Error {
  constructor() {
    super('retry cancelled')
    this.name = 'RetryCancelledError'
  }
}

const sleep = (ms: number): Promise<void> => new Promise(res => setTimeout(res, ms))

/**
 * 모든 시도가 실패하면 마지막 에러를 throw 한다 — 조용히 삼키지 않아야 호출부가 최종 실패를 표면화한다.
 */
export async function retryAsync<T>(fn: () => Promise<T>, opts: RetryOptions = {}): Promise<T> {
  const attempts = opts.attempts ?? 4
  const baseDelayMs = opts.baseDelayMs ?? 150
  const factor = opts.factor ?? 2

  let lastErr: unknown
  for (let i = 0; i < attempts; i++) {
    if (opts.isCancelled?.()) throw new RetryCancelledError()
    try {
      return await fn()
    } catch (err) {
      lastErr = err
      const isLast = i === attempts - 1
      if (isLast) break
      opts.onRetry?.(err, i + 1)
      await sleep(baseDelayMs * factor ** i)
    }
  }
  // ADR-0102(FIX-4): 소진 throw 직전 취소를 재확인한다 — 마지막 시도가 실패하는 *도중* unmount 되면
  //   위 루프 상단 가드는 이미 지났으므로 backend 에러가 그대로 throw 돼 호출부가 헛된 최종-실패를
  //   로깅한다(unmount 는 실패가 아니다). 여기서 취소면 RetryCancelledError 로 바꿔 호출부가 조용히
  //   무시하게 한다. ★한계★: in-flight fn()·backoff sleep 중간의 취소는 여기서 안 잡힌다(그 순간
  //   isCancelled 전이는 다음 체크포인트까지 반영 안 됨) — 최종 시도 경계의 spurious 로그만 없앤다.
  if (opts.isCancelled?.()) throw new RetryCancelledError()
  throw lastErr
}
