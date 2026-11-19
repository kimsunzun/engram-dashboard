# ADR-0385: Library roots can be several and independent, added as an S25 step between 10a and 9b

- 상태: 확정 (2026-11-19, 근거: user 2026-11-19 「여러개 도 될수있으면 좋을것같은데 물론 서로간에 간섭하지 않는다고하고」 · 「샘플 제공하려면 기본 연결상태로 둬야되잖아. 그리고 그게 GIT에 따로 보관되어있고, 그 상태에서 다른 유저가 자기꺼 추가할 수 있게 해야지」 · 「ENGRAM 전용과 회사 전용 분리하려면 폴더로 GIT에 각각 명확하게 분리하는게 좋고」 · timing 「ㅇㅇ 너 추천대로해」)
- 관련: Amends ADR-0312 (decision 1 one root) · ADR-0315 (root-relative `LibPath`, built-in `engram:/`) · ADR-0320 (trash inside the root) · ADR-0330 (relink) · ADR-0384 (step-9a rules) · research `docs/research/multi-root-asset-library-practice-2026-10-30.md` · parent TRD `docs/process/S25-preset-library/trd.md` §5 · step-10a TRD `docs/process/S25-preset-library/trd-library-commands.md` (Risk R7)

## 맥락
ADR-0312 (2026-10-31) reverted a multi-root design (G25 chain, with references across roots) to one root. The user now wants several roots again for two uses: a sample library shipped connected by default, kept in its own git repository, beside which each user adds their own; and the user's own Engram-only and company-only content kept in separate git repositories. With one root both must sit as subfolders of the same root, so every reference carries the subfolder name, every PC must clone under the same names, and the parts cannot live in different places.

## 결정
1. Several library roots can be registered. Roots are independent: a reference never crosses from one root into another (the G25-RESOLVED cross-root references are not revived).
2. The path form follows ADR-0312 decision 6: an added root gets a name prefix; a path without a prefix means the current (first) root, so every path written before stays valid.
3. It lands as a new S25 step after 10a and before 9b. Steps 9a and 10a stay single-root; 10a's command shapes must not preclude decision 2. Step 9b (move, copy, trash) and the client stage are built knowing roots.
4. The step's own TRD decides the rest: the prefix spelling and how it relates to `LibPath`, root management commands, how an agent's attached presets name their root, what happens to agents and presets when a root is removed or renamed, move and copy between roots, trash and root generation per root, and what the first start registers.

## 거부한 대안
- **Keep one root, one git repository per subfolder** — works today (dot folders such as `.git` are not read), but references are root-relative (ADR-0315), so they carry the subfolder name, every PC must clone under the same name, and the parts cannot sit in different places. Basis: code (`LibPath` is relative to the one root).
- **Ship samples as the built-in `engram:/`** — the built-in is read-only and travels inside the Engram repository and the distribution (ADR-0315 decision 3), so it cannot be kept in its own git repository or edited by the user. Basis: ADR-0315 and the `builtin\` layout.
- **Add roots after S25 edition 1 as a separate topic** — 9b's move and copy rules and the screens (steps 11–13) would be built single-root and redone. Basis: parent TRD §5 order. User directly rejected (offered as an option).
- **Add roots now, before 10a** — 10a's shapes extend additively (10a TRD Risk R7), so holding 10a back gains nothing. User directly rejected (offered as an option).

## 근거
- User decisions 2026-11-19 (quoted in the status line).
- Research 2026-10-30: Claude Code, codex, Blender and Ableton register several roots; overlapping names are told apart by a prefix; Obsidian keeps one root per vault.
- The 10a TRD draft found no 10a shape that blocks decision 2 (Risk R7).

## 영향 / 불변식
- ADR-0312 decision 1 is replaced; its decisions 2–7 stand, read per root (decision 6 is now the path form). Its 「영향」 line 「two root management commands」 holds until the multi-root step.
- ADR-0312 decision 3 (outside content is brought into the root, not used in place) meets this ADR where a root itself is any folder the user registers (e.g. a git clone elsewhere); the step TRD states which folders may be roots and asks the user if that reopens decision 3.
- Until the multi-root step lands, code stays single-root; 10a adds no shape that decision 2 would force to reshape (10a TRD Risk R7).
