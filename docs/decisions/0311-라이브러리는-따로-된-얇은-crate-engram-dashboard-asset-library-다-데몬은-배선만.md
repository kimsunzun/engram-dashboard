# ADR-0311: 라이브러리는 따로 된 얇은 crate `engram-dashboard-asset-library` 다 — 데몬은 배선만

- 상태: 제안 (2026-10-31, 근거: 사용자 결정 G16(2026-10-20 「라이브러리를 별도 create로 확정하는건 맞는데」) · G24(2026-10-29 「asset 으로해」) + 메인 결정 M1 · M17 · M18 · M25(보고) · 판정표 `docs/research/crate-split-checklist-2026-10-20.md`)
- 관련: 옛 메모 96(데몬 안 모듈 — 사용자 2026-10-14)을 대신한다 · ADR-0151(crate 분리 판정 기준) · ADR-0155 결정 1(어휘는 생산자 옆 — `library.*` 는 agent 블록) · ADR-0251(동기 포트 + `blocking_handler` 선례) · ADR-0175 · ADR-0269(base 입주 조건) · CLAUDE.md 「백엔드 확장」 · ADR-0334 · TRD `docs/process/S25-preset-library/trd.md` §1-1 거부한 대안 · §1-4 · §7 30

## 맥락
라이브러리(지침 · 스킬 · 설정 · 프리셋 파일의 훑기 · 읽기 · 쓰기 · 해석 · 다시 연결)를 어디 둘지. 메모 96(2026-10-14)은 데몬 crate 안 전용 모듈로 정했다 — trait seam 은 처음부터 두고, base 는 입주 조건(도메인 지식 · 소비자 하나)에 걸려 배제했으며, 따로 된 crate 는 「다른 crate 가 필요하거나 프리셋 로직이 커지거나 격리 시험이 필요할 때」로 미뤘다. TRD 의 옛 기각 사유는 「소비자가 하나다」였다.

## 결정
1. 라이브러리는 신설 crate `crates/engram-dashboard-asset-library/`(Rust `engram_dashboard_asset_library`)다. 이름 = 담는 것이 에이전트에 싣는 에셋이라 asset(TRD 어휘 `AssetRef` 와 맞춤). 버스 명령 `library.*` 와 화면 이름 「라이브러리」 는 그대로다.
2. 의존 = 정확히 agent · platform · command · base. 데몬이 이 crate 를 의존하고 **배선만** 한다(만들기 · 포트 꽂기 · 값 넘기기). agent 는 이 crate 를 모르고 포트만 정의한다.
3. 바깥 접점은 포트다. `LaunchKitSource` · `LibraryCommandHost`(명령마다 타입 있는 메서드)를 이 crate 의 서비스가 직접 구현한다. 백엔드마다 다른 설정 지식은 agent `backend` 의 설정 검사 포트(ADR-0334). 명부 변경은 manager 의 프로필 편집 포트로 — 명부 변경과 스폰 예약이 한 락 순서를 쓴다(M18).
4. 네트워크 클라이언트를 갖지 않는다 — 스키마 받기는 데몬이 꽂는 fetch 포트(trait 은 이 crate 가 정의 — M25). 시험 기능(`test-support`)이 없다 — 데몬 쪽 시험은 실 `FsStore` + 임시 폴더(M17).
5. 데몬 내부(`connection_core` · `command_delivery` · `control`)를 부르지 않는다. 데몬은 값(`DataLayout` 이 낸 자리 · `daemon_id`)으로 넘긴다.
6. 게이트 넷(M17): 시험 줄 · 의존 상한 = 이름 **집합** {agent · asset-library · base · command · platform}(줄 수 아님) · 메시징 격리 정규식에 `asset_library` · `use tauri` 0줄. async 반입 게이트는 스키마 받기가 들어올 때 정한다.

## 거부한 대안
- **데몬 안 모듈**(메모 96 — 사용자 2026-10-14 「애매하긴 하지만 그렇게 해」) — 사용자가 2026-10-20 따로 된 crate 로 확정했다(G16 — 「분리할수 있으면 분리 … 데몬안에 있는 조건이 데몬에 있는걸 너무 많이 연관시켜야되고 상호참조가 필요해서 어쩔수 없을때」). 옛 기각 사유 「소비자가 하나다」 는 ADR-0151 결정 1 이 폐기한 기준이다(판정표 결론 · TRD §1-1).
- **base 에 두기** — 도메인 지식이 있고 소비자가 하나라 입주 조건에 걸린다(메모 96 · ADR-0269).
- **crate 이름 `engram-dashboard-library`**(메모의 예) — 사용자가 asset 을 골랐다(G24 — 담는 것이 에셋 · `AssetRef` 어휘).

## 근거
- 판정표: 「분리할 수 있나」 다섯 물음이 다 「예」 — 단 백엔드 검사 지식을 backend 포트로 옮긴 뒤(M2). 대가는 고정분(매니페스트 · 게이트 등록 · 공개 표면)뿐이고, 들어오는 간선이 없는 신설 때가 가장 싸다(TRD §1-1). 판정표의 확신도 = 중간.
- 옮길 타입이 없다(판정표 C6 = 0) · 시험 기능이 필요 없다(C5 = 0)(TRD §1-4).

## 영향 / 불변식
- 이 crate 가 데몬 내부를 부르게 되면 crate 판정이 뒤집힌다(TRD §1-4 · 판정표 §3 「뒤집는 것」).
- 데몬 조립 순서(TRD §1-4): 명부 → 라이브러리 서비스(프로필 편집 포트 = 늦게 채우는 빈 칸 · 채운 뒤 약한 참조) → manager → 빈 칸 채움 → 스키마 갱신 스레드. 빈 칸 채우기는 명령 표 슬롯 `command_table_slot.set` **앞**이다 — 제어 라우트(MCP 서버)는 데몬 배선보다 먼저 뜨므로 빈 칸을 지키는 것은 그 표 슬롯이고(표가 서기 전 라우트 = 503) 수락 루프는 그 뒤에 연다(M31 — 옛 「채운 다음에 수락 루프 · 제어 라우트를 연다」 는 코드와 어긋났다). 빈 칸에 닿으면 오류로 답한다 — Noop 으로 접지 않는다.
- 데몬 `DataLayout` 은 `daemon\state\` 폴더를 넘기고 그 안 `library.json` 이름은 asset-library 가 소유한다(ADR-0264 결정 4 · M30).
- CLAUDE.md 「백엔드 모듈 맵」 · 「빌드·검증 명령」 의 새 줄은 구현 때 사용자 확인을 받아 더한다(M1 · M17).
- 메모 96 에 대체 표시(TRD §5 0c).
