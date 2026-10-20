# 라이브러리 폴더를 아직 안 골랐을 때 — 빈 화면 안내 (관행 요약)

- **상태:** light · 결정 대기(프리셋 PRD 이전 구상 — 사용자 결정 2026-10-15: 라이브러리 슬롯은 처음에도 평소 모습으로 뜨고 트리만 비며, 사용자가 눌러서 알아챌 수 있게 안내한다)
- **날짜:** 2026-10-15
- **방법:** research light. 수집자 1(검색 4 · fetch 2) · 메인 grounding(Obsidian 원문) · 적대 리뷰 없음.

## 결론

1. **자리 = 그 칸의 빈 내용 영역 안(띠가 아니다).** VS Code 탐색기는 빈 탐색기 안에 「welcome view」 — 문장 하나 + 「Open Folder」 · 「Open Recent」. 원문 문자열(소스 · 요약기 경유): 「You have not yet opened a folder.」. **가능성 높음.** Obsidian · Lightroom 은 막아서는 고르기 창이다.
2. **단추 = 「기존 폴더 열기」 + (앱이 저장소를 만들 수 있으면) 「새로 만들기」.** Obsidian 볼트 관리: 「Create new vault」(이름 · Browse · Create) · 「Open folder as vault」(Open) — **가능성 높음**(메인 원문 확인). Lightroom: 새 카탈로그 만들기 · 다른 카탈로그 고르기(커뮤니티 — 불확실).
3. **기본 경로 추천은 저장소를 「소유하는」 앱만 한다.** Calibre 첫 실행 마법사가 홈 아래 「Calibre Library」를 제안(포럼 — 불확실). 폴더를 보여 주기만 하는 VS Code 는 추천하지 않는다.
4. **지정해 둔 폴더가 없어졌을 때의 관행은 확인 못 했다.** Lightroom 은 마지막 카탈로그를 못 열면 카탈로그 고르기 창을 띄운다(커뮤니티 — 불확실). **공백.**

## Engram 에 옮기면 (후보)

- 빈 트리 칸 안에 VS Code 식 안내 — 한 문장 + [폴더 열기](기존 폴더) + [새로 만들기](배포판 안 `library\` 를 미리 채워 둔 위치에 만들기 — Obsidian 「Create new vault」 · Calibre 기본 경로 꼴).
- 폴더가 없어지면 같은 자리에 「…를 찾을 수 없습니다」 + 같은 단추 — 관행 확인 못 함(세션 제안).

## grounding

| 클레임 | 출처 | 판정 |
|---|---|---|
| Obsidian Create new vault(Create · Vault name · Browse) · Open folder as vault(Open) | https://obsidian.md/help/manage-vaults (메인 · 요약기 경유 원문 인용) | 지지 |
| VS Code 「You have not yet opened a folder.」 · Open Folder · Open Recent | https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/contrib/files/browser/explorerViewlet.ts (수집자 · 요약기 경유) | 미확인(메인 대조 안 함) |
| Calibre 기본 「Calibre Library」 · Lightroom 고르기 창 | 포럼 · 커뮤니티 | 불확실 |
