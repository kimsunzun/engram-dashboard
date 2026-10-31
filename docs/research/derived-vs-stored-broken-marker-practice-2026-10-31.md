# 깨진 참조 표시 — 볼 때 계산하나, 바뀔 때 계산해 저장하나 (관행 조사 · light)

- **상태:** light · 2026-10-31 · 수집자 1(worker-scout) · 메인 grounding 스팟체크 2건(Foam `graph.ts` 증분 처리 주석 · `document-decorator.ts` 의 그릴 때 `resolveLink` — 둘 다 원문 지지). 적대 리뷰 없음(light).
- **계기:** S25 — 라이브러리 폴더를 옮겨 다닐 때마다 보이는 프리셋의 「찾을 수 없음」을 메모리 색인에서 계산하는 설계(P8)에 대해 사용자가 「폴더를 이리저리 옮기는데 매번 저 검사를 한다고?」로 물음(2026-10-31). 바로 앞 조사 = `broken-reference-check-timing-practice-2026-10-31.md`(언제 디스크를 보나).
- **확신도 범례:** 확실 = 1차 출처(소스) 확인 · 가능성 높음 = 단일 출처 · 불확실 = 추정 · 출처 없음.

## 결론

- 순수 「볼 때 계산」도 순수 「항목마다 깨짐 표시를 저장」도 아닌 **나눈 꼴이 흔하다**:
  - **저장하는 것 = 파일 사이 관계 색인**(누가 누구를 가리키나 · 백링크 · 풀리지 않은 링크 지도) — 추가 · 수정 · 삭제 때 **증분으로** 고친다(전체 다시 만들기는 처음 · 비상용).
  - **항목 옆 표시 = 그 색인에서 볼 때 찾아 계산** — 따로 「깨짐」 칸을 두지 않는다.
- **메모리 색인에서 볼 때마다 계산하는 것은 정상 · 값싼 것으로 다뤄진다**(Foam = 화면에 보이는 링크마다 그릴 때 `resolveLink` · Unity Inspector = 그릴 때 참조를 풀어 보고 없으면 「Missing」).
- **피하는 것은 변경마다 전체를 다시 훑거나 다시 만드는 것**이다(Foam 주석이 명시).
- 항목마다 표시를 저장하는 꼴은 **판정에 디스크 I/O 가 드는 곳**에서 보인다(Nautilus — 디렉터리 읽을 때 · 감시자가 알릴 때 따라가 본 결과를 파일 정보에 담아 둠). 메모리 찾기로 충분한 곳에선 안 보인다.

→ S25 대입: 지금 설계(첫 훑기 때 관계 표 ① 파일 목록 + 참조 · ② 거꾸로 표를 만들고 Engram 변경 때 증분으로 고침 · 폴더를 볼 때 보이는 프리셋만 그 표에서 찾아 표시)가 **Foam 과 같은 꼴**이다.

## 도구별

| 도구 | 저장하는 것 | 표시는 언제 | 확신도 · 출처 |
|---|---|---|---|
| Foam(VS Code 노트 확장 · 공개 소스) | 그래프 — 링크 · 백링크 · `placeholders`(풀리지 않은 링크) Map. 추가 · 수정 · 삭제마다 증분 갱신(삭제 = 그 노트를 가리키던 쪽만 다시 풀기). 전체 다시 만들기 = 처음 · 비상용 | 편집기 장식 = 활성 편집기 바뀔 때 · 글이 바뀔 때(지연) 화면의 링크마다 `resolveLink` → `isPlaceholder()` | 확실(메인 원문 확인 — `packages/foam-core/src/model/graph.ts` 21 · 87–104 · `packages/foam-vscode/src/vscode/features/editing/document-decorator.ts` 45–46 · 110–114) |
| Obsidian | `MetadataCache` 의 `resolvedLinks` · `unresolvedLinks` 지도 — 파일이 바뀔 때마다 갱신(`resolve` · `resolved` 사건) | 편집기의 「풀리지 않음」 모양을 어떻게 정하는지 = 비공개 소스라 미상 | 지도 = 가능성 높음(https://docs.obsidian.md/Reference/TypeScript+API/MetadataCache) · 그리기 = 불확실 |
| VS Code · 언어 서버 | 진단(「모듈을 찾을 수 없음」) — 밀어 주기(`publishDiagnostics`: 열 때 · 바뀔 때 다시 검사해 보냄 → 클라이언트가 저장) · LSP 3.17 당겨 받기(`textDocument/diagnostic` — 클라이언트가 보이는 문서만 · 언제 계산할지 고름) | 저장된 진단을 그린다 · 추세 = 보이는 것 위주로 게으르게 + 무효화 | 가능성 높음 |
| GNOME Files(Nautilus) | 파일마다 정보(따라가 본 종류) — 디렉터리 읽을 때 · 감시자가 알릴 때 | 「깨진 심볼릭 링크」 = 저장된 종류에서 바로 나옴(그릴 때 다시 stat 하지 않음) | 확실(`src/nautilus-file.c` 약 6921 · 2453 — 수집자 확인) |
| Unity Inspector | 직렬화된 참조 id | 그릴 때 id 가 아무것도 안 가리키면 「Missing」 — 따로 표시 저장 없음 | 가능성 높음(이슈 트래커 · 커뮤니티) |
| Unreal Asset Registry · Explorer · Finder | — | — | 미조사 · 불확실 |

## 공백

- Obsidian 그리기 쪽은 비공개라 미확인.
- Unreal · Windows 탐색기(.lnk) · Finder(별칭) 미조사.
- Logseq · Dendron · Zotero 미조사.
