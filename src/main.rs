use clap::{Parser, ValueEnum};
use image::{GenericImageView, ImageFormat, RgbaImage};
use is_terminal::IsTerminal;
use pixobfus::{
    BLOCK_SIZE, Curve as LibCurve, derive_seed, generate_random_phrase, process_image,
    validate_dimensions, validate_format,
};
use std::env;
use std::fs;
use std::io::{self, BufRead, Cursor, Read, Write};
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
    /// - restore: Required (via -k, PIXOBFUS_KEY env var, or interactive input).
    #[arg(short = 'k', long, value_name = "KEY", verbatim_doc_comment)]
    key: Option<String>,

    /// Output generated key to file instead of stderr
    #[arg(short = 'K', long, value_name = "FILE")]
    key_file: Option<String>,

    /// Curve type for block rearrangement
    #[arg(short = 'c', long, value_enum, default_value = "gilbert")]
    curve: Curve,

    #[cfg(feature = "visualize")]
    #[arg(long, default_value_t = false)]
    visualize: bool,

    #[arg(short = 'o', long)]
    output: Option<String>,

    /// Output format (png, jpeg, webp). If not specified, uses input format.
    #[arg(short = 'f', long, value_name = "FORMAT")]
    format: Option<String>,

    /// Input image file path. If omitted and stdin is piped, reads from stdin.
    #[arg(num_args = 0..)]
    input: Vec<String>,
}

/// 处理密钥逻辑
fn handle_key(args: &Args, is_restore_mode: bool, stdin_used_for_input: bool) -> u64 {
    // 优先使用-k参数
    if let Some(ref k) = args.key {
        return k.parse::<u64>().unwrap_or_else(|_| derive_seed(k));
    }

    // 检查环境变量PIXOBFUS_KEY
    if let Ok(key_from_env) = env::var("PIXOBFUS_KEY") {
        if !key_from_env.is_empty() {
            return key_from_env
                .parse::<u64>()
                .unwrap_or_else(|_| derive_seed(&key_from_env));
        }
    }

    if is_restore_mode {
        // 恢复操作需要密钥
        if stdin_used_for_input {
            // stdin被用于图片输入
            eprintln!("Error: Key is required for restore operation.");
            eprintln!("Provide key via -k option or PIXOBFUS_KEY environment variable.");
            exit(1);
        } else if io::stdin().is_terminal() {
            // stdin是终端
            eprint!("Enter key for restore: ");
            io::stderr().flush().unwrap();
            let mut input = String::new();
            io::stdin()
                .lock()
                .read_line(&mut input)
                .unwrap_or_else(|e| {
                    eprintln!("\nError reading key: {}", e);
                    exit(1);
                });
            let key = input.trim();
            if key.is_empty() {
                eprintln!("Error: Key cannot be empty.");
                exit(1);
            }
            key.parse::<u64>().unwrap_or_else(|_| derive_seed(key))
        } else {
            // stdin不是终端，无法交互
            eprintln!("Error: Key is required for restore operation.");
            eprintln!("Provide key via -k option or PIXOBFUS_KEY environment variable.");
            exit(1);
        }
    } else {
        // 混淆操作，生成随机密钥
        let phrase = generate_random_phrase();

        // 根据是否指定 key_file 决定输出方式
        if let Some(ref key_file) = args.key_file {
            // 输出到文件
            fs::write(key_file, format!("{}\n", phrase)).unwrap_or_else(|e| {
                eprintln!("Error writing key file '{}': {}", key_file, e);
                exit(1);
            });
        } else {
            // 输出到 stderr
            eprintln!("Key: {}", phrase);
        }

        derive_seed(&phrase)
    }
}

/// 保存输出文件或输出到stdout
fn save_output(out_img: &RgbaImage, output_path: &str, format: ImageFormat) -> Result<(), String> {
    let mut buffer = Cursor::new(Vec::new());
    out_img
        .write_to(&mut buffer, format)
        .map_err(|e| format!("Error encoding image: {}", e))?;
    let final_data = buffer.into_inner();

    if output_path == "-" {
        // 输出到stdout
        io::stdout()
            .write_all(&final_data)
            .map_err(|e| format!("Error writing to stdout: {}", e))?;
    } else {
        // 输出到文件
        fs::write(output_path, final_data)
            .map_err(|e| format!("Error writing file '{}': {}", output_path, e))?;
    }
    Ok(())
}

/// 生成输出文件路径
fn generate_output_path(input: &str, output: Option<String>, output_dir: Option<&str>) -> String {
    if let Some(output_path) = output {
        // 如果明确指定了output
        if let Some(dir) = output_dir {
            // 多文件模式：output是目录，需要加上输入文件名
            if input == "-" {
                // stdin在多文件模式下输出到stdout
                return "-".to_string();
            }
            let input_filename = Path::new(input).file_name().unwrap().to_str().unwrap();
            Path::new(dir)
                .join(input_filename)
                .to_str()
                .unwrap()
                .to_string()
        } else {
            // 单文件模式：直接使用指定的output
            output_path
        }
    } else {
        // 未指定output，输出到stdout
        "-".to_string()
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
    output_format: Option<ImageFormat>,
) -> Result<(), String> {
    // 读取输入文件或从stdin读取
    let file_bytes = if input == "-" {
        let mut buffer = Vec::new();
        io::stdin()
            .read_to_end(&mut buffer)
            .map_err(|e| format!("Error reading from stdin: {}", e))?;
        buffer
    } else {
        fs::read(input).map_err(|e| format!("Error reading input '{}': {}", input, e))?
    };

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

    // 确定输出格式（优先使用指定的格式，否则使用输入格式）
    let final_format = output_format.unwrap_or(img_format);

    let output_path = generate_output_path(input, output, output_dir);
    save_output(&out_img, &output_path, final_format)?;

    Ok(())
}

fn main() {
    let mut args = Args::parse_from(wild::args());

    // 如果没有提供输入文件，检查是否从stdin读取
    if args.input.is_empty() {
        if !io::stdin().is_terminal() {
            // stdin被管道或重定向，自动从stdin读取
            args.input.push("-".to_string());
        } else {
            // 既没有输入文件，stdin 也是终端
            eprintln!("Error: No input file specified.");
            eprintln!(
                "Usage: {} -O|-R -k KEY [OPTIONS] <INPUT>",
                std::env::args()
                    .next()
                    .unwrap_or_else(|| "pixobfus".to_string())
            );
            eprintln!(
                "   or: cat image.png | {} -O|-R -k KEY [OPTIONS]",
                std::env::args()
                    .next()
                    .unwrap_or_else(|| "pixobfus".to_string())
            );
            exit(1);
        }
    } else {
        // 提供了输入文件，检查是否同时从stdin输入
        if !io::stdin().is_terminal() {
            eprintln!("Error: Cannot use both file input and stdin input simultaneously.");
            eprintln!("Use either file paths or pipe from stdin, not both.");
            exit(1);
        }
    }

    // 确定操作模式
    let is_restore_mode = args.restore;

    // 如果没有指定-o参数
    if args.output.is_none() {
        // 单文件模式且stdout不是TTY
        if args.input.len() == 1 && !io::stdout().is_terminal() {
            // 允许输出到stdout
        } else if args.input.len() == 1 {
            // 单文件模式但stdout是TTY
            eprintln!(
                "Error: Output file required. Use -o FILE to specify output, or redirect stdout."
            );
            eprintln!(
                "Example: {} -O -k KEY input.png -o output.png",
                std::env::args()
                    .next()
                    .unwrap_or_else(|| "pixobfus".to_string())
            );
            eprintln!(
                "     or: {} -O -k KEY input.png > output.png",
                std::env::args()
                    .next()
                    .unwrap_or_else(|| "pixobfus".to_string())
            );
            exit(1);
        }
    }

    // 解析输出格式
    let output_format = if let Some(ref fmt) = args.format {
        Some(match fmt.to_lowercase().as_str() {
            "png" => ImageFormat::Png,
            "jpeg" | "jpg" => ImageFormat::Jpeg,
            "webp" => ImageFormat::WebP,
            _ => {
                eprintln!(
                    "Error: Unsupported format '{}'. Use png, jpeg, or webp.",
                    fmt
                );
                exit(1);
            }
        })
    } else {
        // 未指定格式，使用输入格式
        None
    };

    // 检查stdin是否被用于图片输入
    let stdin_used_for_input = args.input.iter().any(|i| i == "-");

    // 处理密钥
    let seed = handle_key(&args, is_restore_mode, stdin_used_for_input);

    // 多文件模式下必须指定输出位置
    if args.input.len() > 1 && args.output.is_none() {
        eprintln!("Error: Multiple input files require an output directory (-o DIR).");
        exit(1);
    }

    // 判断是否应该将output作为目录（多文件模式）
    let output_dir = if args.input.len() > 1 && args.output.is_some() {
        let dir = args.output.as_ref().unwrap();
        if dir != "-" {
            fs::create_dir_all(dir).unwrap_or_else(|e| {
                eprintln!("Error creating output directory: {}", e);
                exit(1);
            });
            Some(dir.as_str())
        } else {
            None
        }
    } else {
        None
    };

    // 统计处理结果
    let mut success_count = 0;
    let mut fail_count = 0;

    for input in &args.input {
        // 在多文件模式下，需要传递output（目录名）
        // 在单文件模式下，直接传递output（文件名或None）
        let output = args.output.clone();

        match process_single_file(
            input,
            output,
            seed,
            args.curve.into(),
            is_restore_mode,
            output_dir,
            output_format,
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

    if fail_count > 0 || success_count == 0 {
        exit(1);
    }
}
