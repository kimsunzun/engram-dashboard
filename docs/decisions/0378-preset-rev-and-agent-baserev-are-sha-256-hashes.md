# ADR-0378: A preset's rev is the SHA-256 of its exact file bytes kept on the catalog item, an agent's baseRev is the SHA-256 of the canonical JSON of its attachments, and both are compared with the value read inside the change

- 상태: 확정 (2026-11-17, 근거: S25 step-9a TRD §8 (b) B7 and B15, PRD+TRD review PASS 2026-11-17 (full))
- 관련: ADR-0338 (decisions 4 and 9: `reorder` is the only edition-checked bus edit; `save`'s optional `baseRev`) · ADR-0379 (refusal shape) · step-9a TRD `docs/process/S25-preset-library/trd-library-edit.md` §2 · §3.3 · §3.4 · §3.5 · §5 · §8 (a) · §8 (b) B7 · B14 · B15 · §9 U2 · U3

## 맥락
Preset and agent-preset edits that are edition-checked carry a `baseRev`: preset `reorder` (required) and `save` (optional) (ADR-0338 decisions 4 and 9, M84), and agent-preset `reorder`. That `rev` is a stable hash of the bytes was already decided (M21, TRD §8 (a)); the hash, where it is kept, how an agent's attachments get one, and what a supplied `baseRev` is compared with were open.

## 결정
1. **Preset `rev`** = lowercase-hex SHA-256 of the exact file bytes (BOM included), kept on the catalog preset item (`ItemContent::Preset` gains `rev`). It is computed from the bytes the item was made from — scan, late merge, edit or create — so `preset.get` answers from the catalog with no disk read.
2. **Agent `baseRev`** = lowercase-hex SHA-256 of the canonical JSON `{"default": <bool>, "presets": [<paths in order>]}`, computed from the registry. The `agent.presets` view carries it.
3. **Compared with the value read inside the change**: preset `reorder` / `save` compare a supplied `baseRev` with the `rev` of the bytes just read inside the change, not the catalog's; agent-preset `reorder` compares it with the agent `baseRev` computed from the registry read inside the change. A mismatch ⇒ `Conflict` · `rev-mismatch`, args `{rev}` (the current one).
4. **An edit that changes nothing writes nothing and keeps `rev`** (TRD §3.3 step 4).

## 거부한 대안
- **Compare a preset `baseRev` with the catalog item's `rev`** — TRD §3.3 names it and requires the bytes just read instead. The TRD gives no separate reason line for this; the ground in the TRD is that the catalog item can differ from the file (an outside edit — §3.3 M11 applies bytes read that differ from the catalog item, also when refusing). Basis: descriptive (weak).
- Agent `baseRev`: no alternative is named in the TRD (§8 (b) B15).

## 근거
- B7: stable across restarts (M21); `sha2` is already a dependency of asset-library; `preset.get` stays a memory read.
- B15: stable, order-sensitive, read straight from the registry.
- Step-9a PRD+TRD review (full, Claude lead + codex) PASS on `43f64c4f`, 2026-11-17 (TRD §13).

## 영향 / 불변식
- `rev-mismatch` is a new code (TRD §5); its class `Conflict` maps onto `CONFLICT` (ADR-0379).
- Adding `rev` to `ItemContent::Preset` touches every `Preset { doc }` pattern or constructor without `..` (TRD §7 U2).
- M11 (B14): an edit that read bytes differing from the catalog item applies them to the catalog also when it answers a refusal, so the same `CONFLICT` does not repeat after an outside edit.
- Tests that must hold: `rev` equal across a service rebuild; `reorder` `rev-mismatch`; an edit that changes nothing keeps `rev` (TRD §9 U2); agent `baseRev` order sensitivity (TRD §9 U3).
- Implemented in step 9a U2 (preset `rev`) and U3 (agent `baseRev`).
