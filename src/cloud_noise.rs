//! The clouds' tiling noise textures, made once at startup on a thread of
//! their own (a second or two): a low-frequency Perlin-Worley base and
//! Worley octaves (the cloud shapes), high-frequency Worley detail (eats
//! into their edges), and 2D curl noise (swirls the detail).

/// One texture's texels, RGBA8.
pub struct Texels {
    pub size: u32,
    pub depth: u32,
    pub data: Vec<u8>,
}

pub struct CloudNoise {
    pub base: Texels,
    pub detail: Texels,
    pub curl: Texels,
}

pub const BASE_SIZE: u32 = 128;
pub const DETAIL_SIZE: u32 = 32;
pub const CURL_SIZE: u32 = 128;

fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f) ^ seed.wrapping_mul(0x9e37_79b9);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    h
}

fn unit(h: u32) -> f32 {
    (h & 0xff_ffff) as f32 / 16_777_215.0
}

/// Worley (cellular) noise tiling every `cells` cells, 1 at a feature
/// point, 0 a cell away; `p` in 0..1.
fn worley(p: [f32; 3], cells: i32, seed: u32) -> f32 {
    let q = [p[0] * cells as f32, p[1] * cells as f32, p[2] * cells as f32];
    let c = [q[0].floor() as i32, q[1].floor() as i32, q[2].floor() as i32];
    let mut best = f32::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let n = [c[0] + dx, c[1] + dy, c[2] + dz];
                let w = [n[0].rem_euclid(cells), n[1].rem_euclid(cells), n[2].rem_euclid(cells)];
                let h = hash(w[0], w[1], w[2], seed);
                let f = [unit(h), unit(h.rotate_left(11) ^ 0x51ed), unit(h.rotate_left(22) ^ 0x2f3a)];
                let d = [n[0] as f32 + f[0] - q[0], n[1] as f32 + f[1] - q[1], n[2] as f32 + f[2] - q[2]];
                best = best.min(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
            }
        }
    }
    (1.0 - best.sqrt()).clamp(0.0, 1.0)
}

fn worley_fbm(p: [f32; 3], cells: i32, seed: u32) -> f32 {
    worley(p, cells, seed) * 0.625 + worley(p, cells * 2, seed + 1) * 0.25 + worley(p, cells * 4, seed + 2) * 0.125
}

fn grad(x: i32, y: i32, z: i32, period: i32, seed: u32) -> [f32; 3] {
    let h = hash(x.rem_euclid(period), y.rem_euclid(period), z.rem_euclid(period), seed);
    let a = unit(h) * std::f32::consts::TAU;
    let zc = unit(h.rotate_left(16)) * 2.0 - 1.0;
    let r = (1.0 - zc * zc).max(0.0).sqrt();
    [r * a.cos(), r * a.sin(), zc]
}

/// Perlin (gradient) noise tiling every `period` cells, about -1..1.
fn perlin(p: [f32; 3], period: i32, seed: u32) -> f32 {
    let q = [p[0] * period as f32, p[1] * period as f32, p[2] * period as f32];
    let c = [q[0].floor() as i32, q[1].floor() as i32, q[2].floor() as i32];
    let f = [q[0] - c[0] as f32, q[1] - c[1] as f32, q[2] - c[2] as f32];
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let u = [fade(f[0]), fade(f[1]), fade(f[2])];
    let corner = |dx: i32, dy: i32, dz: i32| {
        let g = grad(c[0] + dx, c[1] + dy, c[2] + dz, period, seed);
        g[0] * (f[0] - dx as f32) + g[1] * (f[1] - dy as f32) + g[2] * (f[2] - dz as f32)
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u[0]);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u[0]);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u[0]);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u[0]);
    lerp(lerp(x00, x10, u[1]), lerp(x01, x11, u[1]), u[2]) * 1.15
}

fn remap(v: f32, lo: f32, hi: f32, nlo: f32, nhi: f32) -> f32 {
    nlo + (v - lo) / (hi - lo).max(1e-5) * (nhi - nlo)
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub fn make() -> CloudNoise {
    // The base: R Perlin-Worley (billowy, connected), GBA Worley fbm at
    // rising frequencies (the shapes' low-frequency breakup).
    let n = BASE_SIZE;
    let mut base = Vec::with_capacity((n * n * n * 4) as usize);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let p = [(x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32, (z as f32 + 0.5) / n as f32];
                let perlin_fbm = perlin(p, 4, 7) * 0.5 + perlin(p, 8, 8) * 0.25 + perlin(p, 16, 9) * 0.125;
                let perlin_fbm = (perlin_fbm * 0.5 + 0.5).clamp(0.0, 1.0);
                let w = worley_fbm(p, 4, 11);
                let pw = remap(perlin_fbm, w - 1.0, 1.0, 0.0, 1.0);
                base.extend([byte(pw), byte(worley_fbm(p, 4, 21)), byte(worley_fbm(p, 8, 31)), byte(worley_fbm(p, 16, 41))]);
            }
        }
    }
    // The detail: Worley fbm at three frequencies.
    let n = DETAIL_SIZE;
    let mut detail = Vec::with_capacity((n * n * n * 4) as usize);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let p = [(x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32, (z as f32 + 0.5) / n as f32];
                detail.extend([byte(worley_fbm(p, 2, 51)), byte(worley_fbm(p, 4, 61)), byte(worley_fbm(p, 8, 71)), 255]);
            }
        }
    }
    // Curl: the 2D curl of a tiling Perlin fbm, 0.5 for none.
    let n = CURL_SIZE;
    let field = |x: f32, y: f32| {
        let p = [x, y, 0.37];
        perlin(p, 4, 81) * 0.6 + perlin(p, 8, 82) * 0.3 + perlin(p, 16, 83) * 0.1
    };
    let e = 0.5 / n as f32;
    let mut curl = Vec::with_capacity((n * n * 4) as usize);
    let mut raw = Vec::with_capacity((n * n) as usize);
    let mut peak = 1e-6f32;
    for y in 0..n {
        for x in 0..n {
            let (u, v) = ((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32);
            let dx = (field(u + e, v) - field(u - e, v)) / (2.0 * e);
            let dy = (field(u, v + e) - field(u, v - e)) / (2.0 * e);
            raw.push((dy, -dx));
            peak = peak.max(dx.abs()).max(dy.abs());
        }
    }
    for (a, b) in raw {
        curl.extend([byte(a / peak * 0.5 + 0.5), byte(b / peak * 0.5 + 0.5), 128, 255]);
    }
    CloudNoise {
        base: Texels { size: BASE_SIZE, depth: BASE_SIZE, data: base },
        detail: Texels { size: DETAIL_SIZE, depth: DETAIL_SIZE, data: detail },
        curl: Texels { size: CURL_SIZE, depth: 1, data: curl },
    }
}
