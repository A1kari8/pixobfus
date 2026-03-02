use bip39::Mnemonic;
use clap::{Parser, ValueEnum};
use image::{GenericImage, GenericImageView, ImageFormat, RgbaImage};
use rand::seq::IndexedRandom;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::exit;

const BLOCK_SIZE: u32 = 8;

#[derive(Debug, Clone, ValueEnum)]
enum Curve {
    Morton,
    Gilbert,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short = 'O', long, conflicts_with = "restore")]
    obfuscate: bool,

    #[arg(short = 'R', long, conflicts_with = "obfuscate")]
    restore: bool,

    #[arg(short = 'k', long)]
    key: Option<String>,

    #[arg(short = 'c', long, value_enum, default_value = "gilbert")]
    curve: Curve,

    #[arg(short = 'o', long)]
    output: Option<String>,

    input: String,
}

/// 将字符串转为u64种子
fn derive_seed(key: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    // 取前8个字节
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&result[0..8]);
    u64::from_le_bytes(bytes)
}

fn generate_random_phrase() -> String {
    let mut rng = rand::rng();
    let entropy = (0..16).map(|_| rng.random::<u8>()).collect::<Vec<u8>>();
    let mnemonic = Mnemonic::from_entropy(&entropy).expect("Failed to generate mnemonic");
    // 拆分成单词列表
    let words: Vec<&str> = mnemonic.words().collect();
    // 随机选6个不同的词
    let selected: Vec<&str> = words.sample(&mut rng, 6).cloned().collect();
    selected.join("-")
}

/// 确定模式
fn determine_mode(args: &Args) -> bool {
    if args.obfuscate {
        false // 混淆
    } else if args.restore {
        true // 还原
    } else {
        eprintln!("Error: Must specify either --obfuscate (-O) or --restore (-R) mode.");
        exit(1);
    }
}

/// 处理密钥逻辑
fn handle_key(args: &Args, is_restore_mode: bool) -> u64 {
    match args.key {
        Some(ref k) => k.parse::<u64>().unwrap_or_else(|_| derive_seed(k)),
        None => {
            if is_restore_mode {
                eprintln!("Error: Key (-k) is required for de-obfuscation.");
                exit(1);
            } else {
                let phrase = generate_random_phrase();
                println!("Generated secret key: {}", phrase);
                derive_seed(&phrase)
            }
        }
    }
}

/// 验证块大小是否合理
fn validate_dimensions(width: u32, height: u32, block_size: u32) -> (u32, u32) {
    let cols = width / block_size;
    let rows = height / block_size;

    if cols == 0 || rows == 0 {
        eprintln!("Error: Block size {} is too large.", block_size);
        exit(1);
    }

    (cols, rows)
}

/// 重新排列图像块
fn rearrange_blocks(
    img: &image::DynamicImage,
    out_img: &mut RgbaImage,
    indices: &[usize],
    cols: u32,
    block_size: u32,
    seed: u64,
    is_restore: bool,
) {
    let num_blocks = indices.len();

    let mut state_rng = ChaCha8Rng::seed_from_u64(seed);
    let block_states: Vec<u8> = (0..num_blocks)
        .map(|_| state_rng.random_range(0..8u8))
        .collect();

    for block_idx in 0..num_blocks {
        let src_idx = block_idx;
        let dest_idx = indices[block_idx];

        let src_x = (src_idx as u32 % cols) * block_size;
        let src_y = (src_idx as u32 / cols) * block_size;
        let dest_x = (dest_idx as u32 % cols) * block_size;
        let dest_y = (dest_idx as u32 / cols) * block_size;

        // 提取块
        let mut block = img.view(src_x, src_y, block_size, block_size).to_image();

        // 获取该块对应的变换状态
        let state = if !is_restore {
            block_states[src_idx]
        } else {
            block_states[dest_idx]
        };

        apply_symmetry(&mut block, state, is_restore);

        out_img.copy_from(&block, dest_x, dest_y).unwrap();
    }
}

/// 应用变换
fn apply_symmetry(block: &mut RgbaImage, state: u8, is_restore: bool) {
    match state {
        1 => image::imageops::flip_horizontal_in_place(block),
        2 => image::imageops::flip_vertical_in_place(block),
        3 => {
            image::imageops::flip_horizontal_in_place(block);
            image::imageops::flip_vertical_in_place(block);
        }
        4 => {
            if !is_restore {
                *block = image::imageops::rotate90(block);
            } else {
                *block = image::imageops::rotate270(block);
            }
        }
        5 => {
            if !is_restore {
                *block = image::imageops::rotate270(block);
            } else {
                *block = image::imageops::rotate90(block);
            }
        }
        6 => {
            *block = image::imageops::rotate180(block);
            image::imageops::flip_horizontal_in_place(block);
        }
        7 => {
            *block = image::imageops::rotate180(block);
        }
        _ => {} // 0: 不动
    }
}

/// 保存输出文件
fn save_output(out_img: &RgbaImage, output_path: &str) {
    let mut buffer = Cursor::new(Vec::new());
    out_img.write_to(&mut buffer, ImageFormat::Png).unwrap();
    let final_data = buffer.into_inner();

    fs::write(output_path, final_data).unwrap_or_else(|e| {
        eprintln!("Error writing file: {}", e);
        exit(1);
    });
}

/// 生成输出文件路径
fn generate_output_path(args: &Args, is_restore_mode: bool) -> String {
    args.output.clone().unwrap_or_else(|| {
        let path = Path::new(&args.input);
        let stem = path.file_stem().unwrap().to_str().unwrap();
        if !is_restore_mode {
            format!("{}_obfus.png", stem)
        } else {
            format!("{}_res.png", stem)
        }
    })
}

/// 莫顿码 (Z-Order)
/// 将二维坐标的位交叉合并
fn morton_encode(x: u32, y: u32) -> u64 {
    let mut z = 0u64;
    for i in 0..32 {
        z |= ((x as u64 & (1u64 << i)) << i) | ((y as u64 & (1u64 << i)) << (i + 1));
    }
    z
}

/// 基于莫顿曲线的循环移位序列
fn generate_morton_indices(cols: u32, rows: u32, seed: u64, is_restore: bool) -> Vec<usize> {
    let num_blocks = (cols * rows) as usize;
    let mut morton_list: Vec<(u64, usize)> = Vec::with_capacity(num_blocks);

    for r in 0..rows {
        for c in 0..cols {
            let idx = (r * cols + c) as usize;
            let z_code = morton_encode(c, r);
            morton_list.push((z_code, idx));
        }
    }

    morton_list.sort_by_key(|&(z, _)| z);

    let shift = (seed % num_blocks as u64) as usize;
    let mut sorted_indices: Vec<usize> = morton_list.iter().map(|&(_, idx)| idx).collect();

    if !is_restore {
        // 混淆：向左转
        sorted_indices.rotate_left(shift);
    } else {
        // 还原：向右转
        sorted_indices.rotate_right(shift);
    }

    let mut final_indices = vec![0; num_blocks];
    let original_morton_order: Vec<usize> = morton_list.iter().map(|&(_, idx)| idx).collect();

    for i in 0..num_blocks {
        final_indices[original_morton_order[i]] = sorted_indices[i];
    }

    final_indices
}

/// Gilbert曲线
fn generate_gilbert_path(
    x: i32,
    y: i32,
    ax: i32,
    ay: i32,
    bx: i32,
    by: i32,
    cols: u32,
    path: &mut Vec<usize>,
) {
    let w = (ax + ay).abs();
    let h = (bx + by).abs();
    let dax = ax.signum();
    let day = ay.signum();
    let dbx = bx.signum();
    let dby = by.signum();

    if h == 1 {
        for i in 0..w {
            path.push(((y + i * day) as u32 * cols + (x + i * dax) as u32) as usize);
        }
        return;
    }

    if w == 1 {
        for i in 0..h {
            path.push(((y + i * dby) as u32 * cols + (x + i * dbx) as u32) as usize);
        }
        return;
    }

    let mut ax2 = ax / 2;
    let mut ay2 = ay / 2;
    let mut bx2 = bx / 2;
    let mut by2 = by / 2;

    let w2 = (ax2 + ay2).abs();
    let h2 = (bx2 + by2).abs();

    if 2 * w > 3 * h {
        if (w2 % 2 != 0) && (w > 2) {
            ax2 += dax;
            ay2 += day;
        }
        generate_gilbert_path(x, y, ax2, ay2, bx, by, cols, path);
        generate_gilbert_path(x + ax2, y + ay2, ax - ax2, ay - ay2, bx, by, cols, path);
    } else {
        if (h2 % 2 != 0) && (h > 2) {
            bx2 += dbx;
            by2 += dby;
        }
        generate_gilbert_path(x, y, bx2, by2, ax2, ay2, cols, path);
        generate_gilbert_path(x + bx2, y + by2, ax, ay, bx - bx2, by - by2, cols, path);
        generate_gilbert_path(
            x + (ax - dax) + (bx2 - dbx),
            y + (ay - day) + (by2 - dby),
            -bx2,
            -by2,
            -(ax - ax2),
            -(ay - ay2),
            cols,
            path,
        );
    }
}

// Gilbert索引生成
fn generate_gilbert_indices(cols: u32, rows: u32, seed: u64, is_restore: bool) -> Vec<usize> {
    let mut path = Vec::with_capacity((cols * rows) as usize);

    // 初始化Gilbert递归
    if cols >= rows {
        generate_gilbert_path(0, 0, cols as i32, 0, 0, rows as i32, cols, &mut path);
    } else {
        generate_gilbert_path(0, 0, 0, rows as i32, cols as i32, 0, cols, &mut path);
    }

    let num_blocks = path.len();
    let shift = (seed % num_blocks as u64) as usize;

    let mut sorted_indices = path.clone();
    if !is_restore {
        sorted_indices.rotate_left(shift);
    } else {
        sorted_indices.rotate_right(shift);
    }

    let mut final_indices = vec![0; num_blocks];
    for i in 0..num_blocks {
        final_indices[path[i]] = sorted_indices[i];
    }
    final_indices
}

fn main() {
    let args = Args::parse();

    // 读取输入文件
    let file_bytes = fs::read(&args.input).unwrap_or_else(|e| {
        eprintln!("Error reading input: {}", e);
        exit(1);
    });

    // 确定操作模式
    let is_restore_mode = determine_mode(&args);

    // 处理密钥
    let seed = handle_key(&args, is_restore_mode);

    // 加载图像
    let img = image::load_from_memory(&file_bytes).unwrap_or_else(|e| {
        eprintln!("Error decoding image: {}", e);
        exit(1);
    });

    // 验证图像尺寸和块大小
    let (width, height) = img.dimensions();
    let (cols, rows) = validate_dimensions(width, height, BLOCK_SIZE);

    // 初始化输出图像
    let mut out_img = RgbaImage::new(width, height);
    out_img.copy_from(&img, 0, 0).unwrap();

    // 生成图像块重排索引
    let indices = match args.curve {
        Curve::Morton => generate_morton_indices(cols, rows, seed, is_restore_mode),
        Curve::Gilbert => generate_gilbert_indices(cols, rows, seed, is_restore_mode),
    };

    // 重新排列图像块
    rearrange_blocks(
        &img,
        &mut out_img,
        &indices,
        cols,
        BLOCK_SIZE,
        seed,
        is_restore_mode,
    );

    // 生成输出路径并保存
    let output_path = generate_output_path(&args, is_restore_mode);
    save_output(&out_img, &output_path);
}
