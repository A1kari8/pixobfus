# Pixobfus

基于空间填充曲线的简易图像混淆工具，支持CLI和WebAssembly

空间填充曲线保留局部性的特点使得图片经过压缩后仍能保持较好的还原效果

[在线试一试](https://a1kari8.github.io/tools/)

## 工作原理

Pixobfus将图像切割为8×8像素的小块，通过以下步骤进行混淆：

1. **空间曲线排序**：使用空间填充曲线遍历所有块，生成一条覆盖整个图像的路径
2. **密钥驱动循环移位**：对路径进行以密钥为种子的循环移位，使每个块的目标位置由密钥决定
3. **块内随机变换**：对每个块独立施加8种变换之一，变换类型同样由密钥决定

还原操作完全对称，使用相同密钥和曲线即可完整复原原图

## CLI使用

### 混淆图像

```bash
# 自动生成随机密钥（密钥会打印到 stderr）
pixobfus -O -o output.png input.png

# 指定密钥
pixobfus -O -k "my-secret-key" -o output.png input.png

# 将密钥保存到文件而非打印到stderr
pixobfus -O -K key.txt -o output.png input.png

# 指定曲线类型（默认gilbert）
pixobfus -O -k "my-key" -c morton -o output.png input.png

# 指定输出格式
pixobfus -O -k "my-key" -f webp -o output.webp input.png
```

### 还原图像

```bash
# 使用相同密钥和曲线类型还原
pixobfus -R -k "my-secret-key" -o restored.png obfuscated.png

# 通过环境变量传递密钥
PIXOBFUS_KEY="my-secret-key" pixobfus -R -o restored.png obfuscated.png
```

### 通过管道处理

```bash
cat input.png | pixobfus -O -k "my-key" > output.png

# 配合其他工具
curl -s https://example.com/image.png | pixobfus -O -k "my-key" -f png > output.png
```

### 批量处理

```bash
# 处理多个文件，结果输出到指定目录（目录不存在时自动创建）
pixobfus -O -k "my-key" -o output_dir/ img1.png img2.jpg img3.webp

# 支持glob展开
pixobfus -O -k "my-key" -o output_dir/ images/*.png
```

### 密钥说明

- 密钥可以是任意字符串，通过SHA-256派生为内部种子
- 混淆时若不提供密钥会自动生成一个由6个BIP39单词组成的随机短语（如 `abandon-ability-able-about-above-absent`）并打印到stderr
- 还原时必须使用与混淆时完全相同的密钥和曲线类型，否则无法正确还原

## WASM使用

在浏览器中导入并调用：

```js
import init, { obfuscate_image, restore_image, generate_random_phrase } from './pkg/pixobfus.js';

await init();

// 混淆
const result = obfuscate_image(imageUint8Array, "my-key", true); // 第三个参数 true=Gilbert, false=Morton

// 还原
const restored = restore_image(obfuscatedUint8Array, "my-key", true);

// 生成随机密钥短语
const phrase = generate_random_phrase();
```
