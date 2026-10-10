//! 프로필 · 프리셋 영속화 — `agents.json` · `presets.json` 의 저장소. 두 파일이 이 모듈의 내부 저장소 하나를
//! 부른다(같은 규칙 · 같은 구현).
//!
//! 규칙은 base `file` 의 데이터 파일 규칙이다(ADR-0291):
//! - 쓰기는 원자적이다(같은 폴더 임시 → rename) — 크래시가 나도 반쪽 파일이 남지 않는다.
//! - ★「덮어도 되나」는 저장마다 그 직전에 지금 파일을 다시 읽어 판정한다★ — 새 판이 쓴 파일 · 못 읽는 파일은
//!   덮지도 떠 두지도 않고 그 저장을 거절한다. 손상 파일은 덮기 전에 `<이름>.corrupt` 로 떠 두고, 떠 두기가
//!   실패하면 그 저장도 실패한다(원본 그대로). 적재는 파일을 바꾸지 않는다.
//! - ★적재 판정이 거절(새 판 · 못 읽음)이면 이 실행 내내 저장하지 않는다★ — 메모리 명부가 그 파일에서 오지
//!   않았으므로, 나중에 파일이 읽히게 돼 재판정이 「쓸 수 있음」이라 답해도 그 위에 빈 명부를 쓰면 명부 전체를
//!   잃는다.
//!
//! ★저장을 직렬화하는 것은 부르는 쪽이다★ — 레지스트리가 자기 맵 락을 쥔 채 저장하고(ADR-0071) 운영에서
//! 저장소를 부르는 것은 레지스트리 하나뿐이다. 이 모듈이 쥐는 락은 잎 하나(저장소 상태 칸)이고 쥔 채 IO · 로그 ·
//! 밖 호출을 하지 않는다 — 락 순서 = `래치 → expected 칸 → profiles|presets → 저장소 상태 칸`.
// ADR-0291

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use engram_dashboard_base::file::{
    self, Claim, Loaded, OsHooks, Parsed, Refused, WritePolicy, BUSY_PAUSE, BUSY_RETRIES,
};
use engram_dashboard_base::sync;
use engram_dashboard_platform::fs::{retry_busy, sync_dir, Retry};
use engram_dashboard_platform::process::pid_alive;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::profile::{AgentProfile, ProfileStore, Refusal, StoreError, StoreStatus};

pub mod presets;
pub use presets::{FilePresetStore, PRESETS_FILE};

/// 프로필 저장소의 파일 이름 — 저장 거절 · 실패를 호출자에게 알리는 문구가 이 이름을 댄다.
pub const AGENTS_FILE: &str = "agents.json";

const PROFILES: FileKind = FileKind {
    name: AGENTS_FILE,
    legacy_tmp: "agents.json.tmp",
    list_key: "profiles",
    spec: file::Spec {
        version_key: "schema_version",
        current: 1,
        // ADR-0291 R6: 실측(이 PC 1,747 B)의 약 만 배 — 정상이면 안 닿고 쓰레기 파일로부터 메모리를 지키는 선.
        cap: 16 * 1024 * 1024,
        shape: profiles_shape,
    },
};

/// 쓰기 쪽 디스크 표현 — 버전 키를 늘 싣는다(읽기는 [`read_list`] 가 목록 칸만 본다).
#[derive(Serialize)]
struct ProfilesFile<'a> {
    schema_version: u64,
    profiles: &'a [AgentProfile],
}

pub struct FileProfileStore {
    inner: FileStore,
}

impl FileProfileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            inner: FileStore::new(dir, PROFILES),
        }
    }
}

impl ProfileStore for FileProfileStore {
    fn save(&self, profiles: &[AgentProfile]) -> Result<(), StoreError> {
        warn_if_secret(profiles);
        self.inner.save(
            profiles.len(),
            &ProfilesFile {
                schema_version: PROFILES.spec.current,
                profiles,
            },
        )
    }

    fn load(&self) -> Vec<AgentProfile> {
        self.inner.load()
    }

    fn status(&self) -> StoreStatus {
        self.inner.status()
    }
}

fn profiles_shape(version: u64, doc: &Map<String, Value>) -> Result<(), String> {
    list_shape::<AgentProfile>(&PROFILES, version, doc)
}

/// env에 자격증명으로 보이는 키가 있으면 경고(보안). persist를 막지는 않되 평문 저장 위험을
/// 로그로 알린다. 이상적으론 시크릿 제외 목록이지만, 우선 가시화부터.
fn warn_if_secret(profiles: &[AgentProfile]) {
    const NEEDLES: [&str; 4] = ["KEY", "TOKEN", "SECRET", "PASSWORD"];
    for p in profiles {
        for (k, _) in &p.env {
            let upper = k.to_uppercase();
            if NEEDLES.iter().any(|n| upper.contains(n)) {
                tracing::warn!(
                    agent = %p.id,
                    env_key = %k,
                    "프로필 env에 자격증명으로 보이는 키 — agents.json에 평문 저장됨. 자격증명은 env에 넣지 말 것."
                );
            }
        }
    }
}

// ── OS 몫 ────────────────────────────────────────────────────────────────────────

/// 저장 · 저장 직전 재판정 · 떠 두기 — 데이터 파일 예산(base `file` 의 상수).
const DATA_HOOKS: OsHooks = OsHooks {
    retry: retry_data,
    sync_dir,
};

/// 적재 읽기 — 따로 긴 예산을 쓴다.
// ADR-0291 R14 (D7): 거기서 남은 잠김은 이 실행 내내 읽기 전용이 되므로(R17) 잠깐 쥐기 하나가 실행 전체를 묶지
//   않게 길게 둔다. 위 끝은 셸의 데몬 찾기 마감(5 초 — 실패면 재시도 없이 `Down`)이 정한다: 적재는 접속 정보
//   발행 앞이라 두 파일 합 최악 약 2 초면 기동의 나머지 몫이 남는다.
const LOAD_HOOKS: OsHooks = OsHooks {
    retry: retry_load,
    sync_dir,
};
const LOAD_RETRIES: u32 = 19;
const LOAD_PAUSE: Duration = Duration::from_millis(50);

fn retry_data(attempt: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
    retry_busy(
        Retry {
            retries: BUSY_RETRIES,
            pause: BUSY_PAUSE,
        },
        attempt,
    )
}

fn retry_load(attempt: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
    retry_busy(
        Retry {
            retries: LOAD_RETRIES,
            pause: LOAD_PAUSE,
        },
        attempt,
    )
}

// ── 공통 저장소 ──────────────────────────────────────────────────────────────────

/// 파일 하나의 몫 — 주인 모듈이 상수로 둔다.
#[derive(Clone, Copy)]
struct FileKind {
    /// 파일 이름 — 로그의 `file` 칸이기도 하다.
    name: &'static str,
    /// 옛 빌드가 쓰던 고정 임시 이름. `file::sweep_temps` 는 번호 없는 이름을 남의 것일 수 있다고 보고 두므로
    /// 이 이름은 저장소가 직접 지운다(ADR-0291 R12).
    legacy_tmp: &'static str,
    /// 목록을 싣는 최상위 칸.
    list_key: &'static str,
    spec: file::Spec,
}

/// `agents.json` · `presets.json` 공통 저장소 — 상태 셋은 [`StoreStatus`] 다.
struct FileStore {
    dir: PathBuf,
    kind: FileKind,
    /// 저장소 상태 칸 — 잎 락. 읽고 쓰는 순간만 쥔다(판정 · 디스크 IO · 로그 중에는 쥐지 않는다).
    // ADR-0291 R12
    state: Mutex<StoreState>,
}

struct StoreState {
    /// 첫 [`FileStore::load`] 가 돌았나 — 적재 거절과 그 `Claim` 은 그때만 선다.
    loaded: bool,
    /// 적재 판정의 거절 — 이 실행 내내 남는다(R17).
    load_refusal: Option<Refusal>,
    /// 마지막 저장의 재판정 거절 — 쓰기가 성공하면 지운다(D6). 같은 까닭이 이어지면 로그를 낮춘다(R15).
    last_refusal: Option<Refusal>,
    /// 첫 적재 전 · 적재가 없음 · 손상이었고 아직 한 번도 안 썼으면 `NotYet`(D14).
    claim: Claim,
}

/// 첫 적재가 상태 칸에 남길 것.
enum LoadVerdict {
    /// 명부가 그 파일에서 왔다.
    Adopted,
    /// 없음 · 손상 — 명부가 그 파일에서 오지 않았다.
    NotAdopted,
    Refused(Refusal),
}

impl FileStore {
    fn new(dir: PathBuf, kind: FileKind) -> Self {
        Self {
            dir,
            kind,
            state: Mutex::new(StoreState {
                loaded: false,
                load_refusal: None,
                last_refusal: None,
                claim: Claim::NotYet,
            }),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(self.kind.name)
    }

    /// 파일을 읽어 목록을 돌려준다 — 없음 · 손상 · 거절이면 빈 목록. 파일은 바꾸지 않는다.
    ///
    /// ★적재 거절과 `Claim` 은 첫 호출에서만 선다★ — 뒤 호출은 읽어 돌려주기만 하고, 그 전에 성공한 쓰기가 올린
    /// `Claim` 을 내리지도 않는다. 첫 호출은 남은 임시 파일도 쓴다 — 이 프로세스 것도 지우므로 같은 파일을 쓰는
    /// 중인 저장이 없을 때(레지스트리를 만들 때) 불려야 한다.
    fn load<T: DeserializeOwned>(&self) -> Vec<T> {
        let first = !std::mem::replace(&mut sync::lock(&self.state).loaded, true);
        if first {
            // 한 프로세스에서 한 경로의 저장소는 하나라고 가정한다(운영 = 데몬 조립이 경로마다 하나를 만든다).
            //   같은 경로에 둘째 저장소를 만들면 그 첫 적재가 첫째 저장소의 쓰는 중인 임시 파일(이 pid 의
            //   번호 꼴)을 지울 수 있다.
            self.sweep_temps();
        }
        let name = self.kind.name;
        let (items, verdict) = match file::load(&self.path(), &self.kind.spec, LOAD_HOOKS) {
            Loaded::Missing => {
                tracing::debug!(file = name, "파일이 없다 — 빈 목록으로 시작한다");
                (Vec::new(), LoadVerdict::NotAdopted)
            }
            Loaded::Parsed(Parsed::Usable { doc, .. }) => match read_list(&doc, self.kind.list_key)
            {
                Ok(items) => (items, LoadVerdict::Adopted),
                // 모양 검사가 같은 함수를 이미 지났다 — 여기 오면 둘이 어긋난 것이라 그 파일을 우리 것으로 잡지
                //   않는다(첫 저장이 덮기 전에 떠 둔다).
                Err(reason) => {
                    tracing::error!(
                        file = name,
                        %reason,
                        "모양 검사를 지난 파일의 목록을 못 읽었다 — 빈 목록으로 시작한다"
                    );
                    (Vec::new(), LoadVerdict::NotAdopted)
                }
            },
            Loaded::Parsed(Parsed::Unusable(reason)) => {
                tracing::error!(
                    file = name,
                    %reason,
                    "파일이 손상됐다 — 빈 목록으로 시작하고, 첫 저장이 덮기 전에 .corrupt 로 떠 둔다"
                );
                (Vec::new(), LoadVerdict::NotAdopted)
            }
            // 읽기 전용은 첫 적재만 세운다 — 뒤 적재는 읽어 보기일 뿐이라 그 문구를 내지 않는다.
            Loaded::Parsed(Parsed::Newer { found }) => {
                let current = self.kind.spec.current;
                if first {
                    tracing::error!(
                        file = name,
                        found,
                        current,
                        "이 데몬보다 새 판이 쓴 파일이다 — 읽지도 덮지도 않는다. 빈 목록으로 시작하고 이 실행 \
                         동안 저장하지 않는다(데몬을 다시 띄워야 풀린다)"
                    );
                } else {
                    tracing::debug!(
                        file = name,
                        found,
                        current,
                        "새 판이 쓴 파일이라 읽지 않았다"
                    );
                }
                (Vec::new(), LoadVerdict::Refused(Refusal::Newer { found }))
            }
            Loaded::Failed(error) => {
                if first {
                    tracing::error!(
                        file = name,
                        %error,
                        "파일을 못 읽었다 — 덮지 않는다. 빈 목록으로 시작하고 이 실행 동안 저장하지 않는다\
                         (데몬을 다시 띄워야 풀린다)"
                    );
                } else {
                    tracing::debug!(file = name, %error, "파일을 못 읽었다");
                }
                (Vec::new(), LoadVerdict::Refused(Refusal::Unreadable))
            }
        };
        if first {
            let mut state = sync::lock(&self.state);
            match verdict {
                LoadVerdict::Adopted => state.claim = Claim::Adopted,
                LoadVerdict::NotAdopted => {}
                LoadVerdict::Refused(refusal) => state.load_refusal = Some(refusal),
            }
        }
        items
    }

    /// 쓰기 직전에 지금 파일을 다시 판정하고 쓴다. 거절이면 쓰지 않고 [`StoreError::ReadOnly`] · 직렬화 · 상한 ·
    /// 떠 두기 · 쓰기 실패면 [`StoreError::Io`](파일 그대로). 로그는 여기서 낸다.
    // ADR-0291 R8 · R15 · R16 · R17
    fn save(&self, count: usize, payload: &impl Serialize) -> Result<(), StoreError> {
        let name = self.kind.name;
        let (load_refusal, claim) = {
            let state = sync::lock(&self.state);
            (state.load_refusal, state.claim)
        };
        if let Some(refusal) = load_refusal {
            tracing::debug!(
                file = name,
                %refusal,
                "적재 때 거절된 파일이라 저장하지 않는다(데몬을 다시 띄워야 풀린다)"
            );
            return Err(StoreError::ReadOnly(refusal));
        }

        let failed = |what: &str, error: io::Error| {
            tracing::error!(file = name, %error, "{what} — 저장하지 않았다(파일은 그대로다)");
            StoreError::Io(error)
        };
        let bytes = serde_json::to_vec_pretty(payload).map_err(|e| {
            failed(
                "직렬화에 실패했다",
                io::Error::new(io::ErrorKind::InvalidData, e),
            )
        })?;
        file::check_cap(bytes.len(), &self.kind.spec)
            .map_err(|e| failed("쓸 원문이 읽기 상한을 넘는다", e))?;

        let path = self.path();
        let current = file::load(&path, &self.kind.spec, DATA_HOOKS);
        match current.write_policy(claim) {
            WritePolicy::Write => {}
            WritePolicy::CopyAsideFirst => {
                let what = match current {
                    Loaded::Parsed(Parsed::Usable { .. }) => {
                        "이 실행이 아직 쓰지 않은 동안 놓인 쓸 만한 파일"
                    }
                    _ => "손상된 파일",
                };
                match file::copy_aside(&path, DATA_HOOKS) {
                    Ok(to) => tracing::warn!(
                        file = name,
                        to = %to.display(),
                        "{what}을 덮기 전에 떠 뒀다"
                    ),
                    Err(error) => return Err(failed(&format!("{what}을 떠 두지 못했다"), error)),
                }
            }
            WritePolicy::Refuse(refused) => return Err(self.refused(refused, &current)),
        }

        file::write_atomic(&path, &bytes, DATA_HOOKS)
            .map_err(|e| failed("파일을 쓰지 못했다", e))?;
        let recovered = {
            let mut state = sync::lock(&self.state);
            state.claim = Claim::Adopted;
            state.last_refusal.take()
        };
        if let Some(refusal) = recovered {
            tracing::info!(file = name, %refusal, "거절 뒤 다시 저장했다");
        }
        tracing::debug!(file = name, count, "저장했다");
        Ok(())
    }

    /// 재판정 거절을 상태 칸에 남기고 로그를 낸다 — 같은 까닭이 이어지는 동안은 debug(R15).
    fn refused(&self, refused: Refused, current: &Loaded) -> StoreError {
        let name = self.kind.name;
        let refusal = match refused {
            Refused::Newer { found } => Refusal::Newer { found },
            Refused::ReadFailed => Refusal::Unreadable,
        };
        let repeated = sync::lock(&self.state).last_refusal.replace(refusal) == Some(refusal);
        if repeated {
            tracing::debug!(file = name, %refusal, "또 저장을 거절했다 — 덮지 않는다");
        } else if let Loaded::Failed(error) = current {
            tracing::error!(
                file = name,
                %error,
                "지금 파일을 못 읽어 덮지 않는다 — 이 저장을 거절한다(다음 저장이 다시 판정한다)"
            );
        } else {
            tracing::error!(
                file = name,
                %refusal,
                "지금 파일을 덮을 수 없다 — 이 저장을 거절한다(다음 저장이 다시 판정한다)"
            );
        }
        StoreError::ReadOnly(refusal)
    }

    fn status(&self) -> StoreStatus {
        let state = sync::lock(&self.state);
        match (state.load_refusal, state.last_refusal) {
            (Some(refusal), _) => StoreStatus::ReadOnly(refusal),
            (None, Some(refusal)) => StoreStatus::Refusing(refusal),
            (None, None) => StoreStatus::Writable,
        }
    }

    fn sweep_temps(&self) {
        let name = self.kind.name;
        match file::sweep_temps(&self.dir, &[name], pid_alive) {
            Ok(swept) => {
                for (path, removed) in swept {
                    match removed {
                        Ok(()) => tracing::debug!(
                            file = name,
                            path = %path.display(),
                            "남은 임시 파일을 지웠다"
                        ),
                        Err(error) => tracing::warn!(
                            file = name,
                            path = %path.display(),
                            %error,
                            "남은 임시 파일을 못 지웠다"
                        ),
                    }
                }
            }
            Err(error) => tracing::warn!(
                file = name,
                %error,
                "남은 임시 파일을 찾지 못했다 — 치우지 않고 진행한다"
            ),
        }
        let legacy = self.dir.join(self.kind.legacy_tmp);
        match fs::remove_file(&legacy) {
            Ok(()) => tracing::debug!(
                file = name,
                path = %legacy.display(),
                "옛 고정 임시 파일을 지웠다"
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(
                file = name,
                path = %legacy.display(),
                %error,
                "옛 고정 임시 파일을 못 지웠다"
            ),
        }
    }
}

/// 주인의 모양 검사(`Spec::shape`) — 적재가 같은 [`read_list`] 로 값을 얻으므로 둘의 손상 정의가 갈리지 않는다.
// ADR-0291 R7 (D1)
fn list_shape<T: DeserializeOwned>(
    kind: &FileKind,
    version: u64,
    doc: &Map<String, Value>,
) -> Result<(), String> {
    // 앞 판을 읽는 리더가 없다 — 판을 올리면 그 판 리더와 함께 이 줄을 고친다(R3).
    if version != kind.spec.current {
        return Err(format!(
            "{} {version} 을 읽는 리더가 없다(이 빌드는 {})",
            kind.spec.version_key, kind.spec.current
        ));
    }
    read_list::<T>(doc, kind.list_key).map(drop)
}

/// 목록 칸(`key`)을 읽는다. 버전 키는 필수가 아니다 — 판은 `file::parse` 가 이미 봤다(키 없음 = 1 판).
fn read_list<T: DeserializeOwned>(doc: &Map<String, Value>, key: &str) -> Result<Vec<T>, String> {
    let Some(list) = doc.get(key) else {
        return Err(format!("`{key}` 칸이 없다"));
    };
    let Some(items) = list.as_array() else {
        return Err(format!("`{key}` 칸이 배열이 아니다"));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            T::deserialize(item).map_err(|e| {
                // serde_json 오류 문구는 싣지 않는다 — 「invalid type: string "…"」처럼 칸 값을 옮겨 적어, 이
                //   사유가 로그로 나가면 프로필 env 같은 값이 샌다. 줄 · 칸은 값 트리에서 읽어 늘 0 이라 항목
                //   번호를 대신 싣는다.
                format!(
                    "`{key}` 의 {index} 번 항목을 읽지 못했다(분류 {:?})",
                    e.classify()
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::AgentCommand;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-persist-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> AgentProfile {
        AgentProfile::new(
            "t".into(),
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            PathBuf::from("."),
            vec![],
            true,
        )
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let store = FileProfileStore::new(dir.clone());
        let p = sample();
        let id = p.id;
        store.save(&[p]).unwrap();

        let loaded = store.load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, id);
        let _ = fs::remove_dir_all(&dir);
    }

    /// ★앞 릴리스로 되돌아간 바이너리가 이 파일을 읽을 수 있어야 한다★: 그 구조체는 `epoch` 를 **필수**
    /// 필드로 선언했으므로 키가 없으면 `missing field` 로 파싱이 깨지고, 그 릴리스의 persistence 는 파일을
    /// `.corrupt-<ts>` 로 치우고 빈 목록으로 시작한 뒤 다음 save 가 그 빈 목록을 덮어써 프로필·세션 id·트리
    /// 부모가 통째로 사라진다.
    /// 실리는 값은 산 표식이 아니라 고정 `0`(옛 카운터의 "재spawn 없음" 상태 — `AgentProfile::epoch` 주석).
    #[test]
    fn agents_json_carries_a_zero_epoch_key_for_older_readers() {
        let dir = temp_dir("epoch-key");
        let store = FileProfileStore::new(dir.clone());
        let mut p = sample();
        p.epoch = 0xDEAD_BEEF;
        store.save(&[p]).unwrap();

        let raw = fs::read(dir.join(PROFILES.name)).expect("agents.json 읽기");
        let v: serde_json::Value = serde_json::from_slice(&raw).expect("JSON 파싱");
        assert_eq!(
            v["profiles"][0].get("epoch"),
            Some(&serde_json::json!(0)),
            "옛 리더가 필수로 읽는 키가 0 으로 실려 있어야 한다: {v}"
        );

        assert_eq!(
            store.load()[0].epoch,
            0,
            "다시 읽어도 표식은 파일에서 오지 않는다(다음 spawn 이 새로 발급)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_is_empty() {
        let dir = temp_dir("missing");
        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert_eq!(store.status(), StoreStatus::Writable);
    }

    // ── 손상 · 새 판 · 읽기 실패 ──

    fn file_text(profiles: &[AgentProfile]) -> String {
        serde_json::to_string_pretty(&ProfilesFile {
            schema_version: 1,
            profiles,
        })
        .unwrap()
    }

    fn put(dir: &std::path::Path, name: &str, text: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), text).unwrap();
    }

    fn read(dir: &std::path::Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    fn aside_name() -> String {
        format!("{}.corrupt", PROFILES.name)
    }

    #[test]
    fn a_corrupt_file_stays_in_place_until_the_first_save_copies_it_aside() {
        let dir = temp_dir("corrupt");
        put(&dir, PROFILES.name, "{ not valid json");

        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert_eq!(
            read(&dir, PROFILES.name),
            "{ not valid json",
            "적재는 파일을 바꾸지 않는다"
        );
        assert!(!dir.join(aside_name()).exists());
        assert_eq!(store.status(), StoreStatus::Writable);

        let p = sample();
        store.save(std::slice::from_ref(&p)).unwrap();
        assert_eq!(read(&dir, &aside_name()), "{ not valid json");
        assert_eq!(FileProfileStore::new(dir.clone()).load()[0].id, p.id);
        let _ = fs::remove_dir_all(&dir);
    }

    /// ★적재 판정을 들고 있지 않는다★ — 들고 있으면 첫 저장 뒤 우리 정상 파일을 저장마다 떠 두어 진짜 손상
    /// 사본을 지운다.
    #[test]
    fn after_the_first_save_the_file_is_not_copied_aside_again() {
        let dir = temp_dir("corrupt-once");
        put(&dir, PROFILES.name, "{ not valid json");
        let store = FileProfileStore::new(dir.clone());
        store.load();
        store.save(&[sample()]).unwrap();
        assert_eq!(read(&dir, &aside_name()), "{ not valid json");

        store.save(&[sample(), sample()]).unwrap();
        assert_eq!(
            read(&dir, &aside_name()),
            "{ not valid json",
            "둘째 저장이 우리 파일을 떠 두어 손상 사본을 덮었다"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_with_one_unreadable_item_is_corrupt_and_copied_aside_before_the_first_save() {
        let dir = temp_dir("bad-kind");
        let mut doc: Value = serde_json::from_str(&file_text(&[sample(), sample()])).unwrap();
        doc["profiles"][1]["command"]["kind"] = Value::from("NoSuchBackend");
        let text = doc.to_string();
        put(&dir, PROFILES.name, &text);

        let store = FileProfileStore::new(dir.clone());
        assert!(
            store.load().is_empty(),
            "항목 하나가 못 쓰면 파일 전체가 손상이다"
        );
        store.save(&[sample()]).unwrap();
        assert_eq!(read(&dir, &aside_name()), text);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_file_is_neither_read_nor_overwritten_for_the_whole_run() {
        let dir = temp_dir("newer");
        let newer = r#"{"schema_version":999,"profiles":[]}"#;
        put(&dir, PROFILES.name, newer);

        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        let refusal = Refusal::Newer { found: 999 };
        assert_eq!(store.status(), StoreStatus::ReadOnly(refusal));
        assert!(matches!(
            store.save(&[sample()]),
            Err(StoreError::ReadOnly(r)) if r == refusal
        ));
        assert_eq!(read(&dir, PROFILES.name), newer);
        assert!(
            !dir.join(aside_name()).exists(),
            "새 판 파일은 떠 두지도 않는다"
        );

        // 메모리 명부가 그 파일에서 오지 않았다 — 파일이 쓸 만해져도 이 실행은 그 위에 쓰지 않는다.
        let ours = file_text(&[sample()]);
        put(&dir, PROFILES.name, &ours);
        assert!(store.save(&[]).is_err());
        assert_eq!(read(&dir, PROFILES.name), ours);
        assert_eq!(store.status(), StoreStatus::ReadOnly(refusal));

        assert_eq!(
            FileProfileStore::new(dir.clone()).load().len(),
            1,
            "다시 띄우면 풀린다"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 파일 자리에 폴더를 두어 읽기를 실패시킨다(Windows = 접근 거부 → 적재 예산 동안 다시 하고 그대로 실패).
    #[test]
    fn an_unreadable_file_keeps_the_store_read_only_even_after_it_goes_away() {
        let dir = temp_dir("unreadable");
        fs::create_dir_all(dir.join(PROFILES.name)).unwrap();

        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        let refusal = Refusal::Unreadable;
        assert_eq!(store.status(), StoreStatus::ReadOnly(refusal));

        fs::remove_dir(dir.join(PROFILES.name)).unwrap();
        assert!(matches!(
            store.save(&[sample()]),
            Err(StoreError::ReadOnly(r)) if r == refusal
        ));
        assert!(
            !dir.join(PROFILES.name).exists(),
            "없어진 뒤에도 빈 명부를 쓰면 안 된다"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rejudge_refusal_shows_as_refusing_until_a_save_succeeds() {
        let dir = temp_dir("refusing");
        let store = FileProfileStore::new(dir.clone());
        store.load();
        store.save(&[sample()]).unwrap();

        let newer = r#"{"schema_version":2,"profiles":[]}"#;
        put(&dir, PROFILES.name, newer);
        let refusal = Refusal::Newer { found: 2 };
        assert!(matches!(
            store.save(&[sample()]),
            Err(StoreError::ReadOnly(r)) if r == refusal
        ));
        assert_eq!(store.status(), StoreStatus::Refusing(refusal));
        assert_eq!(read(&dir, PROFILES.name), newer);

        put(&dir, PROFILES.name, &file_text(&[]));
        let p = sample();
        store.save(std::slice::from_ref(&p)).unwrap();
        assert_eq!(store.status(), StoreStatus::Writable);
        assert_eq!(FileProfileStore::new(dir.clone()).load()[0].id, p.id);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 적재 때 없던 파일을 실행 중에 누가 되살려 두었으면 첫 저장이 덮기 전에 떠 둔다 — 한 번만.
    #[test]
    fn a_usable_file_placed_after_a_missing_load_is_copied_aside_once() {
        let dir = temp_dir("not-yet");
        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());

        let restored = file_text(&[sample()]);
        put(&dir, PROFILES.name, &restored);
        store.save(&[sample(), sample()]).unwrap();
        assert_eq!(read(&dir, &aside_name()), restored);

        fs::remove_file(dir.join(aside_name())).unwrap();
        store.save(&[sample()]).unwrap();
        assert!(
            !dir.join(aside_name()).exists(),
            "한 번 쓴 뒤의 파일은 우리 것이다"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // ── 읽기 규칙 ──

    #[test]
    fn a_bom_prefixed_file_loads() {
        let dir = temp_dir("bom");
        let p = sample();
        put(
            &dir,
            PROFILES.name,
            &format!("\u{feff}{}", file_text(std::slice::from_ref(&p))),
        );
        assert_eq!(FileProfileStore::new(dir.clone()).load()[0].id, p.id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_without_a_version_key_loads_as_the_first_version() {
        let dir = temp_dir("no-version");
        let p = sample();
        let mut doc: Value = serde_json::from_str(&file_text(std::slice::from_ref(&p))).unwrap();
        doc.as_object_mut().unwrap().remove("schema_version");
        put(&dir, PROFILES.name, &doc.to_string());

        let store = FileProfileStore::new(dir.clone());
        assert_eq!(store.load()[0].id, p.id);
        store.save(&[p]).unwrap();
        assert!(
            !dir.join(aside_name()).exists(),
            "적재한 파일은 우리 것이라 떠 두지 않는다"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_zero_is_corrupt_not_refused() {
        let dir = temp_dir("version-zero");
        put(&dir, PROFILES.name, r#"{"schema_version":0,"profiles":[]}"#);
        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert_eq!(store.status(), StoreStatus::Writable);
        store.save(&[]).unwrap();
        assert!(dir.join(aside_name()).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// 모양 실패 사유가 로그로 나간다 — 칸 값(프로필 env 같은 것)을 옮겨 적으면 안 된다.
    #[test]
    fn a_shape_failure_reason_does_not_echo_field_values() {
        let mut doc: Value = serde_json::from_str(&file_text(&[sample()])).unwrap();
        doc["profiles"][0]["auto_restore"] = Value::from("TOP-SECRET-VALUE");
        match file::parse(&doc.to_string(), &PROFILES.spec) {
            Parsed::Unusable(reason) => {
                assert!(!reason.contains("TOP-SECRET"), "{reason}");
                assert!(reason.contains("0 번 항목"), "{reason}");
            }
            other => panic!("손상이어야 한다: {other:?}"),
        }
    }

    // ── 기동 쓸기 ──

    #[test]
    fn the_first_load_sweeps_the_old_fixed_temp_and_our_own_numbered_temps() {
        let dir = temp_dir("sweep");
        let me = std::process::id();
        let own = format!("{}.tmp{me}.7", PROFILES.name);
        put(&dir, PROFILES.legacy_tmp, "old");
        put(&dir, &own, "half");
        put(&dir, "agents.json.tmpx", "not ours");

        let store = FileProfileStore::new(dir.clone());
        store.load();
        assert!(!dir.join(PROFILES.legacy_tmp).exists());
        assert!(!dir.join(&own).exists());
        assert!(
            dir.join("agents.json.tmpx").exists(),
            "우리 꼴이 아닌 이름은 둔다"
        );

        // 뒤 적재는 쓸지 않는다 — 그 사이 진행 중인 저장의 임시 파일을 지우면 그 rename 이 실패한다.
        put(&dir, &own, "in flight");
        store.load();
        assert!(dir.join(&own).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    // ── 운영 OS 몫 ──

    /// ★아무것도 안 하는 다시 하기를 넘겨도 컴파일은 선다★ — 그 실수를 이 시험이 잡는다.
    #[test]
    fn both_os_hooks_really_retry_a_locked_attempt() {
        for (label, hooks) in [("데이터", DATA_HOOKS), ("적재 읽기", LOAD_HOOKS)] {
            let mut attempts = 0;
            let outcome = (hooks.retry)(&mut || {
                attempts += 1;
                if attempts == 1 {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    Ok(())
                }
            });
            assert!(outcome.is_ok(), "{label}: {outcome:?}");
            assert_eq!(attempts, 2, "{label}");
        }
    }

    /// 같은 예산을 둘에 넘기는 실수를 잡는다 — 잠김이 안 풀리면 각자의 예산만큼 다시 하고 포기한다.
    #[test]
    fn the_load_read_hook_waits_longer_than_the_data_hook() {
        let attempts_until_given_up = |hooks: OsHooks| {
            let mut attempts = 0;
            let outcome = (hooks.retry)(&mut || {
                attempts += 1;
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            });
            assert!(outcome.is_err());
            attempts
        };
        assert_eq!(attempts_until_given_up(DATA_HOOKS), BUSY_RETRIES + 1);
        assert_eq!(attempts_until_given_up(LOAD_HOOKS), LOAD_RETRIES + 1);
        assert!(LOAD_PAUSE * LOAD_RETRIES > BUSY_PAUSE * BUSY_RETRIES);
    }
}
