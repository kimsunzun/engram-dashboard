# ADR-0003: OutputSink / StatusSink — 코어 Tauri 격리

- 상태: 확정 (S1 설계, S3/S5 구현)
- 관련: CLAUDE.md §1·§4 · `types.rs::{OutputSink,StatusSink}` · `lib.rs::{ChannelOutputSink,TauriStatusSink}`

## 맥락
코어(`pty/`)가 Tauri AppHandle에 직접 묶이면 화면 없이 테스트할 수 없고, 전송 경로(PTY/HTTP/WebSocket)를 교체할 수 없다.

## 결정
출력·상태는 `OutputSink`/`StatusSink` **trait으로만** 흐른다. 코어는 Tauri·전송 방식을 모른다. Tauri 구현(ChannelOutputSink/TauriStatusSink)은 `lib.rs`에서 주입한다.

## 거부한 대안
- **코어가 AppHandle 직접 보유** — headless 테스트 불가, 전송 경로가 Tauri에 고정. 데몬·모바일 전환 시 코어 재작성.

## 근거
sink만 갈아끼우면 새 전송 경로가 흡수된다. `examples/headless·smoke`가 Noop/테스트 sink로 코어를 단독 실행해 검증.

## 영향 / 불변식
- **`pty/`(현 `engram-dashboard-core`) 하위 tauri import 0** — `rg "use tauri"` → 0줄 게이트.
- manager는 AppHandle이 아니라 `Arc<dyn StatusSink>`를 주입받는다.
