//! Ocean scene viewed through four physically spaced display windows.
//! 23 mm key pitch, 15.2064 mm active LCD width, 128 pixels per panel.
pub const TILE_WIDTH: usize = 16;
pub const BUFFER_BYTES: usize = TILE_WIDTH * 128 * 2;
pub const LAYERS: usize = 3;
pub type Column = [u8; LAYERS];
const PITCH: i64 = 230_000; // 0.1 micrometres
const ACTIVE_WIDTH: i64 = 152_064;
const fn origin(panel: i64) -> i32 {
    (panel * PITCH * 128 * 256 / ACTIVE_WIDTH) as i32
}
const ORIGINS: [i32; 4] = [origin(0), origin(1), origin(2), origin(3)];

pub fn world_x_q8(x: usize) -> i32 {
    ORIGINS[x / 128] + ((x % 128) as i32 * 256) + 128
}
const SINE: [i16; 64] = [
    0, 4, 7, 10, 14, 17, 20, 23, 25, 28, 30, 32, 33, 34, 35, 36, 36, 36, 35, 34, 33, 32, 30, 28,
    25, 23, 20, 17, 14, 10, 7, 4, 0, -4, -7, -10, -14, -17, -20, -23, -25, -28, -30, -32, -33, -34,
    -35, -36, -36, -36, -35, -34, -33, -32, -30, -28, -25, -23, -20, -17, -14, -10, -7, -4,
];

fn sin(phase: i32) -> i32 {
    let p = (phase & 65535) as usize;
    let i = p >> 10;
    let a = SINE[i] as i32;
    let b = SINE[(i + 1) & 63] as i32;
    a * 256 + (b - a) * (p & 1023) as i32 / 4
}

pub fn heights(time: u32, output: &mut [Column; 512]) {
    let t = (time & 65535) as i32;
    for (x, column) in output.iter_mut().enumerate() {
        let world = world_x_q8(x);
        // Different wavelengths, speeds and directions for depth/parallax.
        let swell = sin(world - t * 14);
        let chop = sin(world * 3 + t * 9);
        column[0] = (46 + swell / 922 + chop / 3072) as u8;
        column[1] = (78 + sin(world * 2 + t * 11) / 1536 + sin(world * 5 - t * 7) / 4608) as u8;
        column[2] = (105 + sin(world + t * 17) / 1843) as u8;
    }
}

const fn rgb(r: u16, g: u16, b: u16) -> u16 {
    ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)
}

const fn lerp(a: u16, b: u16, n: u16, d: u16) -> u16 {
    ((a as u32 * (d - n) as u32 + b as u32 * n as u32) / d as u32) as u16
}

const fn palettes() -> [[u16; 128]; 4] {
    let mut colors = [[0; 128]; 4];
    let mut y = 0;
    while y < 128 {
        // Dusk plum -> dusty coral -> apricot, complementary to deep teal sea.
        let h = if y < 26 {
            y
        } else if y < 60 {
            y - 26
        } else {
            34
        } as u16;
        colors[0][y] = if y < 26 {
            rgb(
                lerp(49, 180, h, 26),
                lerp(32, 77, h, 26),
                lerp(78, 104, h, 26),
            )
        } else {
            rgb(
                lerp(180, 255, h, 34),
                lerp(77, 170, h, 34),
                lerp(104, 103, h, 34),
            )
        };
        colors[1][y] = rgb(14, 102 - y as u16 / 3, 109 - y as u16 / 4);
        colors[2][y] = rgb(10, 78 - y as u16 / 3, 88 - y as u16 / 4);
        colors[3][y] = rgb(8, 59 - y as u16 / 4, 72 - y as u16 / 4);
        y += 1;
    }
    colors
}
const COLORS: [[u16; 128]; 4] = palettes();
// Golden-ratio placement over panel 3 within the 709-scene-pixel physical span.
const SUN_X: i32 = 438;

fn backdrop(layer: usize, world: i32, y: u8) -> u16 {
    let dx = (world - SUN_X).abs();
    if layer == 0 {
        let dy = y as i32 - 35;
        if dx < 20 && dx * dx + dy * dy < 400 {
            return rgb(255, 233 - y as u16, 150 - y as u16);
        }
        // Sparse, fine coral cloud streaks, continuous through physical gaps.
        if (y == 12 || y == 14) && (world + 37).rem_euclid(241) < 81 {
            return rgb(143, 69, 104);
        }
    } else {
        // A widening, broken reflection path. Only its boundaries are exposed
        // by moving waves, preserving the dirty-region rendering contract.
        let width = 5 + (y as i32 - 38).max(0) / 3;
        let ripple = ((y as i32 * 13) % 11) - 5;
        if (world - SUN_X + ripple).abs() < width && y % 7 < 3 {
            return match layer {
                1 => rgb(224, 154, 91),
                2 => rgb(163, 112, 77),
                _ => rgb(112, 83, 65),
            };
        }
    }
    COLORS[layer][y as usize]
}

fn foam_color(layer: usize, world: i32, edge: bool) -> u16 {
    if (world - SUN_X).abs() < 43 {
        if edge {
            [rgb(235, 158, 101), rgb(176, 124, 88), rgb(123, 96, 77)][layer]
        } else {
            [rgb(255, 221, 161), rgb(228, 175, 112), rgb(186, 142, 101)][layer]
        }
    } else if edge {
        [rgb(66, 126, 130), rgb(46, 93, 103), rgb(30, 68, 83)][layer]
    } else {
        [rgb(136, 171, 160), rgb(88, 136, 137), rgb(61, 103, 116)][layer]
    }
}

pub fn pixel(x: usize, y: u8, column: &Column) -> u16 {
    let world = world_x_q8(x) >> 8;
    for layer in (0..LAYERS).rev() {
        if y >= column[layer] {
            let depth = y - column[layer];
            if depth == 0 {
                return foam_color(layer, world, false);
            }
            if depth == 1 && (world % 19) < 13 {
                return foam_color(layer, world, true);
            }
            return backdrop(layer + 1, world, y);
        }
    }
    backdrop(0, world, y)
}

pub fn render_tile(
    x: usize,
    layer: Option<usize>,
    old: &[Column],
    new: &[Column],
    buffer: &mut [u8; BUFFER_BYTES],
) -> (u8, u8, usize) {
    assert_eq!(old.len(), TILE_WIDTH);
    assert_eq!(new.len(), TILE_WIDTH);
    let (top, bottom) = if let Some(layer) = layer {
        let top = old.iter().chain(new).map(|c| c[layer]).min().unwrap();
        let bottom = old.iter().chain(new).map(|c| c[layer]).max().unwrap() + 1;
        (top, bottom)
    } else {
        (0, 127)
    };
    let height = bottom - top + 1;
    let mut offset = 0;
    if let Some(layer) = layer {
        // Layer ranges never overlap, so this region contains exactly one
        // water boundary. Precompute horizontal foam masks once per tile.
        let mut foam = [false; TILE_WIDTH];
        let mut worlds = [0; TILE_WIDTH];
        let mut crests = [[0; 2]; TILE_WIDTH];
        let mut edges = [[0; 2]; TILE_WIDTH];
        for i in 0..TILE_WIDTH {
            worlds[i] = world_x_q8(x + i) >> 8;
            foam[i] = worlds[i] % 19 < 13;
            crests[i] = foam_color(layer, worlds[i], false).to_be_bytes();
            edges[i] = foam_color(layer, worlds[i], true).to_be_bytes();
        }
        for y in top..=bottom {
            for (i, column) in new.iter().enumerate() {
                let center = column[layer];
                let bytes = if y < center {
                    backdrop(layer, worlds[i], y).to_be_bytes()
                } else if y == center {
                    crests[i]
                } else if y == center + 1 && foam[i] {
                    edges[i]
                } else {
                    backdrop(layer + 1, worlds[i], y).to_be_bytes()
                };
                buffer[offset] = bytes[0];
                buffer[offset + 1] = bytes[1];
                offset += 2;
            }
        }
    } else {
        for y in top..=bottom {
            for (i, column) in new.iter().enumerate() {
                let color = pixel(x + i, y, column);
                buffer[offset..offset + 2].copy_from_slice(&color.to_be_bytes());
                offset += 2;
            }
        }
    }
    (top, height, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_windows_include_the_unseen_gaps() {
        assert_eq!(world_x_q8(127) - world_x_q8(0), 127 * 256);
        for panel in 1..4 {
            let step = world_x_q8(panel * 128) - world_x_q8((panel - 1) * 128);
            assert!((step as f64 / 256.0 - 23.0 * 128.0 / 15.2064).abs() < 0.004);
            let edge_step = world_x_q8(panel * 128) - world_x_q8(panel * 128 - 1);
            assert!(edge_step > 66 * 256 && edge_step < 67 * 256);
        }
    }

    #[test]
    fn ocean_dirty_updates_equal_full_render() {
        let mut display = [0u16; 512 * 128];
        let mut old = [[0; LAYERS]; 512];
        let mut next = old;
        let mut buffer = [0; BUFFER_BYTES];
        heights(0, &mut old);
        for x in 0..512 {
            for y in 0..128 {
                display[y * 512 + x] = pixel(x, y as u8, &old[x]);
            }
        }
        for time in [1, 2, 17, 64, 127, 255, 900, 65535, 65536, u32::MAX] {
            heights(time, &mut next);
            for x in (0..512).step_by(TILE_WIDTH) {
                for layer in 0..LAYERS {
                    let (y, h, len) = render_tile(
                        x,
                        Some(layer),
                        &old[x..x + 16],
                        &next[x..x + 16],
                        &mut buffer,
                    );
                    assert!(y as usize + h as usize <= 128);
                    assert_eq!(len, h as usize * 16 * 2);
                    for row in 0..h as usize {
                        for col in 0..16 {
                            let i = (row * 16 + col) * 2;
                            display[(y as usize + row) * 512 + x + col] =
                                u16::from_be_bytes([buffer[i], buffer[i + 1]]);
                        }
                    }
                }
            }
            for x in 0..512 {
                for y in 0..128 {
                    assert_eq!(
                        display[y * 512 + x],
                        pixel(x, y as u8, &next[x]),
                        "time={time} x={x} y={y}"
                    );
                }
            }
            old = next;
        }
    }
}
