# study-note — 2026-06-28 · Tauri Channel 멀티윈도우 carrier (deep)

**왜 deep:** D3 carrier가 출력배선의 척추 + 동시성-치명 + 되돌리기 비용 큼(잘못 고르면 T5~T8 전부 재작업) → deep 트리거 충족.

## 강도 체감 (deep가 medium과 다른 점)
- **갈래 3개 × Claude 1 + Codex 2(주제별)** 동시 BLIND → 산출이 medium보다 넓고, **적대 검증 별도 pass**를 추가로 돌림(medium은 핵심 2~3개만, deep은 모순+미검증 전수).
- 이번엔 적대 pass가 결정적이었음: 수집 단계에서 **모순 1건(Channel raw 지원 여부)** 과 **미검증 2건(순서버그 #12065 fix 버전, raw 경로 실재)** 이 남았는데, medium이었으면 병기로 끝났을 것을 deep의 적대 Codex가 소스(`channel.rs`/`mod.rs`/CHANGELOG)로 **확정**.

## 쟁점 → 결론 도달 경로
1. **모순: Channel binary 지원?** Claude A=미지원 / Claude B+Codex=지원. → 적대 검증이 `mod.rs` blanket `impl<T:Serialize> IpcResponse` 확인 → **둘 다 부분적으로 맞음**: raw 경로는 실재하나 기본 `Serialize`는 JSON으로 샘. A의 "미지원"은 #13405(이벤트 ArrayBuffer FR)와 혼동한 환각. **단일 모델이면 A의 오류를 못 걸렀을 것** = cross-family 가치 실증.
2. **순서 보장 신뢰?** Claude B는 #12065 fix 버전 "불확실"로 남김 → 적대 Codex가 PR #12069 merge(2025-01-02) + CHANGELOG "2.2.0 fix" 확정 → 우리 2.11.2 = 해소. "불확실"을 "확실"로 승격.
3. **fan-out 설계:** Claude C·Codex가 **독립으로 거의 동일 코드**(`ArcSwap<RoutingSnapshot{by_agent}>` + `Mutex<HashMap<AgentId,usize>>` ref-count) 제시 → 만장일치지만 "공통 편향" 의심해 OSS 소스(arc-swap docs 공식 "routing table" 용례 + tmux/zellij/wezterm)로 교차 → 관행 일치 확인.

## 환각 거른 사례
- Claude A "Channel zero-copy binary 경로 없음" — 일부 사실(blanket=JSON)이나 결론(미지원)은 틀림. 적대 검증 없었으면 carrier 설계가 "Channel은 binary 안 되니 다른 거" 오판으로 샐 뻔.
- 핸드오프의 "#8916 = 2.6.0 미만" 표기도 이번에 beta.3 이슈로 정정(부수 발견).

## 미해결(실측 영역)
- raw Channel 실제 throughput은 문서/소스 추정 → `cdp.mjs` 실측(QA full) 필요. 리서치로 못 닫는 경계.
