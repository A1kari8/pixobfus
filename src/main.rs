use clap::Parser;
use image::{GenericImage, GenericImageView, ImageFormat, RgbaImage};
use rand::distr::StandardUniform;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
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
    seed: Option<u64>,

    #[arg(short, long, default_value_t = 8)]
    block: u32,

    #[arg(short, long)]
    output: Option<String>,
}

fn main() {
    let args = Args::parse();
    let block_size = args.block;

    let file_bytes = fs::read(&args.input).unwrap_or_else(|e| {
        eprintln!("Error reading input: {}", e);
        exit(1);
    });

    let is_scrambled = file_bytes.ends_with(SIGNATURE);

    // 仅在生成新种子时打印
    let seed = match args.seed {
        Some(s) => s,
        None => {
            if is_scrambled {
                eprintln!("Error: Seed (-s) is required for de-obfuscation.");
                exit(1);
            } else {
                let s: u64 = rand::rng().sample(StandardUniform);
                println!("{}", s);
                s
            }
        }
    };

    let img = image::load_from_memory(&file_bytes).unwrap_or_else(|e| {
        eprintln!("Error decoding image: {}", e);
        exit(1);
    });

    let (width, height) = img.dimensions();
    let (cols, rows) = (width / block_size, height / block_size);

    if cols == 0 || rows == 0 {
        eprintln!("Error: Block size {} is too large.", block_size);
        exit(1);
    }

    let mut indices: Vec<usize> = (0..(cols * rows) as usize).collect();
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    indices.shuffle(&mut rng);
    let num_blocks = indices.len();

    let target_indices = if is_scrambled {
        let mut inv = vec![0; num_blocks];
        for (i, &val) in indices.iter().enumerate() {
            inv[val] = i;
        }
        inv
    } else {
        indices
    };

    let mut out_img = RgbaImage::new(cols * block_size, rows * block_size);
    for i in 0..num_blocks {
        let (src_idx, dest_idx) = (i as u32, target_indices[i] as u32);
        let src_x = (src_idx % cols) * block_size;
        let src_y = (src_idx / cols) * block_size;
        let dest_x = (dest_idx % cols) * block_size;
        let dest_y = (dest_idx / cols) * block_size;
        let part = img.view(src_x, src_y, block_size, block_size).to_image();
        out_img.copy_from(&part, dest_x, dest_y).unwrap();
    }

    let output_path = args.output.unwrap_or_else(|| {
        let path = Path::new(&args.input);
        let stem = path.file_stem().unwrap().to_str().unwrap();
        if !is_scrambled {
            format!("{}_obfus.png", stem)
        } else {
            format!("{}_res.png", stem)
        }
    });

    let mut buffer = Cursor::new(Vec::new());
    out_img.write_to(&mut buffer, ImageFormat::Png).unwrap();
    let mut final_data = buffer.into_inner();

    if !is_scrambled {
        final_data.extend_from_slice(SIGNATURE);
    }

    fs::write(&output_path, final_data).unwrap_or_else(|e| {
        eprintln!("Error writing file: {}", e);
        exit(1);
    });
}
