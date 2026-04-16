//! ユーティリティモジュール

/// 非常に軽量な xorshift32 による疑似乱数生成器。
/// 主にホワイトノイズなどの音声合成や、高速な乱数が必要な場面で使用します。
#[derive(Clone, Debug)]
pub struct RandomXorShift32 {
    state: u32,
}

impl RandomXorShift32 {
    /// 新しいジェネレータを作成します。
    /// シードが 0 の場合は意図せず 0 が出続けるのを防ぐため 1 に置換します。
    pub fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    /// 次のランダムな 32 bit 符号なし整数を返します。
    pub fn next_u32(&mut self) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state
    }

    /// -1.0 〜 1.0 の範囲の浮動小数点数の乱数を返します。(音声信号用)
    pub fn next_f32_signed(&mut self) -> f32 {
        // next_u32 を i32 にキャストして、それを 2147483647.0 で割ることで [-1.0, 1.0] に近い範囲にする
        let n: u32 = self.next_u32();
        (n as i32) as f32 / 2_147_483_647.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xorshift_sequence() {
        let mut rng = RandomXorShift32::new(1);
        let a = rng.next_u32();
        let b = rng.next_u32();
        assert_ne!(a, b);
        assert_ne!(a, 0);
    }

    #[test]
    fn test_xorshift_seed_0() {
        let mut rng = RandomXorShift32::new(0);
        assert_ne!(rng.next_u32(), 0); // 0は回避される
    }

    #[test]
    fn test_xorshift_f32_signed() {
        let mut rng = RandomXorShift32::new(12345);
        let mut min = 1.0f32;
        let mut max = -1.0f32;
        
        for _ in 0..1000 {
            let val = rng.next_f32_signed();
            assert!(val >= -1.0001 && val <= 1.0001);
            if val < min { min = val; }
            if val > max { max = val; }
        }
        
        // 正と負の値が両方でているか
        assert!(min < -0.1);
        assert!(max > 0.1);
    }
}
