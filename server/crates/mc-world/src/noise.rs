//! 噪声引擎:种子化 Perlin 噪声(2D/3D)+ fBm,自实现、确定性。

pub struct Noise {
    perm: [u8; 512],
}

fn hash_seed(seed: u64) -> u64 {
    let mut x = seed.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

impl Noise {
    pub fn new(seed: u64) -> Self {
        let mut table = [0u8; 256];
        for i in 0..256 {
            table[i] = i as u8;
        }
        // Fisher-Yates,以哈希流为随机源
        let mut s = seed;
        for i in (1..256).rev() {
            s = hash_seed(s);
            let j = ((s >> 16) % (i as u64 + 1)) as usize;
            table.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = table[i & 255];
        }
        Noise { perm }
    }

    /// 2D Perlin 噪声,输出范围约 [-1, 1]。
    pub fn noise2(&self, x: f64, y: f64) -> f64 {
        let xf = x.floor();
        let yf = y.floor();
        let xi = ((xf as i64) & 255) as usize;
        let yi = ((yf as i64) & 255) as usize;
        let xf = x - xf;
        let yf = y - yf;

        let u = fade(xf);
        let v = fade(yf);

        let p = &self.perm;
        let aa = p[p[xi] as usize + yi];
        let ab = p[p[xi] as usize + yi + 1];
        let ba = p[p[xi + 1] as usize + yi];
        let bb = p[p[xi + 1] as usize + yi + 1];

        let x1 = lerp(grad2(aa, xf, yf), grad2(ba, xf - 1.0, yf), u);
        let x2 = lerp(grad2(ab, xf, yf - 1.0), grad2(bb, xf - 1.0, yf - 1.0), u);
        lerp(x1, x2, v)
    }

    /// 3D Perlin 噪声,输出范围约 [-1, 1]。
    pub fn noise3(&self, x: f64, y: f64, z: f64) -> f64 {
        let xf = x.floor();
        let yf = y.floor();
        let zf = z.floor();
        let xi = ((xf as i64) & 255) as usize;
        let yi = ((yf as i64) & 255) as usize;
        let zi = ((zf as i64) & 255) as usize;
        let xf = x - xf;
        let yf = y - yf;
        let zf = z - zf;

        let u = fade(xf);
        let v = fade(yf);
        let w = fade(zf);

        let p = &self.perm;
        let a = p[xi] as usize + yi;
        let b = p[xi + 1] as usize + yi;
        let aa = p[a] as usize + zi;
        let ab = p[a + 1] as usize + zi;
        let ba = p[b] as usize + zi;
        let bb = p[b + 1] as usize + zi;

        let x1 = lerp(grad3(p[aa], xf, yf, zf), grad3(p[ba], xf - 1.0, yf, zf), u);
        let x2 = lerp(grad3(p[ab], xf, yf - 1.0, zf), grad3(p[bb], xf - 1.0, yf - 1.0, zf), u);
        let y1 = lerp(x1, x2, v);
        let x1 = lerp(
            grad3(p[aa + 1], xf, yf, zf - 1.0),
            grad3(p[ba + 1], xf - 1.0, yf, zf - 1.0),
            u,
        );
        let x2 = lerp(
            grad3(p[ab + 1], xf, yf - 1.0, zf - 1.0),
            grad3(p[bb + 1], xf - 1.0, yf - 1.0, zf - 1.0),
            u,
        );
        lerp(y1, lerp(x1, x2, v), w)
    }

    /// fBm 分形叠加,输出约 [-1, 1]。
    pub fn fbm2(&self, x: f64, y: f64, octaves: usize, lacunarity: f64, gain: f64) -> f64 {
        let mut amp = 1.0;
        let mut freq = 1.0;
        let mut sum = 0.0;
        let mut norm = 0.0;
        for _ in 0..octaves {
            sum += self.noise2(x * freq, y * freq) * amp;
            norm += amp;
            amp *= gain;
            freq *= lacunarity;
        }
        sum / norm
    }
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

fn grad2(h: u8, x: f64, y: f64) -> f64 {
    match h & 7 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x,
        5 => -x,
        6 => y,
        _ => -y,
    }
}

fn grad3(h: u8, x: f64, y: f64, z: f64) -> f64 {
    match h & 15 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        11 => -y - z,
        12 => x,
        13 => -x,
        14 => y,
        _ => -y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_same_seed() {
        let a = Noise::new(12345);
        let b = Noise::new(12345);
        for i in 0..20 {
            let (x, y, z) = (i as f64 * 0.73, i as f64 * 1.31, i as f64 * 0.19);
            assert_eq!(a.noise2(x, y), b.noise2(x, y));
            assert_eq!(a.noise3(x, y, z), b.noise3(x, y, z));
        }
    }

    #[test]
    fn different_seed_differs() {
        let a = Noise::new(1);
        let b = Noise::new(2);
        let mut diff = 0;
        for i in 0..50 {
            let (x, z) = (i as f64 * 0.37, i as f64 * 0.29);
            if a.noise2(x, z) != b.noise2(x, z) {
                diff += 1;
            }
        }
        assert!(diff > 40);
    }

    #[test]
    fn range_bounded() {
        let n = Noise::new(42);
        for i in 0..200 {
            let x = (i as f64) * 0.5 - 50.0;
            let v = n.noise2(x, x * 1.7);
            assert!(v >= -1.0 && v <= 1.0, "out of range: {v}");
        }
    }

    #[test]
    fn fbm_in_range() {
        let n = Noise::new(7);
        for i in 0..50 {
            let v = n.fbm2(i as f64 * 0.13, i as f64 * 0.07, 4, 2.0, 0.5);
            assert!(v >= -1.0 && v <= 1.0);
        }
    }

    #[test]
    fn no_zeros_on_bounded_region() {
        let n = Noise::new(2026);
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for x in 0..64 {
            for z in 0..64 {
                let v = n.noise2(x as f64 * 0.31, z as f64 * 0.47);
                min = min.min(v);
                max = max.max(v);
            }
        }
        assert!(max - min > 0.5, "noise too flat: {min}..{max}");
    }
}