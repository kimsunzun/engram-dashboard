# Cache key for check-pass records — content fingerprint vs modification time (practice survey)

- **Status:** light survey · 2026-10-18 · 1 collector (documents) + main grounding (checked against the Prettier · Gradle originals) · no adversarial review
- **Questions:** To keep 「이 파일은 이미 검사를 통과했다」 (this file has already passed the check) across an app restart and skip it, what is used as the key, is the tool version put into the key, where is it stored, and how is an edit made while the tool was off caught.
- **Context:** A design that records, per config asset, the pass of the claude · codex CLI check (about 2 seconds) as (content fingerprint · CLI version).

## Conclusions

- **If correctness matters, the key is a content fingerprint.** Prettier defaults to content, and Gradle uses a content hash as the key but has only a shortcut that does not recompute the fingerprint when the modification time · size are the same. ESLint defaults to modification time, with content as an option. — certain (checked against the Prettier · Gradle originals)
- **Tool version · settings go into the key** — Prettier (version · options · Node version) · Gradle (the task's code) · ESLint (config · version). What is left out (Prettier's plugins) is separately warned about by the docs as 「캐시를 지우라」 (clear the cache). — likely
- **An edit made while the tool was off is caught on the next run by comparing with the stored fingerprint** — the tool does not need to be running. The modification-time method misses an edit that preserved the time · size, and git clone · checkout changes only the time without changing the content (which is why Prettier · ESLint offer the content method). — likely
- **Storage = one tool-specific file** (`.eslintcache` · `node_modules/.cache/prettier/.prettier-cache` · `.tsbuildinfo`) · the location can be changed with a flag. — likely
- **This design (content fingerprint · CLI version) matches practice.** The cost of hashing a small YAML is negligible next to a 2-second check (no source for the number — inference).

## Per peer

| Peer | Key | Tool version | Storage | Confidence | Source |
|---|---|---|---|---|---|
| Prettier `--cache` | default content (「If no strategy is specified, `content` will be used.」) · optional metadata | included in the key (「Prettier version, Options, Node.js version」) · plugins are left out (「not used as cache keys」) | `./node_modules/.cache/prettier/.prettier-cache` | certain (checked against the original) | https://prettier.io/docs/cli |
| Gradle incremental build | path + content hash · 「If the stats of a file (i.e. lastModified and size) did not change, Gradle will reuse the file's fingerprint」 | 「Gradle also considers the code of the task as part of the inputs」 | not confirmed | certain (checked against the original) | https://docs.gradle.org/current/userguide/incremental_build.html |
| ESLint `--cache` | default metadata (modification time) · optional content | invalid when the config · version changes (summary quote) | `.eslintcache` (cwd) | likely | https://eslint.org/docs/latest/use/command-line-interface |
| TypeScript `.tsbuildinfo` | per file version (content hash) + signature | unconfirmed | JSON file | likely (secondary source) | https://deepwiki.com/microsoft/TypeScript/8.3-builderprogram-and-state |
| git racy-git | if the modification time is at or after the index time, compare content | — | — | certain | https://www.kernel.org/pub/software/scm/git/docs/technical/racy-git.html |

## Gaps

- pre-commit · Bazel · Nx · Turborepo not surveyed. The Gradle cache location · the TypeScript version field unconfirmed. Handling when the cache file is corrupted or written concurrently not surveyed.

## Addendum — deleting stale records (2026-10-18 · light · checked against the file-entry-cache README original)

- **Check caches that keep a result per file (ESLint · Prettier — both `file-entry-cache`) are keyed by path.** When saving, they sweep all keys and **delete the records of files that are not on disk** — 「Saves the cache to disk and removes any files that are no longer found.」 (README · checked against the original). Content comparison is optional (`useCheckSum` — 「use a checksum to determine if the file has changed」). — likely https://github.com/jaredwray/cacheable/blob/main/packages/file-entry-cache
- **Even when the version changes, they do not wipe everything** — they hash the tool version · Node version · config and attach it to each record, and if it mismatches they treat that record as missed and overwrite it (ESLint `lint-result-cache.js` · Prettier `format-results-cache.js`). Prettier deletes and recreates the cache file if its shape is broken. — likely (source summary)
- **Stores keyed only by a content fingerprint (ccache · the Bazel disk cache) do not delete by 「안 쓰이는 지문」 (unused fingerprint) but by a size cap + oldest first (LRU · age).** — certain (collector quote) https://ccache.dev/manual/latest.html · https://bazel.build/remote/caching
- **No precedent was found for 「지금 어떤 파일과도 지문이 안 맞는 기록 지우기」 (deleting records whose fingerprint matches no file right now)** — the practice for this case = the ESLint · Prettier shape above (path key + content comparison + deleting nonexistent paths + overwriting on version mismatch).
