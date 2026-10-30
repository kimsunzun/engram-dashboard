# ★ 상세 카드의 긴 「참조」 목록 — 보여 주는 관행 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 결정 대기(사용자) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 약 10회 · 가져오기 없음) · 메인 grounding 스팟체크 1건(NSW 디자인 시스템 「Show more」 WebFetch) · 적대 리뷰 없음 |
| 질문 | 항목 상세 카드의 「참조」(그 항목을 참조하는 프리셋 · 에이전트 — 이름 링크 · 건너서는 「(<프리셋> 통해)」)가 20개를 넘게 길어질 때 — 묶음 머리 · 5개만 · 「N개 더 보기」 제자리 펼침 안이 관행에 맞나 |

## 결론

- **개수는 늘 앞에 보이고, 긴 목록은 따로 된 전체 보기로 보낸다** — GitHub 「Used by N」 → 의존 저장소 페이지 · npm Dependents 탭 · 언리얼 레퍼런스 뷰어(따로 된 창 · 목록 복사 · 컬렉션 만들기) · Unity(결과가 다른 창에 걸러져 나옴). (가능성 높음 — 검색 요약 수준)
- **제자리에 두는 예는 Obsidian 백링크 하나 — 단위는 「N개 더」 가 아니라 파일별로 접고 펴는 묶음**(개수 표시). (가능성 높음 — 변경 기록)
- **「더 보기」 로 잘린 목록을 펼치는 꼴은 디자인 시스템이 말린다** — NSW: 「truncated list reveal」 에 쓰지 말고 아코디언(제목 붙은 여러 묶음) · 더 불러오기 · 페이지 나누기를 쓰라. (확실에 가까움 — 메인 원문 확인 · 단일 출처) · Siemens iX · Oracle = 중요한 몇 개만 보이고 「모두 보기」 는 새 화면. (가능성 높음)
- **건너서 참조를 줄 안에 표시하는 곳은 못 봤다** — 언리얼 · Unity 는 그래프 깊이나 따로 된 「의존」 동작으로 가른다. 우리 「(… 통해)」 는 선례가 없다. (불확실 — 못 찾음 ≠ 없음)

## 발견

| 도구 | 묶음 | 개수 | 처음 몇 개 | 더 → | 건너서 표시 | 출처 | 확신 |
|---|---|---|---|---|---|---|---|
| GitHub Used by | 아님(곁칸 아바타) · 의존 페이지에서 저장소/패키지 거르기 | 「Used by N」 | 아바타 몇 개 | 따로 된 Dependents 페이지(페이지 나눔) | 아님 | https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/exploring-the-dependencies-of-a-repository | 가능성 높음 |
| 언리얼 레퍼런스 뷰어 | 그래프(참조하는 쪽 · 의존하는 쪽) | 못 찾음 | — | 따로 된 창 · 목록 복사 · 컬렉션 만들기 · Asset Audit CSV | 그래프 깊이 | https://dev.epicgames.com/documentation/en-us/unreal-engine/reference-viewer-in-unreal-engine | 가능성 높음 |
| Unity | 아님 · 우클릭 동작이 Hierarchy/Project 를 거름 | 못 찾음 | — | 다른 창에 결과 | 아님 | https://docs.unity3d.com/Manual/search-ref-prop.html | 가능성 높음 |
| Obsidian 백링크 | 파일별 · 접고 펴기 | 총 개수 | 다 보임 · 사용자가 파일별로 접음 | 제자리 접고 펴기 | 아님(링크 안 된 언급은 따로 된 칸) | https://obsidian.md/changelog/2020-08-12-desktop-v0.8.4/ | 확실(변경 기록) |
| VS Code 참조 | 파일별 | 「N references」 CodeLens | — | 편집기 안 peek 목록 · 참조 보기 | 아님 | — | 불확실 |
| Confluence(서드파티 앱) | 탭 | 「8 links」 | 개수만 | 팝업 | 아님 | https://marketplace.atlassian.com/apps/1223325/incoming-links-for-confluence | 가능성 높음(코어 아님) |
| npm Dependents | 아님 | 탭에 수 | — | 따로 된 탭 · 페이지 | 직접만 | — | 불확실 |
| NSW 디자인 시스템 | — | — | — | 「Show more」 는 짧은 보충 한 덩이만 · 잘린 목록 펼치기 · 더 불러오기에 쓰지 않음 → 아코디언 · 더 불러오기 · 페이지 나눔 | — | https://designsystem.nsw.gov.au/components/show-more/ (메인 확인) | 확실에 가까움 |
| Notion | — | — | — | — | — | — | 기권 |

## 우리 설계에 주는 것

- 「5개 + N개 더 보기」 제자리 펼침은 관행과 어긋난다(NSW 가 말리는 꼴).
- 선례에 맞는 꼴 둘: ① Obsidian 꼴 — 「프리셋 3」 · 「에이전트 17」 처럼 **개수 붙은 묶음 머리를 접고 펴기**(아코디언) ② GitHub · 언리얼 꼴 — 카드엔 개수 · 몇 개, 「모두 보기」 는 **따로 된 전체 보기**로.

## 공백

- 어느 도구도 처음 몇 개를 보이는지는 확인 못 함 · Notion 자료 없음 · VS Code 참조 보기 · npm 은 공식 출처 없음 · 언리얼 에셋 상세의 Referencers 미확인.
