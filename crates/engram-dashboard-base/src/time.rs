// ADR-0269

use std::time::{SystemTime, UNIX_EPOCH};

/// 벽시계 epoch 밀리초. 시계가 1970 보다 앞이면 0 이다 — 실패를 돌려주지 않는다.
pub fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::now_epoch_ms;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn wall_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("시험 기계의 시계는 1970 뒤다")
            .as_millis() as i64
    }

    /// 단위가 밀리초인지 잰다 — 초나 마이크로초면 앞뒤 읽기 사이에 들지 못한다.
    #[test]
    fn the_reading_falls_between_two_wall_clock_reads_in_milliseconds() {
        let before = wall_ms();
        let now = now_epoch_ms();
        let after = wall_ms();
        assert!(
            before <= now && now <= after,
            "{before} <= {now} <= {after}"
        );
    }

    #[test]
    fn the_reading_is_after_2020() {
        assert!(now_epoch_ms() > 1_577_836_800_000);
    }
}
