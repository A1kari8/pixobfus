use clap::{Parser, ValueEnum};
use image::{GenericImageView, ImageFormat, RgbaImage};
use pixobfus::{
    BLOCK_SIZE, Curve as LibCurve, derive_seed, generate_random_phrase, process_image,
    validate_dimensions,
};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::exit;

#[derive(Debug, Clone, ValueEnum)]
enum Curve {
    Morton,
    Gilbert,
}

impl From<Curve> for LibCurve {
    fn from(curve: Curve) -> Self {
        match curve {
            Curve::Morton => LibCurve::Morton,
            Curve::Gilbert => LibCurve::Gilbert,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    author, version, about,
    group(
        clap::ArgGroup::new("mode")
            .required(true)
            .args(&["obfuscate", "restore"]),
    )
)]
struct Args {
    /// Obfuscate the image
    #[arg(short = 'O', long, conflicts_with = "restore")]
    obfuscate: bool,
    /// Restore the image
    #[arg(short = 'R', long, conflicts_with = "obfuscate")]
    restore: bool,

    /// Secret key for obfuscation/restoration (can be a string or a number).
    /// - obfuscate: If not provided, a random key will be generated and displayed.
    /// - restore: Required to reverse the obfuscation.
    #[arg(short = 'k', long, value_name = "KEY", verbatim_doc_comment)]
    key: Option<String>,

    /// Curve type for block rearrangement
    #[arg(short = 'c', long, value_enum, default_value = "gilbert")]
    curve: Curve,

    #[arg(short = 'o', long)]
    output: Option<String>,

    /// Input image file path
    input: String,
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

fn main() {
    let args = Args::parse();

    // 读取输入文件
    let file_bytes = fs::read(&args.input).unwrap_or_else(|e| {
        eprintln!("Error reading input: {}", e);
        exit(1);
    });

    // 确定操作模式
    let is_restore_mode = args.restore;

    // 处理密钥
    let seed = handle_key(&args, is_restore_mode);

    // 加载图像
    let img = image::load_from_memory(&file_bytes).unwrap_or_else(|e| {
        eprintln!("Error decoding image: {}", e);
        exit(1);
    });

    // 验证图像尺寸和块大小
    let (width, height) = img.dimensions();
    if validate_dimensions(width, height, BLOCK_SIZE).is_none() {
        eprintln!("Error: Block size {} is too large.", BLOCK_SIZE);
        exit(1);
    }

    // 处理图像
    let out_img = process_image(&img, seed, args.curve.clone().into(), is_restore_mode)
        .unwrap_or_else(|e| {
            eprintln!("Error processing image: {}", e);
            exit(1);
        });

    // 生成输出路径并保存
    let output_path = generate_output_path(&args, is_restore_mode);
    save_output(&out_img, &output_path);
}
