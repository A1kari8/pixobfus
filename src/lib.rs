use image::{GenericImage, GenericImageView, RgbaImage};
use rand::seq::IndexedRandom;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sha2::{Digest, Sha256};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

pub const BLOCK_SIZE: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Morton,
    Gilbert,
}

/// 将字符串转为u64种子
pub fn derive_seed(key: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    // 取前8个字节
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&result[0..8]);
    u64::from_le_bytes(bytes)
}

/// 生成随机短语
pub fn generate_random_phrase() -> String {
    use bip39::Mnemonic;
    let mut rng = rand::rng();
    let entropy = (0..16).map(|_| rng.random::<u8>()).collect::<Vec<u8>>();
    let mnemonic = Mnemonic::from_entropy(&entropy).expect("Failed to generate mnemonic");
    // 拆分成单词列表
    let words: Vec<&str> = mnemonic.words().collect();
    // 随机选6个不同的词
    let selected: Vec<&str> = words.sample(&mut rng, 6).cloned().collect();
    selected.join("-")
}

/// 验证块大小是否合理
pub fn validate_dimensions(width: u32, height: u32, block_size: u32) -> Option<(u32, u32)> {
    let cols = width / block_size;
    let rows = height / block_size;

    if cols == 0 || rows == 0 {
        None
    } else {
        Some((cols, rows))
    }
}

/// 应用变换
fn apply_symmetry(block: &mut RgbaImage, state: u8, is_restore: bool) {
    match state {
        // 旋转
        1 => {
            if !is_restore {
                *block = image::imageops::rotate90(block);
            } else {
                *block = image::imageops::rotate270(block);
            }
        }
        2 => *block = image::imageops::rotate180(block),
        3 => {
            if !is_restore {
                *block = image::imageops::rotate270(block);
            } else {
                *block = image::imageops::rotate90(block);
            }
        }

        // 翻转
        4 => image::imageops::flip_horizontal_in_place(block),
        5 => image::imageops::flip_vertical_in_place(block),

        // 对角线翻转
        6 => {
            if !is_restore {
                *block = image::imageops::rotate90(block);
                image::imageops::flip_horizontal_in_place(block);
            } else {
                image::imageops::flip_horizontal_in_place(block);
                *block = image::imageops::rotate270(block);
            }
        }
        7 => {
            if !is_restore {
                // 混淆先转后翻
                *block = image::imageops::rotate90(block);
                image::imageops::flip_vertical_in_place(block);
            } else {
                // 还原先翻后转
                image::imageops::flip_vertical_in_place(block);
                *block = image::imageops::rotate270(block);
            }
        }
        _ => {} // 0: 不动
    }
}

/// 重新排列图像块
pub fn rearrange_blocks(
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
pub fn generate_morton_indices(cols: u32, rows: u32, seed: u64, is_restore: bool) -> Vec<usize> {
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
    rng: &mut ChaCha8Rng,
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

    let mut do_split_w = 2 * w > 3 * h;

    // 长宽比接近正方形时随机决定切割方向
    let aspect_ratio = w as f32 / h as f32;
    if aspect_ratio > 0.75 && aspect_ratio < 1.5 {
        do_split_w = rng.random_bool(0.5);
    }

    if do_split_w {
        if (w2 % 2 != 0) && (w > 2) {
            ax2 += dax;
            ay2 += day;
        }
        generate_gilbert_path(x, y, ax2, ay2, bx, by, cols, path, rng);
        generate_gilbert_path(
            x + ax2,
            y + ay2,
            ax - ax2,
            ay - ay2,
            bx,
            by,
            cols,
            path,
            rng,
        );
    } else {
        if (h2 % 2 != 0) && (h > 2) {
            bx2 += dbx;
            by2 += dby;
        }
        generate_gilbert_path(x, y, bx2, by2, ax2, ay2, cols, path, rng);
        generate_gilbert_path(
            x + bx2,
            y + by2,
            ax,
            ay,
            bx - bx2,
            by - by2,
            cols,
            path,
            rng,
        );
        generate_gilbert_path(
            x + (ax - dax) + (bx2 - dbx),
            y + (ay - day) + (by2 - dby),
            -bx2,
            -by2,
            -(ax - ax2),
            -(ay - ay2),
            cols,
            path,
            rng,
        );
    }
}

/// Gilbert索引生成
pub fn generate_gilbert_indices(cols: u32, rows: u32, seed: u64, is_restore: bool) -> Vec<usize> {
    let mut path = Vec::with_capacity((cols * rows) as usize);

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    // 初始化Gilbert递归
    if cols >= rows {
        generate_gilbert_path(
            0,
            0,
            cols as i32,
            0,
            0,
            rows as i32,
            cols,
            &mut path,
            &mut rng,
        );
    } else {
        generate_gilbert_path(
            0,
            0,
            0,
            rows as i32,
            cols as i32,
            0,
            cols,
            &mut path,
            &mut rng,
        );
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

/// 处理图像混淆/还原
pub fn process_image(
    img: &image::DynamicImage,
    seed: u64,
    curve: Curve,
    is_restore: bool,
) -> Result<RgbaImage, String> {
    let (width, height) = img.dimensions();

    let (cols, rows) = validate_dimensions(width, height, BLOCK_SIZE).ok_or_else(|| {
        format!(
            "Block size {} is too large for image dimensions",
            BLOCK_SIZE
        )
    })?;

    let mut out_img = RgbaImage::new(width, height);
    out_img.copy_from(img, 0, 0).unwrap();

    let indices = match curve {
        Curve::Morton => generate_morton_indices(cols, rows, seed, is_restore),
        Curve::Gilbert => generate_gilbert_indices(cols, rows, seed, is_restore),
    };

    rearrange_blocks(
        img,
        &mut out_img,
        &indices,
        cols,
        BLOCK_SIZE,
        seed,
        is_restore,
    );

    Ok(out_img)
}

/// WASM接口
#[cfg(target_arch = "wasm32")]
pub mod wasm {
    use super::*;
    use image::ImageFormat;
    use std::io::Cursor;

    #[wasm_bindgen]
    pub fn obfuscate_image(
        image_data: &[u8],
        key: &str,
        use_gilbert: bool,
    ) -> Result<Vec<u8>, JsValue> {
        let img = image::load_from_memory(image_data)
            .map_err(|e| JsValue::from_str(&format!("Failed to load image: {}", e)))?;

        let seed = derive_seed(key);
        let curve = if use_gilbert {
            Curve::Gilbert
        } else {
            Curve::Morton
        };

        let out_img = process_image(&img, seed, curve, false).map_err(|e| JsValue::from_str(&e))?;

        let mut buffer = Cursor::new(Vec::new());
        out_img
            .write_to(&mut buffer, ImageFormat::Png)
            .map_err(|e| JsValue::from_str(&format!("Failed to encode image: {}", e)))?;

        Ok(buffer.into_inner())
    }

    #[wasm_bindgen]
    pub fn restore_image(
        image_data: &[u8],
        key: &str,
        use_gilbert: bool,
    ) -> Result<Vec<u8>, JsValue> {
        let img = image::load_from_memory(image_data)
            .map_err(|e| JsValue::from_str(&format!("Failed to load image: {}", e)))?;

        let seed = derive_seed(key);
        let curve = if use_gilbert {
            Curve::Gilbert
        } else {
            Curve::Morton
        };

        let out_img = process_image(&img, seed, curve, true).map_err(|e| JsValue::from_str(&e))?;

        let mut buffer = Cursor::new(Vec::new());
        out_img
            .write_to(&mut buffer, ImageFormat::Png)
            .map_err(|e| JsValue::from_str(&format!("Failed to encode image: {}", e)))?;

        Ok(buffer.into_inner())
    }

    #[wasm_bindgen]
    pub fn get_seed_from_key(key: &str) -> String {
        derive_seed(key).to_string()
    }

    #[wasm_bindgen]
    pub fn generate_random_phrase() -> String {
        super::generate_random_phrase()
    }
}
