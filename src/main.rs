use clap::{Parser, ValueEnum};
use image::{GenericImageView, ImageFormat, RgbaImage};
use pixobfus::{
    BLOCK_SIZE, Curve as LibCurve, derive_seed, format_to_extension, generate_random_phrase,
    process_image, validate_dimensions, validate_format,
};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::exit;

#[derive(Debug, Clone, ValueEnum, Copy)]
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

    #[cfg(feature = "visualize")]
    #[arg(short = 'v', long, default_value_t = false)]
    visualize: bool,

    #[arg(short = 'o', long)]
    output: Option<String>,

    /// Input image file path
    #[arg(required = true, num_args = 1..)]
    input: Vec<String>,
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
                println!("{}", phrase);
                derive_seed(&phrase)
            }
        }
    }
}

/// 保存输出文件
fn save_output(out_img: &RgbaImage, output_path: &str, format: ImageFormat) {
    let mut buffer = Cursor::new(Vec::new());
    out_img.write_to(&mut buffer, format).unwrap();
    let final_data = buffer.into_inner();

    fs::write(output_path, final_data).unwrap_or_else(|e| {
        eprintln!("Error writing file: {}", e);
        exit(1);
    });
}

/// 生成输出文件路径
fn generate_output_path(
    input: &str,
    output: Option<String>,
    is_restore_mode: bool,
    format: ImageFormat,
    output_dir: Option<&str>,
) -> String {
    if let Some(output_path) = output {
        // 如果明确指定了output（单文件模式）
        output_path
    } else {
        let path = Path::new(input);
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let extension = format_to_extension(format);
        let filename = if !is_restore_mode {
            format!("{}_obfus.{}", stem, extension)
        } else {
            format!("{}_res.{}", stem, extension)
        };

        if let Some(dir) = output_dir {
            // 如果有输出目录，将文件名放在该目录下
            Path::new(dir).join(filename).to_str().unwrap().to_string()
        } else {
            // 否则在当前目录
            filename
        }
    }
}

#[cfg(feature = "visualize")]
fn generate_visual_path(width: u32, height: u32, indices: &[usize], block_size: u32) -> RgbaImage {
    use image::Rgba;
    use imageproc::drawing::draw_line_segment_mut;

    let mut canvas = RgbaImage::new(width, height);
    for pixel in canvas.pixels_mut() {
        *pixel = Rgba([30, 30, 30, 255]);
    }

    let cols = width / block_size;
    let line_color = Rgba([0, 255, 255, 255]);

    for i in 0..(indices.len() - 1) {
        let s_idx = indices[i];
        let e_idx = indices[i + 1];

        // 计算两个块的中心点坐标
        let x1 = (s_idx % cols as usize) as u32 * block_size + block_size / 2;
        let y1 = (s_idx / cols as usize) as u32 * block_size + block_size / 2;
        let x2 = (e_idx % cols as usize) as u32 * block_size + block_size / 2;
        let y2 = (e_idx / cols as usize) as u32 * block_size + block_size / 2;

        draw_line_segment_mut(
            &mut canvas,
            (x1 as f32, y1 as f32),
            (x2 as f32, y2 as f32),
            line_color,
        );

        if i == 0 {
            canvas.put_pixel(x1, y1, Rgba([255, 0, 0, 255])); // 起点是红色
        }
    }
    canvas
}

fn process_single_file(
    input: &str,
    output: Option<String>,
    seed: u64,
    curve: LibCurve,
    is_restore_mode: bool,
    output_dir: Option<&str>,
) -> Result<(), String> {
    // 读取输入文件
    let file_bytes =
        fs::read(input).map_err(|e| format!("Error reading input '{}': {}", input, e))?;

    // 检测并加载图像
    let img_format = image::guess_format(&file_bytes)
        .map_err(|e| format!("Error detecting image format for '{}': {}", input, e))?;

    // 验证格式是否支持
    if !validate_format(img_format) {
        return Err(format!(
            "Unsupported image format for '{}'. Only PNG, JPEG, and WebP are supported.",
            input
        ));
    }

    let img = image::load_from_memory(&file_bytes)
        .map_err(|e| format!("Error decoding image '{}': {}", input, e))?;

    // 验证图像尺寸和块大小
    let (width, height) = img.dimensions();
    if validate_dimensions(width, height, BLOCK_SIZE).is_none() {
        return Err(format!(
            "Image '{}' size too small for block size {}.",
            input, BLOCK_SIZE
        ));
    }

    // 处理图像
    let out_img = process_image(&img, seed, curve, is_restore_mode)
        .map_err(|e| format!("Error processing image '{}': {}", input, e))?;

    #[cfg(feature = "visualize")]
    if args.visualize {
        use pixobfus::get_raw_gilbert_path;

        let cols = width / BLOCK_SIZE;
        let rows = height / BLOCK_SIZE;
        let indices = get_raw_gilbert_path(cols, rows, seed);
        let vis_img = generate_visual_path(img.width(), img.height(), &indices.as_slice(), 8);
        vis_img
            .save("debug_path.png")
            .expect("Failed to save debug image");
        println!("Successfully generated path visualization to debug_path.png");
        return Ok(());
    }

    let output_path = generate_output_path(input, output, is_restore_mode, img_format, output_dir);
    save_output(&out_img, &output_path, img_format);

    Ok(())
}

fn main() {
    let args = Args::parse_from(wild::args());

    // 确定操作模式
    let is_restore_mode = args.restore;

    // 处理密钥
    let seed = handle_key(&args, is_restore_mode);

    // 判断是否应该将output作为目录
    let output_dir = if args.input.len() > 1 && args.output.is_some() {
        let dir = args.output.as_ref().unwrap();
        fs::create_dir_all(dir).unwrap_or_else(|e| {
            eprintln!("Error creating output directory: {}", e);
            exit(1);
        });
        Some(dir.as_str())
    } else {
        None
    };

    // 统计处理结果
    let mut success_count = 0;
    let mut fail_count = 0;

    for input in &args.input {
        let output = if output_dir.is_some() {
            None
        } else {
            args.output.clone()
        };

        match process_single_file(
            input,
            output,
            seed,
            args.curve.into(),
            is_restore_mode,
            output_dir,
        ) {
            Ok(_) => {
                success_count += 1;
            }
            Err(e) => {
                fail_count += 1;
                eprintln!("Failed: {}", e);
            }
        }
    }

    if args.input.len() > 1 {
        eprintln!(
            "\nProcessing completed: {} succeeded, {} failed",
            success_count, fail_count
        );
    }

    if fail_count > 0 && success_count == 0 {
        exit(1);
    }
}
