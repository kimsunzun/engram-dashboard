//! 시험용 로그 관측 하네스 — ★WARN·ERROR 이벤트의 **필드 텍스트**를 모은다★.
//!
//! ★왜 레벨을 둘 다 잡나★: 이것을 쓰는 시험들이 재는 것은 「어느 레벨로 냈나」가 아니라 「그 사건이
//! 남았나 / 그 필드에 무엇이 갔나」다. 레벨별로 함수를 쪼개면 같은 질문에 하네스가 둘 생긴다.
//!
//! 진입점: [`capture_loud`](동기 본문) · [`capture_loud_async`](future 본문).
//!
//! ## ★덮는 범위 — 「이 crate 의 로그 관측 하네스는 여기 하나」가 **아니다**★
//!
//! 이 모듈이 흡수한 것은 **WARN|ERROR 을 레벨 구분 없이 필드 텍스트로** 모으는 갈래 하나뿐이다. 손으로 쓴
//! 구독자가 셋 남아 있고, 셋 다 **레벨 축이 달라** 이 함수로 그대로 갈아탈 수 없다:
//!
//! - `command_roster` 의 `capture_info` — INFO **만** 잡는다(다른 레벨이 섞이면 그 시험이 재는 「그 한 줄」이 흐려진다).
//! - `connection_core` 의 `capture_logs` — `≤DEBUG` 를 잡고 **레벨을 값으로 함께** 돌려준다(레벨 자체가 단언 대상이다).
//! - `control::mod` 의 `WarnCollector` — WARN **만** 잡는다(ERROR 이 섞이면 그 시험이 보는 「그 WARN 이 났나」가
//!   흐려진다). 시험 함수 **안**에 사는 유일한 사본이다.
//!
//! ★합치려면 레벨 술어와 반환 모양을 인자로 열어야 하고, 그건 「같은 질문에 하네스 하나」를 사는 대신
//! **하네스 자체를 설정 가능한 것으로** 만드는 거래다★ — 이 라운드에서 그 거래를 하지 않았다. 여기 새 갈래를
//! 더할 때는 위 셋 중 하나로 되는지 먼저 볼 것(넷째 사본이 최악이다).
//!
//! ## ★관측 조건 — 이것을 모르면 **빈손이 실패로 보인다**★
//!
//! `with_default` 는 **스레드 로컬**이다. 여기서 나오는 두 하자를 둘 다 알고 써야 한다.
//!
//! ① **스레드를 가로지르는 callsite 관심 하자**(tracing 0.1.44 / tracing-core 0.1.36 실측). 관심 캐시가
//!    끈적한 것은 아니다 — `with_default` 는 `Dispatch::new` 를 거치고 그것이
//!    `callsite::register_dispatch` → `CALLSITES.rebuild_interest` 를 불러 **매 capture 머리에서** 등록된
//!    모든 callsite 의 관심을 다시 계산한다. ★그래서 「한 번이라도 밖에서 때리면 그 자리가 영영 죽는다」는
//!    틀렸다 — 그렇게 적어 두면 시험해 본 사람이 이 감싸개를 미신으로 보고 걷어낸다.★
//!    진짜 하자는 좁고 스레드를 가로지른다: 살아 있는 dispatcher 가 하나뿐인 동안
//!    (`Dispatchers::rebuilder` → `Rebuilder::JustOne`) callsite 의 **최초 등록**은 `dispatcher::get_default`
//!    로 **그 스레드의** 기본 구독자에게 묻는다. 그래서 다른 스레드가 capture 한복판인 사이 **구독자 없는
//!    스레드가 그 자리를 처음 등록하면** `NoSubscriber::register_callsite` 가 `Interest::never` 를 주고
//!    뒤따를 rebuild 가 없어 **그 capture 만 빈손**이 된다(단언이 조용히 실패한다).
//!    ★그래서 규율은 하나다 — **그 callsite 를 때리는 시험은 전부 capture 안에서 돌린다**★. 밖에서 때리는
//!    시험이 하나도 없으면 위 「구독자 없는 스레드」가 아예 없다. 이렇게 깨졌을 때 고칠 것은 관측 조건이고,
//!    관측하려던 코드는 멀쩡하다.
//! ② **[`capture_loud_async`] 는 자기 런타임 밖의 로그를 못 본다.** 현재 스레드 런타임이라 spawn 된 태스크는
//!    같은 스레드에서 돌지만, `spawn_blocking` 풀로 나간 본문은 **다른 스레드**라 그 로그가 이 구독자에게
//!    오지 않는다. 그 갈래를 재려면 로그가 아닌 관측 표면(반환값·셈)을 쓸 것.

use std::sync::{Arc, Mutex};

struct Collector {
    lines: Arc<Mutex<Vec<String>>>,
}

struct Visit<'a>(&'a mut String);

impl tracing::field::Visit for Visit<'_> {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={:?} ", f.name(), v));
    }
}

impl tracing::subscriber::Subscriber for Collector {
    fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
        matches!(*m.level(), tracing::Level::WARN | tracing::Level::ERROR)
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
        tracing::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut buf = String::new();
        event.record(&mut Visit(&mut buf));
        self.lines.lock().expect("lines poisoned").push(buf);
    }
    fn enter(&self, _: &tracing::Id) {}
    fn exit(&self, _: &tracing::Id) {}
}

/// 동기 본문이 내는 큰 소리를 모은다.
///
/// ★`with_default` 는 **이 스레드에만** 걸린다★ — 그래서 본문이 다른 스레드에서 로그를 내면 안 잡힌다.
/// 그 제약을 async 본문에서 어떻게 다루는지는 [`capture_loud_async`].
pub(crate) fn capture_loud<T>(body: impl FnOnce() -> T) -> (T, Vec<String>) {
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let collector = Collector {
        lines: lines.clone(),
    };
    let out = tracing::subscriber::with_default(collector, body);
    let captured = lines.lock().expect("lines poisoned").clone();
    (out, captured)
}

/// future 본문이 내는 큰 소리를 모은다.
///
/// ★런타임을 **이 안에서** 만들어 몸통을 통째로 감싼다★: 위 스레드 제약 때문이다 — 현재 스레드 런타임이면
/// spawn 된 태스크도 같은 스레드에서 돌아 그 로그가 이 구독자에게 온다. 그래서 이 함수는 `async` 가 아니다
/// (런타임 안에서 부르면 런타임 중첩이다).
pub(crate) fn capture_loud_async<F: std::future::Future<Output = ()>>(body: F) -> Vec<String> {
    let (_, lines) = capture_loud(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("현재 스레드 런타임")
            .block_on(body);
    });
    lines
}
