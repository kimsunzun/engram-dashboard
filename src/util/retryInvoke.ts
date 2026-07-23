// ADR-0102: 부팅 pull 용 유계 재시도(backoff) 헬퍼 — 조기 invoke 레이스의 프론트측 방어망.
//
// ★왜 필요한가★: main 창은 이벤트 복구 경로가 없다(window:tabs-updated 는 탭 *변형* 시에만 발화 —
//   부팅 직후 정적 상태엔 안 온다). 그래서 부팅 시 list_tabs/get_view pull 이 한 번 실패하면(예: Tauri v2
//   부팅 레이스로 managed state 가 아직 준비 전인 순간) 그걸로 끝이고 화면이 로딩 플레이스홀더에 영구
//   고착된다. LayoutState 는 이제 pre-build manage 로 레이스가 구조적으로 사라졌지만(근본 수정), pull 은
//   여전히 one-shot 이면 안 된다 — 다른 조기 transient(DaemonClient 등 런타임 의존 상태의 순간적 미준비,
//   IPC 초기화 지연)에도 스스로 회복해야 한다. 이 헬퍼가 "몇 번 재시도 후 성공하면 채우고, 다 실패하면
//   조용히 삼키지 말고 신호를 남긴다"를 두 부팅 pull 호출부(WindowLayout·initMainWindowFromBackend)에서
//   공유하게 한다(로직 중복 0).

/** 재시도 정책. 기본값은 부팅 pull 에 맞춘 보수적 값(짧고 몇 번만 — 부팅 UX 를 오래 막지 않음). */
export interface RetryOptions {
  /** 총 시도 횟수(첫 시도 포함). 기본 4 = 첫 시도 + 재시도 3회. */
  attempts?: number
  /** 첫 재시도 전 대기(ms). 이후 시도마다 factor 배로 증가(backoff). 기본 150ms. */
  baseDelayMs?: number
  /** backoff 배수. 기본 2 → 150·300·600ms 로 벌어진다(총 대기 ~1s, 부팅 체감 상한). */
  factor?: number
  /** 각 실패 후(마지막 실패 제외) 호출 — 진단 로깅용(선택). attempt=이번 실패 시도(1-base). */
  onRetry?: (err: unknown, attempt: number) => void
  /** 취소 신호(선택). true 를 반환하면 다음 시도 전 즉시 중단(unmount 가드용). */
  isCancelled?: () => boolean
}

/** isCancelled 로 중단됐을 때 던지는 sentinel(정상 실패와 구분 — 호출부가 조용히 무시). */
export class RetryCancelledError extends Error {
  constructor() {
    super('retry cancelled')
    this.name = 'RetryCancelledError'
  }
}

const sleep = (ms: number): Promise<void> => new Promise(res => setTimeout(res, ms))

/**
 * `fn` 을 유계 재시도한다. 성공하면 그 값을 반환하고, 모든 시도가 실패하면 *마지막* 에러를 throw 한다
 * (조용히 삼키지 않음 — 호출부가 최종 실패를 표면화하도록). isCancelled 가 중간에 true 면
 * RetryCancelledError 를 throw 한다(정상 실패와 구분).
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
      if (isLast) break // 마지막 시도 실패 → 대기 없이 lastErr throw(아래).
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
