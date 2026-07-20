//! 抽奖：服务端权威的结果生成与奖池扣减。
//!
//! **结果 100% 由服务端产生，前端只播动画。** 这条不是防作弊洁癖 —— 转盘的
//! 中奖结果直接决定奖池成本，也决定后续能否签发领奖码（进而决定一笔可计费
//! 转化是否成立）。把结果交给前端算，等于把奖池和账单都交给用户改。

pub mod play;

use rand::Rng;

/// 参与抽奖的一个奖项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prize {
    pub id: i64,
    pub label: String,
    pub weight: i64,
    /// 剩余库存。为 0 的奖项不参与抽奖。
    pub remaining: i64,
}

impl Prize {
    /// 该奖项此刻是否可被抽中。
    fn available(&self) -> bool {
        self.weight > 0 && self.remaining > 0
    }
}

/// 按权重抽取一个奖项，返回其在 `prizes` 中的下标。
///
/// `roll` 由调用方注入而非在函数内取随机数 —— 与 `telegram::verify` 注入 `now`
/// 同样的理由：概率逻辑必须能被确定性地测试，否则「权重为 0 的奖项会不会被
/// 抽中」这类问题只能靠跑一万次碰运气来回答。
///
/// `roll` 的取值范围是 `[0, total_weight)`，`total_weight` 为所有**有库存**
/// 奖项的权重之和。库存为 0 的奖项在计算总权重时就被排除，因此不会出现
/// 「抽中了一个没货的奖」再回退的情况。
pub fn draw_with(prizes: &[Prize], roll: i64) -> Option<usize> {
    let total = total_weight(prizes);
    if total == 0 {
        return None;
    }

    let mut cursor = roll.rem_euclid(total);
    for (i, p) in prizes.iter().enumerate() {
        if !p.available() {
            continue;
        }
        if cursor < p.weight {
            return Some(i);
        }
        cursor -= p.weight;
    }
    // 累加逻辑保证 cursor < total 时一定会命中某个区间，这里不可达。
    None
}

/// 有库存奖项的权重总和。
pub fn total_weight(prizes: &[Prize]) -> i64 {
    prizes
        .iter()
        .filter(|p| p.available())
        .map(|p| p.weight)
        .sum()
}

/// 用密码学安全随机源抽取一个奖项。
///
/// 用 CSPRNG 而不是普通伪随机：奖项对应真实成本，可预测的序列意味着有人能
/// 掐着点去抽限定奖。
pub fn draw(prizes: &[Prize]) -> Option<usize> {
    let total = total_weight(prizes);
    if total == 0 {
        return None;
    }
    let roll = rand::thread_rng().gen_range(0..total);
    draw_with(prizes, roll)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prizes() -> Vec<Prize> {
        vec![
            Prize {
                id: 1,
                label: "100 金币".into(),
                weight: 70,
                remaining: 100,
            },
            Prize {
                id: 2,
                label: "500 金币".into(),
                weight: 25,
                remaining: 100,
            },
            Prize {
                id: 3,
                label: "限定皮肤".into(),
                weight: 5,
                remaining: 100,
            },
        ]
    }

    #[test]
    fn roll_lands_in_the_right_weight_bucket() {
        let p = prizes();
        // 区间：[0,70) → 0，[70,95) → 1，[95,100) → 2
        for (roll, want) in [(0, 0), (69, 0), (70, 1), (94, 1), (95, 2), (99, 2)] {
            assert_eq!(draw_with(&p, roll), Some(want), "roll={roll}");
        }
    }

    /// 库存为 0 的奖项不参与抽奖，且不占用权重区间 —— 否则会出现
    /// 「抽中了一个没货的奖」，只能回退重抽，中奖概率也会悄悄偏离配置。
    #[test]
    fn sold_out_prizes_are_excluded_from_the_weight_space() {
        let mut p = prizes();
        p[0].remaining = 0;

        assert_eq!(total_weight(&p), 30, "总权重应只剩 25 + 5");
        for (roll, want) in [(0, 1), (24, 1), (25, 2), (29, 2)] {
            assert_eq!(draw_with(&p, roll), Some(want), "roll={roll}");
        }
    }

    #[test]
    fn zero_weight_prizes_are_never_drawn() {
        let mut p = prizes();
        p[2].weight = 0;

        for roll in 0..total_weight(&p) {
            assert_ne!(draw_with(&p, roll), Some(2), "权重为 0 的奖项被抽中了");
        }
    }

    #[test]
    fn everything_sold_out_yields_nothing() {
        let mut p = prizes();
        for x in &mut p {
            x.remaining = 0;
        }
        assert_eq!(draw_with(&p, 0), None);
        assert_eq!(draw(&p), None);
        assert_eq!(total_weight(&p), 0);
    }

    #[test]
    fn empty_pool_yields_nothing() {
        assert_eq!(draw_with(&[], 0), None);
        assert_eq!(draw(&[]), None);
    }

    /// 越界的 roll 不应 panic，也不应偏向某一端。
    #[test]
    fn out_of_range_rolls_wrap_around() {
        let p = prizes();
        assert_eq!(draw_with(&p, 100), draw_with(&p, 0));
        assert_eq!(draw_with(&p, -1), draw_with(&p, 99));
    }

    /// 真随机路径只保证「落在合法范围内」；概率分布由上面的确定性用例覆盖。
    #[test]
    fn csprng_draw_stays_in_range() {
        let p = prizes();
        for _ in 0..200 {
            let i = draw(&p).expect("有库存时必有结果");
            assert!(i < p.len());
            assert!(p[i].available());
        }
    }
}
