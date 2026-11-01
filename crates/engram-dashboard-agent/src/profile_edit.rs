//! 프로필 편집 포트 — 라이브러리가 에이전트 붙임을 고치는 유일한 길(`ProfileRegistry` 를 직접 고치지 않는다).
//! agent 가 정의하고 `AgentManager` 가 구현한다 — 명부 변경과 스폰 예약이 manager 안의 한 락 순서를 나눈다.
// ADR-0311

use crate::profile::{AttachedPreset, StoreError};
use crate::types::AgentId;

/// 에이전트에 붙은 프리셋 — 프로필의 「기본」 칸과 붙인 순서의 목록을 한 값으로 나른다.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Attachments {
    pub default_preset: bool,
    pub presets: Vec<AttachedPreset>,
}

/// 라이브러리 경로 바꿔 쓰기 한 줄 — 붙은 경로 전체가 `from` 과 platform `same_path` 로 같을 때만 `to` 로 바꾼다
/// (대소문자는 OS 규칙 · 접두 맞춤 없음 — 폴더 이동은 부르는 쪽이 항목마다 펼쳐 넘긴다).
// ADR-0325
// ADR-0335
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetPathRename {
    pub from: String,
    pub to: String,
}

/// 「작업 폴더」 가 잠긴 까닭.
// ADR-0321
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceLock {
    /// 떠 있다.
    Running,
    /// 실행 준비 중이다(스폰 예약이 잡혀 있다).
    Preparing,
    /// 대화했다(영속된 backend 세션 id 가 있다) — 그 에이전트에서는 풀리지 않는다.
    Conversed,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileEditError {
    #[error("명부에 없는 에이전트 {0}")]
    NotFound(AgentId),
    /// 저장이 실패해 아무것도 바뀌지 않았다.
    #[error("프로필 저장 실패 — {0}")]
    Store(#[from] StoreError),
}

/// 라이브러리 → 명부 변경 포트.
///
/// 부르는 쪽(asset-library)은 자기 락을 쥔 채 부를 수 있다 — 그래서 구현은 실행 재료 포트(라이브러리)를 되부르지
/// 않는다(asset-library → manager 한 방향).
// ADR-0311
// ADR-0324
pub trait ProfileEditPort: Send + Sync + 'static {
    /// 이 에이전트의 붙임을 `next` 로 통째 바꾼다 — 명부 변경 한 번.
    ///
    /// ★대조 없이 나중 쓰기가 이긴다★ — 있는 에이전트의 붙임을 쓰는 쪽이 전부 라이브러리 쓰기 락 아래 줄 서기에만
    /// 맞다(복제 · `agent.new` 는 새 id 에 쓰고, 스폰 병합은 붙임을 보존한다).
    fn set_attachments(&self, id: AgentId, next: Attachments) -> Result<(), ProfileEditError>;

    /// **지금** 명부 전체에서 붙은 경로를 `renames` 대로 바꾼다 — 명부 락 한 번 보유 안에서. 돌려주는 것 = 바뀐
    /// 에이전트. 에이전트 목록을 받지 않는 것은 의도다 — 미리 뽑은 목록은 그 사이 복제된 사본을 놓친다.
    ///
    /// `renames` 는 한꺼번에 걸리는 대응 하나다 — 각 줄은 **원래** 값에만 맞추고, 붙은 경로 하나는 많아야 한 번
    /// 바뀐다(이어 걸리지 않는다: `a → b` · `b → c` 면 `a` 는 `b` 가 된다, `c` 가 아니다). `same_path` 로 같은
    /// `from` 이 여러 줄이면 앞 줄이 이긴다(뒤 줄은 버린다). 값이 글자 그대로 바뀐 에이전트만 id 순으로 돌려준다 —
    /// 제자리 대응(`a → a`)은 바꾼 것이 아니고, 대소문자만 고친 것(`a → A`)은 바꾼 것이다. 바뀐 에이전트의 목록은
    /// `same_path` 로 같은 경로를 첫 자리 하나만 남긴다(`[a, b]` 에 `a → b` 면 `[b]`).
    // ADR-0335
    // ADR-0339
    fn rewrite_preset_paths(
        &self,
        renames: &[PresetPathRename],
    ) -> Result<Vec<AgentId>, ProfileEditError>;

    /// **지금** 명부 전체에서 붙은 경로 중 `paths` 의 어느 것과 `same_path` 로 같은 항목을 걷는다 — 명부 락 한 번
    /// 보유 안에서. 돌려주는 것 = 목록이 줄어든 에이전트(id 순). 에이전트 목록을 받지 않는 까닭은
    /// [`Self::rewrite_preset_paths`] 와 같다. 남는 항목의 순서와 「기본」 칸은 그대로다. 경로 전체로만 맞춘다
    /// (접두 맞춤 없음 — 폴더 지우기는 부르는 쪽이 붙은 항목마다 펼쳐 넘긴다).
    // ADR-0325
    // ADR-0339
    fn remove_preset_paths(&self, paths: &[String]) -> Result<Vec<AgentId>, ProfileEditError>;

    /// 이 에이전트의 「작업 폴더」 가 잠겼나 — `None` = 바꿀 수 있다. `agent.setWorkspace` 와 같은 판정이다.
    /// 까닭이 여럿이면 `Running` > `Preparing` > `Conversed` 순으로 앞선 하나만 답한다.
    ///
    /// ★실행 전처리 동안은 그 실행의 예약이 잡혀 있다★ — [`crate::launch::LaunchKitSource::check`] 가 띄우는 바로 그
    /// 에이전트를 물으면 `Preparing` 을 받는다.
    fn workspace_lock(&self, id: AgentId) -> Result<Option<WorkspaceLock>, ProfileEditError>;
}
