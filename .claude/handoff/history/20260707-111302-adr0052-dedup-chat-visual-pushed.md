# 핸드오프: ADR-0052 json uuid/isReplay dedup + 채팅 시각 튜닝 — 완료·origin/master 푸시됨 (3커밋)

## 한 줄 상태 · 다음 첫 액션
- **상태:** (1) json 유저 에코 중복 제거를 blunt suppress → **uuid/isReplay dedup**으로 교체(ADR-0052) (2) 채팅 시각(유저 버블 좌측 인셋 둥근 박스 + 간격·폰트) 라이브 튜닝 후 소스 bake. **둘 다 게이트 통과·커밋·origin/master 푸시 완료.** 앱 실행 중(CDP 9223, 내 빌드).
- **다음 첫 액션:** 특정 진행 과업 없음(대기 상태). 후속 후보 = 아래 "남은 것"의 boy-scout/미검증 엣지, 또는 원 백로그(에이전트 간 메시지 데모 등, step-log "다음").

## 이 세션 커밋 (origin/master 푸시됨 · 74ce001..1bcb845)
- `dbced5a` feat(json): 유저 에코 dedup을 uuid/isReplay 기반으로 교체 (6 files)
- `f9d40da` docs(ADR-0052): 결정 박제 + step-log (3 files)
- `1bcb845` style(chat): 유저 버블 좌측 인셋 둥근 박스 + 간격·폰트 튜닝 (userPx 키 추가) (5 files)

## 무엇을 했나
- **dedup(ADR-0052):** 우리 생성 uuid를 stdin(`wrap_user_turn`)·합성 에코 양쪽 부착 → decoder 통과(억제 X) → 프론트 `structuredAccumulator`가 `type=="text"` user 블록만 uuid dedup. tool_result·과거·비매칭 보존. 근거 = claude가 `--replay-user-messages`에서 uuid 보존+`isReplay:true` 실측 확정 + 공식 VSCode 확장도 uuid dedup.
- **채팅 시각:** `__engramChat`로 라이브 튜닝 후 확정값 bake — 기본값 4개(lineHeight 1.45·railRowPt 0.8rem·plainRowPt 0.7rem·userPy 7px) 2곳 동기 + 유저 버블 좌측 인셋(양쪽 0.75rem 마진)·rounded-[0.75rem]·좌우 패딩 = 신규 tunable 키 `userPx`(0.9rem, userPy 대칭, §5 제어 표면 확장).

## 검증 상태 (쌍으로)
- **dedup:** `/review code deep` 적대 3인 PASS(multi-block 소실 결함 적출→FIX→재리뷰 2인 PASS) · full build · core 144+통합 · protocol 42 · fmt · 격리 · tsc · vitest · **uuid 왕복 실측** · **정상경로 GUI smoke**(실행 앱=dedup 빌드, 유저 메시지 1개씩·이중 렌더 없음).
- **채팅 시각:** `/review code light` PASS · `/qa quick`(vitest 277·tsc 0·BOM 없음·drift green).
- **미검증(정직):** ① dedup 신 로직의 *실익*(멀티블록 text+tool_result 동시 보존)은 이번 세션 **tool 호출이 없어 라이브 미검증** — 단위테스트로만 커버. "유저 1개씩 렌더"는 신·구 dedup 둘 다 만족하므로 신 로직 특정 증명 아님. 확인하려면 **tool을 쓰는 메시지를 보내** tool_result가 보존되는지 화면 확인. ② workspace 전체 `cargo test`는 src-tauri lib 테스트 `STATUS_ENTRYPOINT_NOT_FOUND`로 실패 = **안 건드린 crate 환경성 이슈**(canonical per-crate 테스트는 PASS, 무시 가능).

## 함정 (do-not) + boy-scout 후속
- **dedup 키 = `type=="text"` user 블록의 uuid만** — `extractUserUuid`를 uuid 단독 키로 되돌리면 멀티블록 tool_result 소실(리뷰 3인 적출).
- **chatStyle 이중 출처** — `CHAT_STYLE_DEFAULTS`(chatStyleStore.ts) ↔ `theme.css :root` 값 바꿀 땐 둘 다(drift 테스트가 지킴). `window.__engramChat` 키 화이트리스트 = `CHAT_STYLE_KEYS` 고정 배열(prototype-safe, `key in DEFAULTS`로 되돌리지 말 것).
- **boy-scout(비차단):** `StructuredTextView.tsx` user 버블 주석의 `rounded-xl` → 실제 className `rounded-[0.75rem]`로 표기 통일(안 하면 다음 세션이 주석 보고 되돌려 테스트 깨질 소지).
- **railLineOffset 커플링(관찰):** `railLineOffset`(-1rem)이 `railRowPt`(0.8rem로 바뀜)와 커플링돼야 하나 -1rem 유지 — 사용자가 라이브로 이 조합을 승인해 결함 아니나, 연결선 기하가 미세하게 어긋나면 `-0.8rem`으로 조정 검토.
- ADR-0044/0045/0050 불변식(decoder·StructuredTextView 순수)·opus thinking 암호화("Thought" 정상)·BOM 검사 — 기존대로.

## repo 미커밋 (이 라운드 밖 — 커밋 말 것)
- `run-dashboard-clean.bat`(데몬 재빌드 dev 스크립트) · `docs/reference/architecture-overview.md`(미추적 초안). 세션 시작부터 미커밋, 이 기능들과 무관.

## 앱 상태
- dev 앱 실행 중(CDP 9223, 내 빌드). JSON 모드 에이전트 활성·채팅 렌더 중. 임시 주입 CSS(`__engram_probe_userpad`)는 새로고침 시 사라지고 소스 CSS가 대체(이미 bake됨).

## 참조 (읽을 것만)
- `docs/decisions/0052-*.md` · step-log 2026-07-07 섹션
- 코드: `src/components/slot/structuredAccumulator.ts`(dedup) · `backend/claude.rs`(uuid/replay) · `src/store/chatStyleStore.ts`(chatStyle+userPx) · `theme.css :root --chat-*` · `StructuredTextView.tsx`(user 버블)
