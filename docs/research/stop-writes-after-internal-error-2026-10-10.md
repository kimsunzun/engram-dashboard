# Precedents for 「계속 돌되 디스크 쓰기는 멈춘다」 (keep running but stop disk writes) after an internal error (2026-10-10)

- **Status:** survey complete (light — 1 collector · no adversarial review · main grounding spot-check). For checking the practice behind decision 3 of ADR-0289 (daemon stops saving while a warning is active).
- **Confidence legend:** certain · likely · uncertain — the main spot-check was laid over the collector's self-report (there is no independent cross-confirmation).

## Conclusion

**It is a practice only in part.** 「쓰기를 막고 계속 돈다」 (block writes and keep running) is a well-known shape in storage engines and filesystems, but the trigger there is an **error in the storage layer** (I/O failure · corruption-check mismatch · out of space). We did not find a direct precedent of stopping saves and continuing to run after a **bug in shared state in application memory**. PostgreSQL, the closest to that situation, does the opposite and restarts everything. Our proposal moves the storage-layer shape up to the application layer.

## Precedents — block writes and keep running

| System | Trigger | Behavior | Source | Confidence |
|---|---|---|---|---|
| Linux ext4 `errors=remount-ro` | filesystem error detected | remounts read-only (`continue` · `panic` can also be chosen) | kernel `fs/ext4/super.c` (https://git.linaro.org/plugins/gitiles/kernel/linux-linaro-stable.git/+/39ef17f1b0613b46c6973596525c2bc816d90b5b%5E%21/fs/ext4/super.c) | certain |
| ZFS `failmode=continue` | pool failure | EIO on new writes · reads allowed (`wait` is the default · `panic` also exists) | https://docs.oracle.com/cd/E19253-01/819-5461/gftgp/index.html | certain |
| RocksDB (hard background error) | write callback · flush error (out of space, etc.) | DB becomes read-only · writes resume only after a recovery action | `db/error_handler.h` (fork copy https://git.nextgraph.org/NextGraph/rocksdb/blame/branch/oxigraph-main/db/error_handler.h) | certain (mechanism) · likely (corruption trigger) |
| etcd alarms | NOSPACE · CORRUPT (hash mismatch between members) | NOSPACE rejects writes until cleared. Whether CORRUPT blocks writes is unchecked | https://etcd.io/docs/v3.6/op-guide/data_corruption/ | certain (NOSPACE) · uncertain (CORRUPT) |
| Redis `stop-writes-on-bgsave-error` | the last background save failed | rejects writes · reads continue | redis.conf (collector's memory — unchecked) | likely |

## The opposing position — restart if it may be broken

- **PostgreSQL** — if one backend dies abnormally and shared memory may be corrupted, the postmaster ends all backends, brings everything up clean again, and then recovers from the WAL. **This is the closest case to our 「공유 상태 패닉」 (shared-state panic), and it is a restart, not a save stop.** (https://pganalyze.com/docs/log-insights/server/S1) Certain.
- **Crash-only software** (Candea & Fox, HotOS 2003) — one path to stop, which is a crash, and one path to start, which is recovery. Recovery is designed to be fast, and no degraded-performance state is kept. (https://static.usenix.org/events/hotos03/tech/full_papers/candea/candea_html/index.html) Certain.
- The `panic` options of ext4 · ZFS have the same reason — stop rather than keep going in a suspicious state.

## Limits

- The behavior of SQLite · LMDB · VS Code · JetBrains was not checked. We did not find an editor that turns off autosave after an internal error (this is not evidence that none exists).
- Because this is a light survey, there is no cross-family adversarial review. The main model checked the collector's three storage-layer cases (ext4 · ZFS · PostgreSQL) against its own knowledge for correctness and did not reopen the sources.
