# 깨진 참조 표시는 언제 계산하나 — 볼 때 · 새로 고침 때 · 쓸 때 (관행 조사 · light)

- **상태:** light · 2026-10-31 · 수집자 1(worker-scout-sonnet · 검색 4회 · 대부분 검색 요약 기반) · 메인 grounding 스팟체크 2건(Blender = 지지 · Obsidian 갱신 시점 = 해당 문서에 없음 → 불확실로 강등). 적대 리뷰 없음(light). **근거가 얇다** — 시점을 문장으로 밝힌 1차 출처가 거의 없다.
- **계기:** S25 — 라이브러리 목록 · 트리 ⛔ 의 「찾을 수 없음」을 폴더를 볼 때마다(메모리 목록에 대고) 계산하는 설계(P8)를, 사용자가 「F5 나 실행할 때만 하면 되지 않나」로 물음(2026-10-31).
- **확신도 범례:** 확실 = 1차 출처 확인 · 가능성 높음 = 단일 출처 · 불확실 = 추정 · 출처 없음.

## 결론

- 갈래 셋이 모두 있다:
  1. **색인(메모리)에서 볼 때 바로 보이고 + 쓸 때 다시 확인** — 참조 색인을 들고 있는 도구(Obsidian 의 메타데이터 캐시 · IDE 류로 짐작 — 시점 1차 출처 없음 · 불확실).
  2. **명시적 검사 명령 때만** — Blender 「Report Missing Files」(확실 · 수동 명령) · Lightroom 「Find All Missing Photographs」(목록은 그 명령을 다시 돌려야 갱신 — 가능성 높음).
  3. **열 때 · 쓸 때 실패로 드러남** — 프로젝트 파일이 경로 문자열만 드는 도구(Premiere 「Link Media」 = 프로젝트 열 때 · Ableton = 세트 불러올 때 — 가능성 높음).
- 「새로 고침이나 쓸 때만」(2+3)은 **파일 기반 도구에서 흔한 꼴**이다. 색인을 들고 있는 도구는 1 이 많다(짐작).
- 바깥 변경은 표본 어디서도 감시자를 확인하지 못했다 — Blender · Lightroom · Ableton 은 수동 명령.
- 「보이는 항목만 vs 전체」를 밝힌 1차 출처는 없다(Lightroom 폴더 펼치기 조언이 「보이는 것만」을 암시할 뿐).

## 도구별

| 도구 | 시점 | 범위 | 바깥 변경 | 확신도 · 출처 |
|---|---|---|---|---|
| Blender | 명시 명령 「Report Missing Files」(경고가 없으면 빠진 것 없음) · 「Find Missing Files」(폴더를 골라 재귀 검색 · 찾은 것 복구) · 그 밖엔 불러올 때(추정) | 파일 전체 | 수동 | 명령 = 확실(메인 원문 확인 — https://docs.blender.org/manual/en/3.4/interface/window_system/topbar.html) · 불러올 때 = 불확실 |
| Lightroom Classic | 썸네일 「!」 배지(언제 검사하는지 미상) · 「Missing Photographs」 목록은 Find All Missing Photographs 를 다시 돌려야 갱신 | 미상 | 감시자 없음 · 다시 동기화 | 갱신 명령 = 가능성 높음 · 시점 = 불확실 (https://helpx.adobe.com/au/lightroom-classic/desktop/manage-catalogs-and-files/locate-missing-photos.html) |
| Premiere | 프로젝트 열 때 「Link Media」 | 프로젝트 전체(열 때 훑기 — 끄자는 요청 글로 짐작) | 미상 | 가능성 높음(Premiere Elements 문서 — https://helpx.adobe.com/ca/premiere-elements/desktop/importing-and-adding-media/offline-files.html) |
| Ableton Live | 세트 불러올 때 missing/offline · 「Manage Files › View Files」로 수시 목록 | 세트 전체 | 미상 | 가능성 높음(검색 요약 — https://help.ableton.com/hc/en-us/articles/209070829) |
| Obsidian | 메타데이터 캐시의 `unresolvedLinks`(볼트 전체 지도) | 볼트 전체 | 미상 | 캐시 존재 = 확실(https://docs.obsidian.md/Reference/TypeScript+API/MetadataCache/unresolvedLinks) · 갱신 시점 = 불확실(그 문서에 없음 — 메인 확인) |

## 공백

- Unity · Unreal · DaVinci Resolve · Zotero · IDE 는 못 봄.
- 전체 페이지를 거의 열지 않았다(검색 요약 기반).
- 색인형 도구가 바깥 변경을 감시자로 따라가는지 미확인.
