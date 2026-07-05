# Third-Party Notices

이 파일은 이 프로젝트에 **포트(코드 복사·적응)** 된 서드파티 코드의 출처·라이선스·수정 내역을 기록한다.
verbatim 포트는 출처 버전을 고정(pin)해 두어야, 상류가 갱신됐을 때 diff·재동기화가 가능하다.

---

## Cline (채팅 렌더 컴포넌트)

- **출처:** https://github.com/cline/cline
- **고정 버전(pinned):** `cli-v3.0.37` — commit `25ef0939cc40cfadf5e916316562b3251c35b592` (2026-07-03)
- **라이선스:** Apache-2.0 — 전문 = [`LICENSES/cline-Apache-2.0.txt`](LICENSES/cline-Apache-2.0.txt). 저작권자 `Cline Bot Inc.` (Cline 저장소에 NOTICE 파일 없음 → Apache-2.0 §4(d) 미발동).
- **결정 근거:** ADR-0048 (`docs/decisions/`).

### 포트된 파일 (upstream `apps/vscode/webview-ui/src/components/` 기준)

| 우리 경로 | Cline 원본 | 방식 |
|---|---|---|
| `src/components/slot/cline/MarkdownBlock.tsx` | `common/MarkdownBlock.tsx` | verbatim + VSCode 콜아웃(useExtensionState Plan/Act·FileService 파일존재) 제거 |
| `src/components/slot/cline/MarkdownRow.tsx` | `chat/MarkdownRow.tsx` | verbatim |
| `src/components/slot/cline/ThinkingRow.tsx` | `chat/ThinkingRow.tsx` | verbatim |
| `src/components/slot/cline/CopyButton.tsx` | `common/CopyButton.tsx` | verbatim |
| `src/components/ui/button.tsx` | `ui/button.tsx` | verbatim + VSCode 토큰 variant 정리 |
| `src/components/slot/cline/cline.css` | `common/` 인라인 마크다운 스타일 | 토큰 매핑 적응 |

- 각 포트 파일 상단에 "Originally from Cline … Modified by …" 헤더가 있다(Apache-2.0 §4(b)).
- **비포트(우리 설계):** `StructuredTextView.tsx`(dispatch·레이아웃)는 Cline `ChatRow`(VSCode gRPC 강결합·복사 불가)를 참조해 **재구성**한 우리 코드다. Cline 룩 구조(flat 스택·헤더 패턴·툴 박스)를 따르되 우리 `StructuredItem` 모델을 소비한다.
- **수정 요지:** VSCode gRPC/컨텍스트 배선 제거 · styled-components 배제(Tailwind, ADR-0047) · VSCode CSS 변수 → 우리 data-theme 토큰 매핑 · 신뢰 불가 콘텐츠는 InertCode로 격리.

### 재동기화(re-sync) 절차

Cline이 채팅 뷰(`chat/`·`common/MarkdownBlock` 등)를 갱신하면:
1. 참조 클론(`I:\Engram_Workspace\references\cline`)을 최신으로 fetch.
2. 위 고정 commit `25ef0939`와 새 버전 사이 해당 파일 diff 확인.
3. 우리 포트 파일에 반영(수정 헤더 갱신) 후 이 문서의 **고정 버전**을 새 commit/tag로 bump.

> 이 파일을 갱신하면 pinned 버전이 정본이다 — 포트 파일 헤더·ADR-0048과 함께 유지한다.
