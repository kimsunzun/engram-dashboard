# ADR-0009: tauri 최신 2.x 핀 (Channel 무손실 실측)

- 상태: 확정 (S4 channel spike)
- 관련: CLAUDE.md 의존성 · `src-tauri/Cargo.toml`

## 맥락
초기 LLD는 "tauri 2.5 금지"였으나 caret 의존이라 2.11.2로 resolve됐다. Channel 무손실 여부에 의심(GitHub #11421)이 있어 버전 핀 결정 필요.

## 결정
**최신 2.x 유지**(spike 당시 2.11.2). 구버전으로 핀하지 않는다.

## 거부한 대안
- **구버전 핀** — #11421 우려로 보수적 고정. 실측 결과 불필요했고 최신 보안·기능을 포기하게 됨.

## 근거
S4 spike: 임시 command로 Channel 1000회 연속 send → 프론트 **1000/1000 무손실**(Windows WebView2). #11421은 Linux 특정 이슈로 Windows 미재현.

## 영향 / 불변식
- 의존성 `tauri = "2"`(최신 2.x). 변경 시 보고.
- Channel은 출력 스트림의 1급 전송로(ADR-0003 sink의 Tauri 구현).
