# ADR 바인딩 — engram

골격(`../flow.md`)이 "스크립트"·"ADR 폴더"·"파일명 규약"·"템플릿"·"인덱스"·"상태 범례"·"앵커 명령"이라 부르는 자리에 끼우는 **engram 전용 실명령·실체·연동**이다. 골격은 어느 프로젝트나 동일한 범용 ADR 엔진이고, 이 파일이 engram으로 바인딩한다.

> **정본 = `scripts/adr.mjs`(서기 작업 실체) + `docs/decisions/README.md`(템플릿·인덱스·상태 범례·supersede 규칙) + CLAUDE.md "★ 설계 결정 기록 (ADR) — 강제 ★" 절.** 이 파일은 그 **현재 바인딩 스냅샷**일 뿐이다 — 충돌하면 스크립트/README.md/CLAUDE.md를 따르고 이 파일을 고친다(rot 방지). 템플릿·인덱스 표를 통째 복붙해 제2의 출처로 만들지 않는다 — *가리키기만* 한다.

## 스크립트 실명령 (골격 §2 "스크립트 호출"에 주입)

서기 작업은 **스킬 폴더 안** `scripts/adr.mjs`(node 내장만, JSON 출력 — cdp.mjs 결)가 결정적으로 한다. 스크립트가 스킬과 함께 소비처로 이동한다(자족 스킬). **워크스페이스 루트(= dashboard repo 루트)에서** 실행한다. `<skill>` = 이 스킬 폴더 경로(배포처 `.claude/skills/adr/`).

engram(dashboard)의 실값은 **전부 스크립트 기본값과 동일**하다 — 그래서 dashboard에선 파라미터 플래그 없이(폴더만 기본) 호출한다:

- ADR 폴더 = `docs/decisions/`(스크립트 기본) · 인덱스 파일 = `README.md`(기본) · 상태 어휘 = `확정/제안/폐기/거부`(기본) · 기본 상태 = `확정`(기본) · 템플릿 = dashboard 내장 템플릿(기본, `<skill>/references/formats/adr.template.md`와 동일 구조) · 코드 앵커 루트 = `crates,src,src-tauri,scripts`(기본).

```bash
# new — 채번(max+1, 쓰기 직전 재스캔) + 스캐폴드 파일 생성. 본문 prose 슬롯은 TODO(스킬이 채움).
node <skill>/scripts/adr.mjs new --title "<한 줄 제목>" [--status 확정|제안]

# supersede 전체 — 새 ADR 스캐폴드 + 옛 status→폐기(기존 status 취소선 보존) + 양방향 Supersedes/Superseded by.
node <skill>/scripts/adr.mjs supersede --old <N> --mode full    --title "<새 제목>" [--status ...]

# supersede 부분 — 새 ADR 스캐폴드 + 옛 status 유지 + 양방향 Amends/Amended by (바뀐 조항 단서).
node <skill>/scripts/adr.mjs supersede --old <N> --mode partial --clause "<바뀐 조항>" --title "<새 제목>"

# index — 본문 H1·상태 스캔해 README 인덱스 표 재생성. 기본 --check(diff만, 안 고침), --write만 실제 갱신.
node <skill>/scripts/adr.mjs index --check     # 점검(read-only diff·경고)
node <skill>/scripts/adr.mjs index --write     # 실제 재생성(본문서 파생 가능한 것만, 큐레이션 셀 보존)

# lint — 정합성 점검(보고 전용, read-only). JSON에 error/advisory 구분.
node <skill>/scripts/adr.mjs lint
```

- **격리 테스트** — `--dir <폴더>` 또는 `ADR_DIR` 환경변수로 대상 폴더를 바꿔 실데이터 밖에서 dry-run. 기본 = `docs/decisions/`(cwd 기준). 스캔/상대경로 기준 루트는 `--root`(기본 = cwd).
- **파라미터 플래그(멀티 소비처)** — 스크립트는 여러 소비처를 하나로 섬긴다. dashboard는 위 기본값이 실값이라 플래그 불필요. 다른 소비처는 `--dir · --index-name · --template · --status-vocab a,b,c · --default-status · --anchor-roots a,b`로 실값을 주입한다(각 프로젝트 바인딩 소관).
- **호출 순서** — `new`/`supersede`로 파일을 만든 뒤 **본문 prose를 채우고**, 그 다음 `index --write`로 인덱스를 재생성한다(스캐폴드만으론 prose가 TODO라 인덱스 제목이 임시값일 수 있음 — prose 먼저, 인덱스 나중).

## ADR 폴더 + 파일명 규약 (골격 §2에 주입)

- **폴더** — `docs/decisions/`.
- **파일명** — `NNNN-제목-슬러그.md`(4자리 zero-pad 번호 + 슬러그). 슬러그는 스크립트가 결정적으로 생성(한국어 유지 · 영문 소문자화 · 공백→`-` · 특수문자 제거). 기존 예시 `docs/decisions/0001-kill-2동사.md`.
- **채번** — 스크립트가 `docs/decisions/`의 현재 `NNNN-*.md` 중 최대 번호 + 1로 결정(쓰기 직전 재스캔). 번호를 LLM이 지정하지 않는다.

## 템플릿 실체 (골격 §2 prose 채우기에 주입)

`docs/decisions/README.md`의 "템플릿" 절이 정본이다 — 섹션 구조(제목 / 상태 / 관련 / 맥락 / 결정 / 거부한 대안 / 근거 / 영향·불변식). 스크립트 스캐폴드가 이 구조로 빈 슬롯(`TODO`)을 만들고, 스킬이 §1에서 받은 내용으로 TODO를 메운다. **섹션 구조는 건드리지 않는다**(스크립트·README 정본 그대로). 여기 복붙하지 않는다(rot).

engram은 스크립트 **내장 기본 템플릿**을 그대로 쓴다(별도 `--template` 불필요) — 그 구조를 파일로 뽑은 사본이 `<skill>/references/formats/adr.template.md`(다른 소비처가 `--template`으로 가리킬 수 있는 형태). 내장 기본과 이 파일이 어긋나면 README.md·스크립트 내장을 정본으로 보고 이 사본을 고친다.

## 인덱스 위치·형식 + 재생성 (골격 §2 "인덱스 재생성"에 주입)

- **위치·형식 정본** — `docs/decisions/README.md`의 "## 인덱스" 표(`| # | 제목 | 상태 |`). `# ` 칸은 `[NNNN](NNNN-제목-슬러그.md)` 링크.
- **재생성** — `node scripts/adr.mjs index --write`가 본문 H1·상태줄을 스캔해 표를 다시 만든다(본문=단일 출처 → 제목/상태 drift 차단). **손으로 인덱스 표를 편집하지 않는다.**
- **보존 규칙(중요)** — 인덱스 셀이 본문보다 정보가 많으면(수작업 큐레이션 제목·레거시 부분폐기 status 단서) 스크립트가 **기존 셀을 보존하고 경고만** 낸다(자동 손실 금지). 신규 ADR은 본문 H1 = 인덱스 제목이 일치하게 만들어 drift를 애초에 안 만든다.
- **부분 폐기 인덱스 표기** — 본문에 양방향 `Amends`/`Amended by` 링크가 있으면 스크립트가 `<어휘> (부분 폐기 by ADR-N: <조항>)` 식으로 인덱스 단서를 합성한다. 링크 없는 레거시(0016/0024)는 기존 인덱스 단서를 보존.

## 상태 범례 (골격 §2 lint·§3에 주입)

`docs/decisions/README.md` "상태 범례"가 정본: **확정(Accepted) / 제안(Proposed) / 폐기(Superseded) / 거부(Rejected)**. 스크립트 lint은 이 **어휘만** 검사한다 — 상태줄의 단서절(em-dash 뒤 자유서술·부분폐기 설명)은 무시하므로 복합 상태 문자열에서 거짓양성이 안 난다. 어휘를 늘리면 스크립트 `STATUS_VOCAB`과 README를 같이 고친다.

## 코드 앵커 규약 + 고아 검출 (골격 §2 lint에 주입)

- **앵커 형식** — load-bearing 코드에 `// ADR-NNNN` 한 줄 주석. CLAUDE.md "rot 방지" 절이 정본(앵커는 코드와 한 몸이라 리스트처럼 rot하지 않는다).
- **고아 검출** — `lint`이 코드 경로(`crates/ src/ src-tauri/ scripts/`)에서만 `// ADR-NNNN`을 긁어 `docs/`는 제외한다(문서 본문의 "ADR-" 언급이 거짓양성 안 나게). 존재하지 않는 ADR을 가리키면 error, *폐기된* ADR을 가리키면 advisory(폐기된 결정을 코드가 아직 따를 수 있음 → 확인 권고).

## 흐름 기록 연동 (골격 §3 "결과 보고"에 주입)

- ADR 추가·번복의 **흐름(언제/무엇)** → `docs/process/step-log.md`. ADR은 *왜*(결정), step-log는 *언제/무엇*(흐름) — 섞지 않는다(CLAUDE.md "기록 분리").
- **스킬·스크립트는 step-log를 직접 쓰지 않는다 — 메인(오케스트레이터)이 처리**한다. 핸드오프 종료 체크리스트(새 ADR 썼나 / 폐기 양방향 박았나 / 인덱스 갱신했나 / step-log 추가했나)는 CLAUDE.md가 정본이며 메인이 세션 끝에 확인한다.
