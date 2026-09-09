use api::snapshot::WorldTile;
use nav::{collision::WorldCollision, pack::encode, transport::TransportGraph};
use std::{env, fs, path::PathBuf};

const LEVELS: usize = 4;

fn main() {
    let mut args = env::args().skip(1);
    let mode = args.next().expect("mode");
    let output = PathBuf::from(args.next().expect("output"));
    let (width, height) = match mode.as_str() {
        "uniform" | "pairs" | "nonuniform" | "boundary" | "planes" => (32, 32),
        "all-pairs" => (32, 16),
        "invalid-dimensions" => return fs::write(output, invalid_dimensions()).unwrap(),
        "overflow" => return fs::write(output, overflow_dimensions()).unwrap(),
        _ => panic!("unknown fixture mode"),
    };
    let cells = width * height * LEVELS;
    let mut walk = vec![0u8; cells];
    let mut blocked = vec![0u64; cells.div_ceil(64)];
    for level in 0..LEVELS {
        for z in 0..height {
            for x in 0..width {
                let index = level * width * height + z * width + x;
                let pair = match mode.as_str() {
                    "uniform" => 0,
                    "pairs" => ((x + z * width) % 512) as u16,
                    "all-pairs" => (x + z * width) as u16,
                    "nonuniform" => {
                        if x < 16 {
                            7
                        } else {
                            8
                        }
                    }
                    "boundary" => {
                        if x + 1 == width || z + 1 == height {
                            9
                        } else {
                            0
                        }
                    }
                    "planes" => (level as u16) << 4 | 1,
                    _ => unreachable!(),
                };
                walk[index] = pair as u8;
                if pair & 0x100 != 0 {
                    blocked[index / 64] |= 1u64 << (index % 64);
                }
            }
        }
    }
    let collision = WorldCollision {
        origin: WorldTile {
            x: 100,
            z: -20,
            level: 0,
        },
        width,
        height,
        walk,
        blocked,
        flags: None,
    };
    fs::write(output, encode(&collision, &TransportGraph::default(), &[])).unwrap();
}

fn invalid_dimensions() -> Vec<u8> {
    let mut bytes = b"274V".to_vec();
    bytes.extend([8u8]);
    bytes.extend([0u8; 12]);
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes
}

fn overflow_dimensions() -> Vec<u8> {
    let mut bytes = b"274V".to_vec();
    bytes.extend([8u8]);
    bytes.extend([0u8; 12]);
    bytes.extend(u32::MAX.to_le_bytes());
    bytes.extend(u32::MAX.to_le_bytes());
    bytes
}
