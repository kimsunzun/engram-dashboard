# ★ 마크다운 본문의 글자만 발췌(스니펫) — 길이 · 자르기 관행 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 결정 대기(사용자) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 · 가져오기 약 11회 · `remove-markdown` 소스 읽음) · 메인 grounding 스팟체크 1건(`strip-markdown` readme WebFetch) · 적대 리뷰 없음 |
| 질문 | 지시서(`.md` — 보통 앞머리 없음)의 상세 카드에 설명 대신 본문 앞부분을 글자만(마크다운 기호 · 띄어쓰기 정리) 보여 줄 때, 한 줄이 너무 길거나 제목 · 코드 · 표 · 긴 URL 로 시작하면 어떻게 하나 |

## 결론

- **줄 · 문단 단위로 자르지 않는다.** 본문 앞부분을 한 줄기로 펴고(줄바꿈 → 띄어쓰기) **글자 수나 줄 수(1~2줄)로 자르고** 넘치면 말줄임(…) 또는 흐림. (가능성 높음 — 앱 쪽은 도움말 · 사용자 글 수준)
- **제목이 있는 곳은 첫 줄(첫 제목)을 제목으로 쓰고 발췌는 그 다음부터** — Apple Notes(첫 줄 = 제목 · 목록 둘째 줄 = 날짜 + 다음 글 한 줄) · Bear. (가능성 높음 · Apple 원문은 안 가져옴)
- **전체 글은 열어서 본다** — 제자리 펼치기 · 올리기 툴팁 근거는 못 찾았다.
- **기호 걷기 라이브러리** — `strip-markdown`(remark): `code` · `html` · `horizontalRule` · `table` · `toml` · `yaml` 은 **내용째** 지우고 나머지는 평문 문단 · 그림은 alt 글자. (확실 — 메인 원문 확인) · `remove-markdown`(정규식): 기호만 떼고 제목 · 링크 글자 · 코드 내용은 남김 · 길이 상한 · 띄어쓰기 정리 없음. (확실 — 수집자가 소스 읽음)

## 발견

| 앱 | 걷는 것 | 첫 제목 | 길이 | 넘침 | 전체 보기 | 출처 | 확신 |
|---|---|---|---|---|---|---|---|
| Apple Notes | 문서 없음 | 첫 줄 = 제목 | 한 줄 | 문서 없음 | 열기 | https://support.apple.com/en-asia/guide/notes/not9474646a9/4.5/mac | 가능성 높음(검색 요약) |
| Bear | 못 찾음 | 못 찾음 | 미리보기 크기 설정 · 줄바꿈 없이 한 줄로 붙는다는 사용자 글 | 못 찾음 | 열기 | https://community.bear.app/t/display-new-lines-in-note-preview-list-view/14423 | 불확실 |
| Obsidian 검색 | 못 찾음 | — (맞은 곳 앞뒤) | 맞은 곳 앞뒤 1줄 · 「Show more context」 = 최대 5줄 | 못 찾음 | 열기 | https://obsidian.md/changelog/2020-08-12-desktop-v0.8.4/ | 가능성 높음 |
| Gmail | 문서 없음 | — | API `snippet` = 「a short part of the message text」(길이 미명시) | 미확인 | 열기 | https://developer.google.com/gmail/api/v1/reference/users/messages | 필드 = 확실 · 길이 = 기권 |
| GitHub 저장소 설명 | 마크다운 처리 안 함 | — | 입력 350자 상한 | 미확인 | 미확인 | https://slategit.com/blog/github-repo-description-character-limit-what-fits | 가능성 높음 |
| Notion 갤러리 「Page content」 | 문서 없음 | 문서 없음 · 첫 블록 · 그림이면 그림 | 첫 블록만 | 못 찾음 | 열기 | https://www.notion.com/help/galleries | 가능성 높음(도움말 사본 검색 요약) |
| Linear | 못 찾음 | 못 찾음 | Peek = 설명 전체 · 목록 줄 미리보기 못 찾음 | 못 찾음 | Peek · 열기 | https://linear.app/docs/peek | 불확실 |
| `remove-markdown` | 목록 기호 · 제목 기호(글자 남김) · 링크(글자 남김) · 강조 · 인라인 코드(내용 남김) · 코드 울타리(내용 남김) · HTML · 인용 `>` · 가로줄 · 각주 | 글자로 남김 | 없음 | — | — | https://github.com/stiang/remove-markdown | 확실(소스) |
| `strip-markdown` | 코드 · HTML · 가로줄 · 표 · toml · yaml = 내용째 · 나머지 = 평문 | 평문 문단 | 없음 | — | — | https://github.com/remarkjs/strip-markdown (메인 확인) | 확실 |
| Outlook | — | — | — | — | — | — | 기권 |

## 수집자 종합(출처 없는 설계 추론 — 관행 측정 아님)

1. 정규식보다 AST 로(`strip-markdown` 꼴) — 코드 블록 · 표 · HTML · 가로줄 · 앞머리를 지우고 링크 글자 · 그림 alt · 인라인 코드 내용 · 목록 글자는 남긴다.
2. 첫 블록이 제목이면 그 다음 글부터(제목뿐이면 제목 글자) · 다 걷고 남는 게 없으면 비우거나 파일 이름.
3. 블록을 띄어쓰기 하나로 잇고 공백 묶음을 하나로 · 맨 URL 은 호스트만이나 약 40자로 · 약 150~200자에서 낱말 경계로 자르고 … — 글자 상한 먼저, CSS 줄 제한은 그 다음(목록 줄 1줄 · 상세 카드 2~3줄).
4. 전체는 열어서(또는 `title` 올리기) — 제자리 펼치기는 하지 않는다.

## 공백

- Outlook · Bear · Apple Notes 원문 · Linear 목록 줄 · Obsidian Notes Explorer · GitHub 이슈 목록 안 봄.
- 기호 걷기 규칙을 문서로 밝힌 앱은 없었다 — 앱 줄 대부분은 도움말 · 사용자 글에서 추론.
- 넘침을 말줄임으로 하나 흐림으로 하나는 어느 출처에도 없었다.
