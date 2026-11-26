# ADR-0398: Before a feature ships its data gets no migration or compatibility path

- 상태: 확정 (2026-11-21, 근거: user 2026-11-21 「지금까지 저장된 경로가 없잖아. 이거 되게 자주 나오는데 배포되고 고려하라니깐 그런건.」)
- 관련: ADR-0396 (library mounts — the first application) · step-21 TRD `docs/process/S25-preset-library/trd-library-mounts.md`

## 맥락
Designs kept adding migration for data that only exists in development folders. The step-21 TRD planned a one-time rewrite of library paths written before mounts (a `library.json` schema-1 reader, rewriting agent attachments and preset files); its review found five must-level defects, all inside that rewrite. The library has never been in a release (the asset-library crate is not in tag `v0.4.0`). The user said this keeps coming up and that such handling belongs after release.

## 결정
1. Data whose format has never been in a release (a `v*` tag) gets no migration, no reader for its older shapes and no compatibility path. A format change simply replaces the old shape; development data in the old shape may be dropped or recreated.
2. Migration and compatibility are designed only for data whose format a release already wrote — from that release on.
3. A design document that changes such a format states 「not shipped — no migration (ADR-0398)」 instead of designing one.

## 거부한 대안
- **Migrate development data anyway** — costs design, code, tests and review rounds for data only the developers hold; in step 21 it produced five must-level defects. Basis: step-21 TRD review 2026-11-21. User rejected.

## 근거
- User decision 2026-11-21 (quoted in the status line).
- Release check: tag `v0.4.0` has no asset-library crate (step-21 TRD author, 2026-11-21).

## 영향 / 불변식
- Before designing a migration, check whether the format was in any `v*` tag; if not, skip it.
- Once a format ships, this ADR no longer covers it — later changes to that format need migration as usual.
