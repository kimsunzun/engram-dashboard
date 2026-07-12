# 핸드오프: i18n 문자열 중앙화 — slice②(command) 완료·푸시, slice③(컴포넌트) 미착수

## 한 줄 상태 · 다음 첫 액션
- **상태:** ADR-0069 UI 문자열 중앙화 **구현 진행 중**. 그릇(커밋 `1bff317`) + 잠정문구/research(`52a46b1`) + **slice②(command 마이그레이션, `3447bc8`) 완료·푸시**. **slice③(컴포넌트 스윕) 미착수.** 워킹트리 클린, `origin/master` 동기화.
- **다음 첫 액션:** **slice③ 컴포넌트 스윕** — `src/components/`의 사용자 노출 한글(aria-label·버튼/라벨 텍스트·기본 탭명 `View 1`·빈상태 `- empty -` 등 ~40곳)을 `t()`로 마이그레이션. 파이프라인 = 코더(worker-senior Opus) → `review code full` 2인(doc-aware + cross-family Codex blind) → `qa full`(컴포넌트라 cdp 실측 대상 많음). `ko.ts` 네임스페이스 확장(기존 tab/slot/window/agent/preset/common/theme/dialog 재사용).

## 무엇이 됨 (커밋·푸시 — origin/master 동기화, 미커밋 0)
1. **`1bff317`** — `src/i18n/` 그릇: `ko.ts`(네임스페이스드 `key→한국어` 테이블 + `deepFreeze`) + `index.ts`(`t(key, params?)` — 두 단계 `keyof` union key(존재 안 하는 key=컴파일 에러) + template-literal params 타입안전 + `{name}` 보간 + `lookup()` `?? key` loud fallback = locale-aware 교체가 호출부·key 불변인 seam) + `index.test.ts`. **자체 경량**(react-i18next 미도입). review full 2R(Codex FIX 다수) + strip-and-check malformed-brace dev-guard + `@ts-expect-error` 타입 회귀 게이트.
2. **`52a46b1`** — `index.ts` 헤더에 **"잠정(interim) 구현" 문구**(사용자 요청) + `/research medium` 재검증 기록. **결론: 자체 t()는 단일언어·내부툴엔 정당(야매 아님), 파라미터까지 타입세이프. 다국어 본격화 시 seam 뒤에서 Lingui(경량·Vite 공식)/i18next(최대 생태계)로 교체가 표준.**
3. **`3447bc8`** — **slice② `src/commands/` 27 문자열** t() 마이그레이션(값 byte-identical, 로직 무변). 신규 네임스페이스 `theme`/`dialog`. review full 2R PASS(doc-aware Opus + Codex blind — Codex FIX 1건 = 순환 테스트 오라클) + qa full(cdp 실측).

## repo 상태
- 브랜치 **master**, `origin/master` 동기화(ahead/behind 0), 미커밋 0. remote = `github.com/kimsunzun/engram-dashboard`.
- **동시 세션 주의:** 이 repo master에 타 세션이 동시 작업할 수 있음(이전 핸드오프 경고 지속). push 전 `git fetch`로 재확인 — 이번 세션 내내 fast-forward였음.

## 검증 상태 (쌍)
- **돌린 것:** `npx tsc --noEmit`(clean) · `npm test`(vitest **530**) · **cdp 실측**(live 포트 9223 — `t()` dynamic import로 마이그레이션 key 반환값 byte-identical 확인: agent.monitor="에이전트 모니터링" 등). **재실행:** `npx tsc --noEmit` + `npm test`. cdp = `node scripts/cdp.mjs eval "..."`(포트 9223 실행 인스턴스 필요).
- **검증 안 된 것:** slice③ 미착수(대상 없음). 이 작업은 순수 프론트 — src-tauri Rust는 0 변경.

## do-not / 주의
- **자체 t() 유지 확정**(사용자 결정 + /research 재확인) — **라이브러리 전환 X.** `index.ts` 헤더 "잠정 구현" 문구 유지.
- **마이그레이션 3원칙(ADR-0069):** ① 값 **byte-identical**(표시 텍스트 불변) ② 내부 `console`/`throw` 진단은 **제외**(UI 노출 문자열만) ③ key = **안정 API**(네이밍 신중, churn 금지).
- **순환 테스트 주의:** 마이그레이션한 문자열을 테스트에서 `t()`로 비교하면 순환(production·assert 둘 다 ko.ts라 잘못된 값도 통과) → **리터럴 기대값으로 못 박는다**(slice②에서 Codex 적출·수정 — `slotContentCommands.test.ts:34` 참고).
- `ko.ts` 테이블 값에 **nested/stray brace 금지**(dev-guard가 `npm test`에서 잡음). 보간 토큰은 `{word}`(=`\w+`)만.
- **bare `cargo test`·`-p engram-dashboard` = WebView2 0xc0000139 크래시**(member-scoped `-core`/`-protocol`만). 단 마이그레이션은 Rust 무관이라 해당 없음.
- **리뷰어 바인딩:** cross-family(blind) = `mcp__codex__codex`(effort high 명시: `config:{model_reasoning_effort:"high"}`, sandbox read-only, approval never). doc-aware = `worker-senior`(Opus·xhigh). **SendMessage 툴 없음** → 코더/리뷰어 이어가기는 codex만 `codex-reply`(threadId) 가능, Claude 코더는 fresh 스폰 + 이전 산출·FIX 주입.

## 정지 조건 (slice③ 진행 시)
- 컴포넌트 문자열이 **UI 노출인지 애매**하면(내부 상태 문구·디버그 텍스트) 남기고 사용자에게 확인.
- 기본 탭명 `View {index}` 등 **보간 있는 것**은 `t('common.defaultTabName', { index })` 형태(seed에 이미 있음).
- **렌더 통합 테스트**(`ViewLayoutRenderer.test.tsx` 등)가 리터럴로 화면 텍스트를 검증하면 **유지**(byte-identical이라 통과) — 이게 black-box 회귀 가드다.

## 미결 결정 (사용자)
- slice③ 완료 후 **원래 로드맵**(에이전트 트리·프리셋 고도화 → Claude 스폰 → 오케스트레이션 → 대시보드 내 튜토리얼 "Engram") 복귀 여부.

## 참조 (읽을 것만)
- **ADR-0069** (`docs/decisions/0069-ui-문자열-중앙화-...md`) — 결정·범위·불변식.
- **코드:** `src/i18n/index.ts`(t() API·seam·"잠정 구현" 헤더) · `src/i18n/ko.ts`(테이블·네임스페이스·deepFreeze). **slice② 참조 구현 패턴:** `src/commands/*.ts`(import t · `t('ns.key')` · 값 byte-identical · 내부 진단 제외).
- **step-log 최근 3항목**(`docs/process/step-log.md`): 그릇 slice① · /research 재검증 · slice② command.
