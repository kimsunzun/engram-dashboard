# 화면 공유 · 원격 화면 — OSS 서베이 (2026-10-12)

- 상태: **medium (적대 리뷰 생략 — cross-family 검증 없음)** · 메인 출처 대조도 생략(사용자가 대조 전에 제안으로 넘어가기로 함, 2026-10-12) · 결정 없음
- 방법: `/research` · 수집 2갈래(완성 제품 · Rust 부품) → 메인 합성. ★아래 판·날짜·라이선스는 수집자 단일 출처다 — 쓰기 전에 다시 확인할 것★
- 계기: 원격 연결 위에서 상대 화면(캡처 · 실시간)도 볼 수 있나 — 사용자 질문
- 관련: `docs/research/transport-security-build-vs-adopt-2026-10-12.md`(같은 날 보안 축)

## 결론

1. **화면 전송은 transport 의 일이 아니다** — transport 는 바이너리 메시지 하나를 더 나를 뿐이고, 캡처 · 압축 · 전송 속도 조절은 그 위 기능이다. 이번 transport 설계에 영향 없음. `가능성 높음`
2. **두 갈래가 있다** — 영상 코덱 스트림(RustDesk · Sunshine/Moonlight · WebRTC 계열 — H.264/H.265/AV1, 대역폭 효율) vs 바뀐 영역 사각형(VNC/RFB · RDP · IronRDP). `가능성 높음`
3. **완성품을 앱에 넣기는 라이선스상 어렵다** — RustDesk = AGPL-3.0 · Sunshine = GPL-3.0(가능성 높음) · Selkies = MPL-2.0 · TigerVNC = GPL-2.0(가능성 높음). 넣을 수 있는 느슨한 것 = IronRDP(Rust · MIT/Apache) · FreeRDP(C · Apache-2.0, 가능성 높음) · noVNC(JS · MPL-2.0). 라이선스 확정 판단은 담당 부서 몫.
4. **현실적인 길 = 부품 조립.**
   - 스크린샷: `xcap`(Apache-2.0 · Win/macOS/Linux) + PNG/WebP.
   - 실시간: 캡처 `windows-capture`(MIT · Windows) / `screencapturekit`(MIT/Apache · macOS) + `openh264`(BSD-2 · 소프트웨어 H.264) → 우리 transport.
5. **빈칸 = 하드웨어 인코딩** — 확인된 Rust 바인딩은 `nvidia-video-codec-sdk`(NVIDIA 전용) 하나. OS · GPU 마다 백엔드를 갈아끼우는 trait 이 필요하다(QSV · AMF · Media Foundation · VideoToolbox 미조사).

## 주의

- OpenH264 특허: Cisco 가 배포하는 바이너리만 MPEG LA 특허 비용이 면제된다는 서술 — 직접 빌드는 면제 밖일 수 있다(가능성 높음 · 원문 미대조). 판단 = 담당 부서.
- FFmpeg/x264 의 GPL/LGPL 빌드 · 2018 이후 정체한 `scrap` 은 피한다.
- WebRTC(webrtc-rs · str0m)는 클라이언트가 브라우저일 때만 이득.
- 셸은 WebView2 `CapturePreview` 로 자기 창을 찍을 수 있다(불확실) — 헤드리스 데몬엔 해당 없음.
- 미검: 캡처 함정(UAC 보안 데스크톱 검은 화면 · DRM · 다중 모니터 DPI) · 각 제품의 대역폭 조절 · RustDesk 암호화 세부 · 일부 최신 릴리스 날짜(수집 중 충돌).

## 참고 실물

- `jrf63/desktop-streaming` — DXGI 캡처 + NVENC + webrtc-rs 의 작은 Rust 예(라이선스 미확인).
- IronRDP(Devolutions) — Rust RDP 클라 · 서버 crate 묶음.
