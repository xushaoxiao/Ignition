package attribution

import (
	"crypto/rand"
	"math/big"
	"strings"
)

// claimAlphabet 领奖码字符集。
//
// 刻意排除 0/O/1/I/l —— iOS 上用户需要手动输入领奖码，而那是 iOS 侧唯一可计费
// 的归因路径。一个字符的误读就是一次收入损失，所以宁可牺牲码空间。
// 32 个字符 × 8 位 ≈ 1.1e12 组合，配合限流足够抗枚举。
const claimAlphabet = "23456789ABCDEFGHJKMNPQRSTUVWXYZ"

const claimCodeLen = 8

// NewClaimCode 生成一个领奖码。使用 CSPRNG —— 领奖码直接对应可计费转化，
// 可预测的码等于可伪造的收入。
func NewClaimCode() (string, error) {
	return randomString(claimAlphabet, claimCodeLen)
}

// trackingAlphabet 投放位 ID 的字符集。
const trackingAlphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"

const trackingIDLen = 10

// NewTrackingID 生成投放位 ID。
//
// 必须不可枚举：如果 KOL 能猜出别人的链接，就能互相刷量或窃取归因。
// 这里不做人类可读优化 —— tracking_id 从不需要手动输入。
func NewTrackingID() (string, error) {
	return randomString(trackingAlphabet, trackingIDLen)
}

// NormalizeClaimCode 把用户输入的领奖码规整为存储形态：大小写不敏感、
// 去除用户或输入法可能带入的分隔符与空白。
//
// 手动输入是 iOS 侧的必经路径，容错直接影响核销完成率（W7 的核心验证指标）。
// 这里刻意不做混淆字符映射：字符集本身已排除 0/O/1/I/L，所以这几个字符
// 出现在输入里必定是误输入，而 "0" 究竟想打 O 还是 D 无法可靠推断。
// 猜错会把一个有效码变成另一个有效码，归到错误的 KOL 名下 —— 宁可报错重输。
func NormalizeClaimCode(in string) string {
	var b strings.Builder
	b.Grow(len(in))
	for _, r := range strings.ToUpper(strings.TrimSpace(in)) {
		switch r {
		case ' ', '-', '_', '\t', '\n', '\r', '.':
			continue
		}
		b.WriteRune(r)
	}
	return b.String()
}

// ValidClaimCodeFormat 报告 code 是否符合领奖码的格式。
//
// 在查库前做格式校验，可以让绝大多数枚举尝试和输入错误在不消耗数据库资源的
// 情况下被拒绝，同时给用户一个比"码不存在"更有用的错误。
func ValidClaimCodeFormat(code string) bool {
	if len(code) != claimCodeLen {
		return false
	}
	for _, r := range code {
		if !strings.ContainsRune(claimAlphabet, r) {
			return false
		}
	}
	return true
}

func randomString(alphabet string, n int) (string, error) {
	max := big.NewInt(int64(len(alphabet)))
	b := make([]byte, n)
	for i := range b {
		idx, err := rand.Int(rand.Reader, max)
		if err != nil {
			return "", err
		}
		b[i] = alphabet[idx.Int64()]
	}
	return string(b), nil
}
