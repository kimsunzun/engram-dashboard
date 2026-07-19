# 에이전트 영속성/메모리 저장 — 선행조사 (관계형 DB 너머)

- **상태:** 조사만 완료 · **결정 없음** · 착수 보류(→ tracking T-14). 나중에 "데이터 저장/메모리" 스텝 착수 시 입력.
- **방법:** `/research` medium(설계-서베이 §7) — 후보별 수집자 5명(Sonnet) + 메인 grounding + cross-family 적대 리뷰(codex/GPT, effort high) 1회. 2026-07-19.
- **확신도 범례:** 확실 / 가능성높음 / 불확실 (grounding+리뷰 파생, 자기보고 아님). 벤더 벤치마크는 전부 미검증.
- **계기:** SQLite 개념 대화 중 사용자가 "관계형 DB보다 나은 에이전트 저장법이 있나"를 물어 파생. 순수 조사 메모(리뷰 파이프라인 없이 인라인 허용).

---

## 0. 전제 정정 — engram엔 아직 데이터 저장 시스템이 없다 (그린필드)

조사 중 드러난 사실: **현재 engram엔 범용 데이터 저장 시스템이 없다.**
- `agents.json`(ProfileRegistry) = 프로필 얇은 포인터(session-id·epoch·cwd) 세이브 파일일 뿐, 범용 저장소 아님.
- 데몬 replay 링 = 메모리 상주(휘발, 2MB/4096 상한).
- SQLite 메일박스(ADR-0086) = **종이 결정일 뿐 미구현** — 현재 스텝은 "최소 전송"(온라인 push).

즉 이 주제는 "있는 걸 확장"이 아니라 **백지에서 뭘 지을까**의 문제다.

## 1. "데이터 vs 데이터제어" → 3층으로 분해 (핵심 프레이밍)

저장 논의는 두 층이 아니라 **세 층**이다. "데이터는 어느정도 나왔냐"의 답은 층마다 다르다:

| 층 | 질문 | 상태 | 리스크·성격 |
|---|---|---|---|
| ① **저장 엔진** | *어디에* 담나 | **거의 결론 있음** | 저위험 · seam 뒤 교체 가능 |
| ② **데이터 모델/스키마** | *무엇을 어떤 관계로* 담나 (거미줄) | **백지** | 고위험 · 잘못 그으면 비쌈 → 지금 신중히 |
| ③ **데이터 제어/메모리 관리** | *어떻게* 회수·정리·망각하나 | **프론티어** | 연구 뜨거움 · LLM 제어와 직결 |

착수 순서는 **② → ③** (엔진 ①은 나중에 갈아끼움). engram "저위험+장기 = 지금 깐다" 원칙상 ②의 seam·enum·경계가 먼저다.

## 2. 프론티어 지도 (5갈래)

| 갈래 | 무엇 | 대표 | engram fit |
|---|---|---|---|
| 벡터/임베딩 | 의미 유사도 회수 | sqlite-vec, LanceDB, FAISS | 임베디드면 적합(단, FTS5 먼저) |
| 지식그래프/시간성 KG | 관계·멀티홉·진화하는 사실 | Graphiti/Zep, Kuzu(사망), SurrealDB | 패턴 유용, 라이브러리 시기상조 |
| 전용 메모리 프레임워크 | 저장 위 *관리 로직* | MemGPT/Letta, Generative Agents, Mem0 | **패턴만 차용**(Rust 네이티브 0) |
| 이벤트소싱/durable | append-only·replay·감사 | 이벤트소싱 패턴, LangGraph, Restate | 일부 요소 공유(과대해석 주의) |
| 하이브리드 임베디드 | 한 로컬 스토어에 관계형+벡터(+그래프) | sqlite-vec, LanceDB, DuckDB, SurrealDB | 실용 정답 후보 |

**2025~2026 흐름(가능성높음):** 순수 벡터 단독 퇴조 → **BM25+벡터 하이브리드 검색** 표준화 · 벡터+그래프+KV 3중 백엔드(Mem0·Zep) 주류 · **"durable agents"**(재개·감사 가능) 급부상 · 메모리가 프레임워크 내장 → **공유 인프라 레이어(MCP 표준화)**로 분리.

## 3. 적대 리뷰가 정정한 것 (수집자 과장 → 반증됨)

- **[HIGH] LanceDB "Rust SDK 1.0"은 틀림** — 1.0은 *Lance 파일포맷 SDK*, `lancedb` **Rust 크레이트는 아직 0.x**. "1.0이라 안정"은 과장. (가능성높음)
- **[HIGH] "engram이 이벤트소싱 80% 구현"은 근거없는 수치** — replay 링·epoch·메일박스는 *닮은 조각*일 뿐. 진짜 이벤트소싱 = 권위 이벤트 스트림 + projection + 스키마 진화 + 순서·동시성 + 부수효과. → "일부 요소 공유"로 낮춤.
- **[HIGH] "claude가 --resume으로 개별 메모리 있으니 per-agent 불필요"도 흔들림** — resume 세션은 컨텍스트 **압축 + 기본 30일 만료**(무손실·영구 아님). Claude Code는 이미 **레포범위 auto-memory**(세션간 공유) 보유. per-agent 니즈가 사라진 게 아님.
- **[MED] "프레임워크 전부 Python 전용"은 부정확** — Mem0=Node도, LangGraph=공식 TS. 단 **"Rust 직접 임베드 불가"는 유효**(engram 핵심).
- **[MED] 벡터 "시간순서 없음/비결정론"은 과장** — datetime·메타데이터 필터 됨, exact 검색은 결정론적.

## 4. 서베이가 빠뜨린 후보 (다음에 반드시 평가)

- **[HIGH] Claude Code 자체 파일시스템 auto-memory / RAG-over-files** — 가장 중요한 미스. engram 에이전트가 claude CLI라 이미 로컬·텍스트·레포범위 메모리를 들고 옴. **새 저장소 짓기 전에 "claude가 이미 뭘 들고 오나"부터 계산.**
- **[MED] SurrealDB** — Rust·임베디드·멀티모달(그래프+벡터+FTS 한 엔진). 평가 대상에 올릴 것.
- **[MED] 로컬 MCP 메모리 서버** — 공식 레퍼런스에 영속 지식그래프 메모리 서버. **engram이 이미 제어채널에 MCP 사용** → 교차-에이전트 메모리 seam으로 직결.
- **[MED] Sleep-time compute** — 유휴 시 백그라운드 consolidation(memory-at-rest, Letta).
- **[LOW] 프롬프트/KV 캐싱** — 영속 아님, 휘발성 작업기억 최적화로 분류(Claude 캐시 TTL 5분/1시간).

## 5. engram 판단 (제약: 로컬 단일호스트 · Rust 임베디드 · 수십 에이전트 · LLM 제어)

**적합(지금/가까이):**
- **FTS5(SQLite 내장 BM25)를 벡터보다 먼저** — 식별자·코드엔 렉시컬 강, 이미 내장. 벡터는 FTS5 재현율/지연 실측 후.
- **sqlite-vec** — 의미검색 필요 시 SQLite 확장 한 줄, Rust 정적링크, 프로세스 0 추가. 단 **pre-v1(바인딩 SemVer 밖)** → seam 뒤에 숨기는 게 전제.
- **패턴 차용(라이브러리 아님):** Graphiti *bi-temporal edge*(성립시각+관측시각), Generative Agents *중요도+감쇠+reflection*, Mem0 *consolidation(ADD/UPDATE/DELETE)*. 전부 Rust 직접 구현 가능한 *아이디어*.

**과잉/시기상조:**
- 서버형 전부 제외(pgvector·Weaviate·Milvus·Temporal·Restate 서버·Neo4j/Memgraph) — "단일 로컬 데몬" 위반.
- **Kuzu** — 임베디드 그래프로 기술적합했으나 **Apple 인수·2025-10 GitHub archive(사망)**. 포크(LadybugDB) 부활 중이나 채택 보류. (확실 — 다중출처)
- **GraphRAG** — 문서 RAG지 실시간 에이전트 메모리 아님, 인덱싱 비용 과다.
- **그래프 DB 도입 자체가 아직 이름** — 현재 거미줄은 SQLite 재귀 CTE로 충분.

**중기 후보:** LanceDB(크레이트 0.x 대기) · SurrealDB(멀티모달 Rust 임베디드) · MCP 메모리 서버(engram MCP 인프라 직결).

## 6. 착수 시 열어야 할 질문 (미결)

1. **②스키마:** 어떤 엔티티(에이전트·세션·메시지·작업·트리노드)를, 어떤 관계(생성·송수신·부모자식)로, 무엇을 source of truth로? (굵은 설계 → PRD/TRD/ADR)
2. **claude 기존 메모리와의 경계:** claude auto-memory/resume이 이미 커버하는 것 vs engram이 새로 들 것(교차-에이전트/오케스트레이션 메모리)의 분담.
3. **③제어를 LLM tool로 구동?** MemGPT식 self-editing 메모리를 §5(LLM-우선 제어)와 어떻게 엮나.
4. **엔진 착수점:** SQLite 정본 + FTS5 → (필요시) sqlite-vec, 전부 storage trait seam 뒤. 언제 벡터가 정당화되나(실측 트리거).
5. **보관정책(retention):** 무한 누적 vs delivered/read 프루닝 vs 아카이브 (ADR-0083 "시체 보존" 성향과 정합).

## 7. 쟁점/한계

- 벤더 벤치마크 전부 미검증(Zep DMR/LongMemEval, FalkorDB "374x", Mem0↔Zep LOCOMO 논쟁 중) — 독립 검증 없이 인용 불가.
- 크레이트 버전·라이선스(sqlite-vec pre-v1, lancedb 0.x)는 시간민감 — 착수 시점 재확인.
- medium 단일 패스 — 누락 탐침은 리뷰어 1회분. 착수 시 deep(독립 병렬수집 + SurrealDB/MCP메모리 포함)으로 재조사 권장.

## 출처 (핵심)
- Zep: Temporal Knowledge Graph for Agent Memory — arXiv:2501.13956
- MemGPT / Letta — github.com/letta-ai/letta · Generative Agents(Park 2023)
- Mem0 — arXiv:2504.19413 · A-MEM arXiv:2502.12110 · MemoryOS(EMNLP 2025)
- sqlite-vec — github.com/asg017/sqlite-vec · alexgarcia.xyz/sqlite-vec/versioning.html(pre-v1)
- LanceDB — docs.rs/crate/lancedb(0.x) · lancedb.com/blog/announcing-lance-sdk(포맷 1.0)
- Kuzu archive/Apple 인수 — github.com/kuzudb/kuzu · theregister.com(2025-10) · LadybugDB
- SurrealDB — github.com/surrealdb/surrealdb
- MCP reference servers(persistent memory) — github.com/modelcontextprotocol/servers
- ESAA 이벤트소싱 — arXiv:2602.23193 · Claude memory/sessions docs(auto-memory·30d 만료)
- 이벤트소싱/durable — Azure Event Sourcing pattern · LangGraph persistence · Restate/DBOS

*피드백: 없음.*
