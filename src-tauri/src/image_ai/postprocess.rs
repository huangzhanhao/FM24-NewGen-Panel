//! 生成图后处理：落盘前把四角裁成圆角（恒输出 RGBA PNG）。
//!
//! 【为什么不再只认白角】文生图拿不到真 alpha，圆角只能由后处理给。旧版靠提示词让模型
//! 把角外画成白底再裁白；提示词去掉那句之后，模型输出的是满幅方图，四角没有任何白弧，
//! 只认白就等于不裁角。现在半径取「固定比例」与「实测白弧」的较大值：
//! - 固定值 = round(短边 × `RADIUS_RATIO`)，满幅方图也能得到圆角；
//! - 白弧实测沿用 flood fill（近白 min 通道 ≥ 228、限本角象限），半径超过短边 1/4 视为
//!   场景级白色、不采信，避免顺着白区吃进大片内容。
//!
//! 每个角只在自己的 r×r 方框内把圆弧外置透明，宽高不变（头像包不能变形）。
//! `Err` = 解码/编码失败，调用方按原图原格式落盘，不拖垮已成功的生成。

/// 落盘前统一把四角裁成圆角，恒输出 RGBA PNG。
pub(super) fn round_corners(bytes: &[u8]) -> Result<Vec<u8>, String> {
    /// 近白判定：用 245 会漏抗锯齿过渡带，裁完沿弧留亮线。
    const EDGE_T: u8 = 228;
    /// 实测白弧的裁切余量：1~2px 仍会露离散亮残点。
    const MARGIN: u32 = 3;
    /// 固定圆角半径占短边的比例；观感不对只改这一个数。
    const RADIUS_RATIO: f32 = 0.10;

    let rgba = image::load_from_memory(bytes)
        .map_err(|e| format!("图片解码失败: {e}"))?
        .into_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let (wz, hz) = (w as usize, h as usize);
    let mut raw = rgba.into_raw();
    let white_at = |raw: &[u8], x: u32, y: u32| {
        let i = (y as usize * wz + x as usize) * 4;
        raw[i].min(raw[i + 1]).min(raw[i + 2]) >= EDGE_T
    };

    let fixed = (((w.min(h) as f32) * RADIUS_RATIO).round() as u32).max(1);
    let cap = w.min(h) / 4;

    // 象限 = (起点, x 下界, x 上界, y 下界, y 上界)。
    let quadrants: [(u32, u32, u32, u32, u32, u32); 4] = [
        (0, 0, 0, w / 2, 0, h / 2),
        (w - 1, 0, w / 2, w, 0, h / 2),
        (0, h - 1, 0, w / 2, h / 2, h),
        (w - 1, h - 1, w / 2, w, h / 2, h),
    ];
    let mut visited = vec![false; wz * hz];
    for (sx, sy, x0, x1, y0, y1) in quadrants {
        // 1) 实测白弧：只有起点够白才 flood fill；量出的半径超出护栏就当没有。
        let mut detected = 0u32;
        if white_at(&raw, sx, sy) {
            let mut need = 0f64;
            let mut stack = vec![(sx, sy)];
            visited[sy as usize * wz + sx as usize] = true;
            while let Some((x, y)) = stack.pop() {
                let lx = (if sx == 0 { x } else { w - 1 - x }) as f64;
                let ly = (if sy == 0 { y } else { h - 1 - y }) as f64;
                need = need.max(lx + ly + (2.0 * lx * ly).sqrt());
                for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let nx = x as i64 + dx;
                    let ny = y as i64 + dy;
                    if nx < x0 as i64 || nx >= x1 as i64 || ny < y0 as i64 || ny >= y1 as i64 {
                        continue;
                    }
                    let vi = ny as usize * wz + nx as usize;
                    if visited[vi] || !white_at(&raw, nx as u32, ny as u32) {
                        continue;
                    }
                    visited[vi] = true;
                    stack.push((nx as u32, ny as u32));
                }
            }
            let r = need.ceil() as u32 + MARGIN;
            if r <= cap {
                detected = r;
            }
        }
        // 2) 固定半径兜底，与实测取大。
        let r = fixed.max(detected);
        if r > cap {
            continue;
        }
        // 只在本角 r×r 方框内工作：圆心 (r,r) 半径 r 的圆弧外（角区）置透明。
        for ly in 0..r {
            let gy = if sy == 0 { ly } else { h - 1 - ly };
            for lx in 0..r {
                let gx = if sx == 0 { lx } else { w - 1 - lx };
                let ddx = (r - lx) as i64;
                let ddy = (r - ly) as i64;
                if ddx * ddx + ddy * ddy > (r as i64) * (r as i64) {
                    raw[(gy as usize * wz + gx as usize) * 4 + 3] = 0;
                }
            }
        }
    }

    let out = image::RgbaImage::from_raw(w, h, raw).ok_or("图片像素缓冲与尺寸不一致")?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(out)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 重编码失败: {e}"))?;
    Ok(png)
}

// ===================== 单元测试：四角裁圆角 ====================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成「内容铺满、四角圆弧外为白」：旧模板下模型的真实输出形态（白弧只在四角）。
    fn rounded_white_corner_sample(size: u32, radius: u32) -> image::RgbaImage {
        let mut img = image::RgbaImage::from_pixel(size, size, image::Rgba([20, 30, 40, 255]));
        let rr = (radius as i64) * (radius as i64);
        for ly in 0..radius {
            for lx in 0..radius {
                let dx = (radius - lx) as i64;
                let dy = (radius - ly) as i64;
                if dx * dx + dy * dy <= rr {
                    continue; // 弧内是内容
                }
                let white = image::Rgba([255, 255, 255, 255]);
                img.put_pixel(lx, ly, white);
                img.put_pixel(size - 1 - lx, ly, white);
                img.put_pixel(lx, size - 1 - ly, white);
                img.put_pixel(size - 1 - lx, size - 1 - ly, white);
            }
        }
        img
    }

    fn encode_as(img: &image::RgbaImage, fmt: image::ImageFormat) -> Vec<u8> {
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut std::io::Cursor::new(&mut buf), fmt)
            .unwrap();
        buf
    }

    #[test]
    fn round_corners_cuts_white_arcs_and_full_bleed_squares() {
        // ① 模型画了白角（旧模板）：实测半径 22 > 固定 10，按 22 裁，宽高不变。
        let sample = rounded_white_corner_sample(100, 20);
        let out = image::load_from_memory(
            &round_corners(&encode_as(&sample, image::ImageFormat::Png)).unwrap(),
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(out.dimensions(), (100, 100), "宽高必须保持不变");
        for (x, y) in [(0, 0), (99, 0), (0, 99), (99, 99)] {
            assert_eq!(out.get_pixel(x, y)[3], 0, "角 ({x},{y}) 弧外应透明");
        }
        // 余量锁在 2–4px 带内：余量小了沿弧留亮残点，大了多吃内容。
        assert_eq!(out.get_pixel(19, 0)[3], 0);
        assert_eq!(out.get_pixel(20, 0)[3], 0);
        assert_eq!(out.get_pixel(23, 0)[3], 255);
        assert_eq!(*out.get_pixel(50, 50), image::Rgba([20, 30, 40, 255]));

        // ② 满幅方图（提示词不再要求圆角）：没有白弧也要按固定比例裁出圆角。
        let plain = image::RgbaImage::from_pixel(100, 100, image::Rgba([10, 20, 30, 255]));
        let out = image::load_from_memory(
            &round_corners(&encode_as(&plain, image::ImageFormat::Png)).unwrap(),
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(out.dimensions(), (100, 100));
        assert_eq!(out.get_pixel(0, 0)[3], 0, "无白弧的图也必须裁角");
        assert_eq!(out.get_pixel(9, 0)[3], 0);
        assert_eq!(out.get_pixel(10, 0)[3], 255, "固定半径之外的顶边不受影响");
        assert_eq!(*out.get_pixel(50, 50), image::Rgba([10, 20, 30, 255]));

        // ③ 场景级白（整行白贴顶角）不当作角弧：半径回落到固定值，不得顺着白区吃内容。
        let mut scene = image::RgbaImage::from_pixel(64, 64, image::Rgba([15, 25, 35, 255]));
        for x in 0..64 {
            scene.put_pixel(x, 0, image::Rgba([255, 255, 255, 255]));
        }
        let out = image::load_from_memory(
            &round_corners(&encode_as(&scene, image::ImageFormat::Png)).unwrap(),
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(out.get_pixel(0, 0)[3], 0);
        assert_eq!(out.get_pixel(20, 0)[3], 255, "整行白不得被当成角弧");
        assert_eq!(out.get_pixel(20, 0)[0], 255, "顶边白色内容原样保留");

        // ④ 中央白块不受影响：只动四角。
        let mut sealed = image::RgbaImage::from_pixel(64, 64, image::Rgba([15, 25, 35, 255]));
        for y in 26..38 {
            for x in 26..38 {
                sealed.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let out = image::load_from_memory(
            &round_corners(&encode_as(&sealed, image::ImageFormat::Png)).unwrap(),
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(*out.get_pixel(32, 32), image::Rgba([255, 255, 255, 255]));

        // ⑤ 解码失败 → Err：generate 据此按原图原格式落盘，不拖垮成功生成。
        assert!(round_corners(b"not an image").is_err());
    }
}
