# 내부 오류 뒤 「계속 돌되 디스크 쓰기는 멈춘다」의 선례 (2026-10-10)

- **상태:** 조사 완료(light — 수집자 1 · 적대 리뷰 없음 · 메인 grounding 스팟체크). ADR-0289 결정 3(경고 중 데몬 저장 멈춤)의 관행 확인용.
- **확신도 범례:** 확실 · 가능성 높음 · 불확실 — 수집자 자기보고에 메인 스팟체크를 얹었다(독립 교차확증은 없다).

## 결론

**부분적으로만 관행이다.** 「쓰기를 막고 계속 돈다」는 저장 엔진 · 파일시스템에서는 잘 알려진 모양이지만, 계기가 **저장 계층의 오류**(I/O 실패 · 손상 검사 불일치 · 공간 부족)다. **앱 메모리의 공유 상태에서 난 버그** 뒤에 저장을 멈추고 계속 도는 직접 선례는 찾지 못했다. 그 상황에 가장 가까운 PostgreSQL 은 반대로 전체를 재시작한다. 우리 안은 저장 계층 모양을 앱 층으로 옮긴 것이다.

## 선례 — 쓰기를 막고 계속 돈다

| 시스템 | 계기 | 동작 | 출처 | 확신도 |
|---|---|---|---|---|
| Linux ext4 `errors=remount-ro` | 파일시스템 오류 감지 | 읽기 전용으로 다시 붙인다(`continue` · `panic` 도 고를 수 있다) | kernel `fs/ext4/super.c` (https://git.linaro.org/plugins/gitiles/kernel/linux-linaro-stable.git/+/39ef17f1b0613b46c6973596525c2bc816d90b5b%5E%21/fs/ext4/super.c) | 확실 |
| ZFS `failmode=continue` | 풀 장애 | 새 쓰기에 EIO · 읽기는 허용(`wait` 기본 · `panic` 도 있다) | https://docs.oracle.com/cd/E19253-01/819-5461/gftgp/index.html | 확실 |
| RocksDB (hard background error) | 쓰기 콜백 · flush 오류(공간 부족 등) | DB 를 읽기 전용으로 · 복구 조치 뒤에야 쓰기 재개 | `db/error_handler.h` (포크 사본 https://git.nextgraph.org/NextGraph/rocksdb/blame/branch/oxigraph-main/db/error_handler.h) | 확실(메커니즘) · 가능성 높음(손상 계기) |
| etcd 경보 | NOSPACE · CORRUPT(멤버 간 해시 불일치) | NOSPACE 는 해제될 때까지 쓰기 거부. CORRUPT 가 쓰기를 막는지는 미확인 | https://etcd.io/docs/v3.6/op-guide/data_corruption/ | 확실(NOSPACE) · 불확실(CORRUPT) |
| Redis `stop-writes-on-bgsave-error` | 마지막 백그라운드 저장 실패 | 쓰기 거부 · 읽기는 계속 | redis.conf (수집자 기억 — 미확인) | 가능성 높음 |

## 반대 입장 — 망가졌을 수 있으면 재시작한다

- **PostgreSQL** — 백엔드 하나가 비정상으로 죽어 공유 메모리가 망가졌을 수 있으면, postmaster 가 모든 백엔드를 끝내고 깨끗이 다시 띄운 뒤 WAL 로 복구한다. **우리 「공유 상태 패닉」에 가장 가까운 사례이고 저장 멈춤이 아니라 재시작이다.** (https://pganalyze.com/docs/log-insights/server/S1) 확실.
- **Crash-only software**(Candea & Fox, HotOS 2003) — 멈추는 길은 크래시 하나, 시작하는 길은 복구 하나. 복구를 빠르게 설계해 두고 성능 저하 상태를 두지 않는다. (https://static.usenix.org/events/hotos03/tech/full_papers/candea/candea_html/index.html) 확실.
- ext4 · ZFS 의 `panic` 선택지도 같은 이유다 — 의심스러운 상태로 계속 가느니 멈춘다.

## 한계

- SQLite · LMDB · VS Code · JetBrains 의 동작은 확인하지 않았다. 내부 오류 뒤 자동 저장을 끄는 에디터는 찾지 못했다(없다는 증거는 아님).
- light 조사라 cross-family 적대 리뷰가 없다. 수집자의 저장 계층 사례 셋(ext4 · ZFS · PostgreSQL)은 메인이 지식으로 맞는지 대조했고 출처를 다시 열지는 않았다.
