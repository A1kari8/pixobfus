use clap::Parser;
use image::{GenericImage, GenericImageView, ImageFormat, RgbaImage};
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::exit;

const SIGNATURE: &[u8; 13] = b"PIXOBFUS_V1_0";

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    input: String,

    #[arg(short, long)]
    key: Option<String>,

    #[arg(short, long, default_value_t = 8)]
    block: u32,

    #[arg(short, long)]
    output: Option<String>,

    /// 执行混淆
    #[arg(short, long, conflicts_with = "restore")]
    scramble: bool,

    /// 执行还原
    #[arg(short, long, conflicts_with = "scramble")]
    restore: bool,
}

/// 将字符串转为u64种子
fn derive_seed(key: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(SIGNATURE);
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    // 取前8个字节
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&result[0..8]);
    u64::from_le_bytes(bytes)
}

fn generate_random_phrase() -> String {
    let adjs = [
        "ancient", "broken", "clever", "distant", "emerald", "flying", "giant", "hidden", "iron",
        "jolly", "kind", "lucky", "mystic", "neon", "odd", "pure", "quiet", "rapid", "silver",
        "tiny", "ultra", "vivid", "wild", "young",
    ];
    let colors = [
        "red", "blue", "green", "yellow", "purple", "orange", "pink", "brown", "black", "white",
        "cyan", "magenta", "gold", "silver", "amber", "teal",
    ];
    let nouns = [
        "tiger", "forest", "mountain", "nebula", "river", "phoenix", "shadow", "storm", "rabbit",
        "ocean", "star", "wolf", "eagle", "dragon", "hammer", "cloud", "knight", "wizard",
        "castle", "bridge", "spirit", "comet", "stone", "flame",
    ];

    let mut rng = rand::rng();
    let adj = adjs.choose(&mut rng).unwrap();
    let color = colors.choose(&mut rng).unwrap();
    let noun = nouns.choose(&mut rng).unwrap();
    let num: u16 = rng.random_range(100..999);

    format!("{}-{}-{}-{}", adj, color, noun, num)
}

fn check_signature(data: &[u8]) -> bool {
    if data.len() < SIGNATURE.len() {
        return false;
    }
    let sig_pos = data.len() - SIGNATURE.len();
    &data[sig_pos..sig_pos + SIGNATURE.len()] == SIGNATURE
}

/// 确定模式
fn determine_mode(args: &Args, has_signature: bool) -> bool {
    if args.scramble {
        false // 混淆
    } else if args.restore {
        if !has_signature {
            eprintln!("Error: Input file does not have the expected signature for restoration.");
            exit(1);
        }
        true // 还原
    } else {
        has_signature // 默认自动识别
    }
}

/// 处理密钥逻辑
fn handle_key(args: &Args, is_scrambled: bool) -> u64 {
    match args.key {
        Some(ref k) => k.parse::<u64>().unwrap_or_else(|_| derive_seed(k)),
        None => {
            if is_scrambled {
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

/// 生成打乱的索引序列
fn generate_shuffle_indices(num_blocks: usize, seed: u64) -> Vec<usize> {
    let mut shuffle_rng = ChaCha8Rng::seed_from_u64(seed);
    let mut indices: Vec<usize> = (0..num_blocks).collect();
    indices.shuffle(&mut shuffle_rng);
    indices
}

/// 生成颜色掩码
fn generate_color_masks(num_blocks: usize, block_size: u32, seed: u64) -> Vec<Vec<u8>> {
    let mut color_rng = ChaCha8Rng::seed_from_u64(seed);
    let mut all_masks = Vec::with_capacity(num_blocks);

    for _ in 0..num_blocks {
        let mut block_mask = vec![0u8; (block_size * block_size * 3) as usize];
        for i in 0..(block_size * block_size) as usize {
            let r: u32 = color_rng.random();
            let b = r.to_le_bytes();
            block_mask[i * 3] = b[0];
            block_mask[i * 3 + 1] = b[1];
            block_mask[i * 3 + 2] = b[2];
        }
        all_masks.push(block_mask);
    }

    all_masks
}

/// 处理图像块（混淆或还原）
fn process_blocks(
    img: &image::DynamicImage,
    out_img: &mut RgbaImage,
    indices: &[usize],
    masks: &[Vec<u8>],
    cols: u32,
    block_size: u32,
    is_scrambled: bool,
) {
    let num_blocks = indices.len();

    for i in 0..num_blocks {
        let (src_idx, dest_idx) = if !is_scrambled {
            // 混淆：从原位置i取块，经过XOR后放到打乱后的位置indices[i]
            (i, indices[i])
        } else {
            // 还原：从混淆后的位置indices[i]取块，经过XOR放回原位置 i
            (indices[i], i)
        };

        let src_x = (src_idx as u32 % cols) * block_size;
        let src_y = (src_idx as u32 / cols) * block_size;
        let dest_x = (dest_idx as u32 % cols) * block_size;
        let dest_y = (dest_idx as u32 / cols) * block_size;

        let mut part = img.view(src_x, src_y, block_size, block_size).to_image();

        // 对颜色进行异或
        let mask = &masks[i];
        for (px_idx, pixel) in part.pixels_mut().enumerate() {
            pixel.0[0] ^= mask[px_idx * 3];
            pixel.0[1] ^= mask[px_idx * 3 + 1];
            pixel.0[2] ^= mask[px_idx * 3 + 2];
        }

        out_img.copy_from(&part, dest_x, dest_y).unwrap();
    }
}

/// 处理边缘像素
fn process_edges(
    out_img: &mut RgbaImage,
    width: u32,
    height: u32,
    cols: u32,
    rows: u32,
    block_size: u32,
    seed: u64,
) {
    let mut edge_rng = ChaCha8Rng::seed_from_u64(seed + 1);

    for y in 0..height {
        for x in 0..width {
            if x >= cols * block_size || y >= rows * block_size {
                let pixel = out_img.get_pixel_mut(x, y);
                let mask: u32 = edge_rng.random();
                let m = mask.to_le_bytes();
                pixel.0[0] ^= m[0];
                pixel.0[1] ^= m[1];
                pixel.0[2] ^= m[2];
            }
        }
    }
}

/// 保存输出文件
fn save_output(out_img: &RgbaImage, output_path: &str, is_scrambled: bool, seed: u64) {
    let mut buffer = Cursor::new(Vec::new());
    out_img.write_to(&mut buffer, ImageFormat::Png).unwrap();
    let mut final_data = buffer.into_inner();

    if !is_scrambled {
        final_data.extend_from_slice(SIGNATURE);
    }

    fs::write(output_path, final_data).unwrap_or_else(|e| {
        eprintln!("Error writing file: {}", e);
        exit(1);
    });
}

/// 生成输出文件路径
fn generate_output_path(args: &Args, is_scrambled: bool) -> String {
    args.output.clone().unwrap_or_else(|| {
        let path = Path::new(&args.input);
        let stem = path.file_stem().unwrap().to_str().unwrap();
        if !is_scrambled {
            format!("{}_obfus.png", stem)
        } else {
            format!("{}_res.png", stem)
        }
    })
}

fn main() {
    let args = Args::parse();
    let block_size = args.block;

    // 读取输入文件
    let file_bytes = fs::read(&args.input).unwrap_or_else(|e| {
        eprintln!("Error reading input: {}", e);
        exit(1);
    });

    // 检查签名并确定模式
    let has_signature = check_signature(&file_bytes);
    let is_scrambled = determine_mode(&args, has_signature);

    // 处理密钥
    let seed = handle_key(&args, is_scrambled);

    // 加载图像
    let img = image::load_from_memory(&file_bytes).unwrap_or_else(|e| {
        eprintln!("Error decoding image: {}", e);
        exit(1);
    });

    // 验证图像尺寸和块大小
    let (width, height) = img.dimensions();
    let (cols, rows) = validate_dimensions(width, height, block_size);

    // 初始化输出图像
    let num_blocks = (cols * rows) as usize;
    let mut out_img = RgbaImage::new(width, height);
    out_img.copy_from(&img, 0, 0).unwrap();

    // 生成打乱索引和颜色掩码
    let indices = generate_shuffle_indices(num_blocks, seed);
    let masks = generate_color_masks(num_blocks, block_size, seed);

    // 处理图像块
    process_blocks(
        &img,
        &mut out_img,
        &indices,
        &masks,
        cols,
        block_size,
        is_scrambled,
    );

    // 处理边缘像素
    process_edges(&mut out_img, width, height, cols, rows, block_size, seed);

    // 生成输出路径并保存
    let output_path = generate_output_path(&args, is_scrambled);
    save_output(&out_img, &output_path, is_scrambled, seed);
}
