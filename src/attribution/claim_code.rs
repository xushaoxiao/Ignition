//! 领奖码与投放位 ID 的生成、归一化与格式校验。

use rand::Rng;

/// 领奖码字符集。
///
/// 刻意排除 `0 O 1 I L` —— iOS 上用户需要手动输入领奖码，而那是 iOS 侧唯一
/// 可计费的归因路径。一个字符的误读就是一次收入损失，所以宁可牺牲码空间。
/// 31 个字符 × 8 位 ≈ 8.5e11 组合，配合限流足够抗枚举。
const CLAIM_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CLAIM_CODE_LEN: usize = 8;

/// 投放位 ID 的字符集。
const TRACKING_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const TRACKING_ID_LEN: usize = 10;

/// 生成一个领奖码。
///
/// 使用密码学安全的随机源 —— 领奖码直接对应可计费转化，可预测的码等于
/// 可伪造的收入。
pub fn new_claim_code() -> String {
    random_string(CLAIM_ALPHABET, CLAIM_CODE_LEN)
}

/// 生成投放位 ID。
///
/// 必须不可枚举：如果 KOL 能猜出别人的链接，就能互相刷量或窃取归因。
/// 这里不做人类可读优化 —— tracking_id 从不需要手动输入。
pub fn new_tracking_id() -> String {
    random_string(TRACKING_ALPHABET, TRACKING_ID_LEN)
}

/// 把用户输入的领奖码规整为存储形态：大小写不敏感、去除用户或输入法可能
/// 带入的分隔符与空白。
///
/// 手动输入是 iOS 侧的必经路径，容错直接影响核销完成率（W7 的核心验证指标）。
///
/// **刻意不做混淆字符映射**：字符集本身已排除 `0 O 1 I L`，所以这几个字符
/// 出现在输入里必定是误输入，而 `0` 究竟想打 `O` 还是 `D` 无法可靠推断。
/// 猜错会把一个有效码变成另一个有效码，归到错误的 KOL 名下 —— 宁可报错重输。
pub fn normalize_claim_code(input: &str) -> String {
    input
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_' | '\t' | '\n' | '\r' | '.'))
        .flat_map(char::to_uppercase)
        .collect()
}

/// 领奖码是否符合格式。
///
/// 在查库前做格式校验，可以让绝大多数枚举尝试和输入错误在不消耗数据库资源
/// 的情况下被拒绝，同时给用户一个比「码不存在」更有用的错误。
pub fn is_valid_claim_code(code: &str) -> bool {
    code.len() == CLAIM_CODE_LEN && code.bytes().all(|b| CLAIM_ALPHABET.contains(&b))
}

fn random_string(alphabet: &[u8], len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| alphabet[rng.gen_range(0..alphabet.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 领奖码字符集必须排除易混字符：手动输入是 iOS 侧唯一可计费的归因路径，
    /// 一个字符的误读就是一次收入损失。
    #[test]
    fn claim_code_excludes_confusable_chars() {
        for _ in 0..500 {
            let code = new_claim_code();
            assert_eq!(code.len(), 8);
            for bad in ['0', 'O', '1', 'I', 'L'] {
                assert!(!code.contains(bad), "码 {code} 含易混字符 {bad}");
            }
        }
    }

    #[test]
    fn claim_codes_do_not_repeat() {
        let codes: HashSet<String> = (0..200).map(|_| new_claim_code()).collect();
        assert_eq!(codes.len(), 200, "领奖码出现重复");
    }

    /// tracking_id 必须不可枚举：能猜出别人的链接就能互相刷量或窃取归因。
    #[test]
    fn tracking_ids_are_unpredictable() {
        let ids: HashSet<String> = (0..300).map(|_| new_tracking_id()).collect();
        assert_eq!(ids.len(), 300, "tracking_id 出现重复");
        assert!(ids.iter().all(|id| id.len() == 10));
    }

    #[test]
    fn normalize_handles_common_input_noise() {
        for (input, want) in [
            ("ab3xy9zk", "AB3XY9ZK"),
            ("AB3X-Y9ZK", "AB3XY9ZK"),
            ("ab3x y9zk", "AB3XY9ZK"),
            (" AB3XY9ZK ", "AB3XY9ZK"),
            ("ab3x_y9zk", "AB3XY9ZK"),
        ] {
            assert_eq!(normalize_claim_code(input), want, "输入 {input:?}");
        }
    }

    /// 刻意不做混淆字符映射：把 `0` 猜成 `O` 或 `D` 都可能把一个有效码变成
    /// 另一个有效码，从而归到错误的 KOL 名下。宁可报格式错让用户重输。
    #[test]
    fn normalize_does_not_guess_confusables() {
        let got = normalize_claim_code("AB0XY1ZK");

        assert_eq!(got, "AB0XY1ZK", "不应擅自替换 0 和 1");
        assert!(!is_valid_claim_code(&got), "应被格式校验拒绝");
    }

    #[test]
    fn format_validation() {
        assert!(is_valid_claim_code(&new_claim_code()));

        assert!(!is_valid_claim_code(""), "空码");
        assert!(!is_valid_claim_code("AB3XY9Z"), "长度不足");
        assert!(!is_valid_claim_code("AB3XY9ZKQ"), "长度超出");
        assert!(!is_valid_claim_code("ab3xy9zk"), "小写未规整");
        assert!(!is_valid_claim_code("00000000"), "含集外字符");
    }
}
