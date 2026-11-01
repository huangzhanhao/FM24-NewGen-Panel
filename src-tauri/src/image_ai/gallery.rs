//! 画廊记录（`data_dir/gallery.json`）+ 两套独立淘汰闸门 + 查询。
//!
//! 记录持久、图片仅存活于会话临时目录（.imgtmp，启动即清），生命周期独立，淘汰拆成两条互不
//! 越界的闸门：记录闸门只删 gallery.json 的行、图片闸门只删 .imgtmp 的字节。禁止引入
//! 「删记录行 + 连带删图」的一体实现——那会让一条闸门越界影响另一条。
//!
//! 【并发】gallery.json 是「read → push → save」的非原子读改写：批量并发下各任务读到同一份
//! 旧表再整体覆盖写，后写者会丢先写者刚追加的行（json_store::save 的 tmp+rename 只保单文件
//! 不损坏，不防丢更新）。故用一把模块级互斥把 gallery.json 的 I/O 串行化；互斥只覆盖同步
//! fs 段、绝不跨 await；不碰 gallery.json 的 enforce_image_limit 不参与加锁。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::json_store::{self, LoadError};

use super::config::tmp_dir;
use super::types::{GalleryItem, GalleryRecord, GenImage, GenRequest};

/// 画廊记录文件：落在 data/ 根、**不在** `.imgtmp` 内——`.imgtmp` 启动即清，
/// 记录必须留在这里才能跨会话存活。
pub fn gallery_path(d: &Path) -> PathBuf {
    d.join("gallery.json")
}

/// 画廊记录时间戳：`%Y-%m-%d %H:%M:%S`（字典序即时间序）。
pub fn now_ts() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

static GALLERY_IO: Mutex<()> = Mutex::new(());

/// 读画廊清单的**不加锁内核**：调用方必须已持有 GALLERY_IO（分开是为了让取锁方在自己
/// 临界区内直接调用，避免加锁函数互相调用造成重入死锁——std Mutex 不可重入）。
/// 缺失（首次写入前）按空表；损坏也按空表并告警——记录损坏不值得阻断生成，下次写入即重建。
fn read_gallery_inner(path: &Path) -> Vec<GalleryRecord> {
    match json_store::load::<Vec<GalleryRecord>>(path) {
        Ok(v) => v,
        Err(LoadError::Missing) => Vec::new(),
        Err(LoadError::Corrupt(e)) => {
            log::warn!(target: "image_ai", "画廊记录损坏，按空记录重建: {}（{e}）", path.display());
            Vec::new()
        }
    }
}

/// 把本次生成的每张图记入画廊（每图一条），再按记录上限淘汰旧行。仅 generate() 成功
/// 路径调用（失败路径图片未落盘）。记录字段取自请求。
pub fn record_gallery(
    data_dir: &Path,
    images: &[GenImage],
    req: &GenRequest,
    limit: usize,
) -> Result<(), String> {
    let path = gallery_path(data_dir);
    let ts = now_ts();
    {
        // 「读—改—写」在同一临界区内完成：并发调用被串行化，杜绝后写者覆盖前写者的丢更新。
        let _guard = GALLERY_IO.lock().unwrap();
        let mut recs = read_gallery_inner(&path);
        for img in images {
            recs.push(GalleryRecord {
                id: img.id.clone(),
                ts: ts.clone(),
                uid: req.uid.clone(),
                name: req.name.clone(),
                model: req.model.clone(),
                prompt: req.prompt.clone(),
                negative_prompt: req.negative_prompt.clone(),
                template_id: req.template_id.clone().unwrap_or_default(),
            });
        }
        json_store::save(&path, &recs)?;
    }
    // 裁剪另起一次加锁（不嵌套，避免 std Mutex 重入死锁）；只裁行、不碰图。
    enforce_gallery_limit(data_dir, limit)
}

/// 图片淘汰闸门：`.imgtmp` 内最多保留 mtime 最新的 limit 个文件，超出删除。
/// 【只碰图片字节】按 mtime 判新旧，不读不写 gallery.json——记录可多于图片（图片被删后
/// 记录仍在，查询期现算为「无图」）。limit==0 = 不裁剪（与记录闸门同口径，互不换算）。
pub fn enforce_image_limit(data_dir: &Path, limit: usize) -> Result<(), String> {
    if limit == 0 {
        return Ok(());
    }
    let Ok(rd) = fs::read_dir(tmp_dir(data_dir)) else {
        return Ok(());
    };
    let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        // 取不到 mtime 的文件不参与裁剪：宁可少删，不可误删新旧不明的图片。
        if let Ok(m) = entry.metadata().and_then(|md| md.modified()) {
            files.push((p, m));
        }
    }
    if files.len() <= limit {
        return Ok(());
    }
    // mtime 升序（旧在前）；同刻按路径名兜底，确保裁剪结果确定。
    files.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    for (p, _) in files.iter().take(files.len() - limit) {
        if let Err(e) = fs::remove_file(p) {
            log::warn!(target: "image_ai", "清理过期临时图片失败 {}: {e}", p.display());
        }
    }
    Ok(())
}

/// 记录淘汰闸门：按 ts 保留最新的 limit 行，超出者从 gallery.json 删除。
/// 【只碰记录行】绝不删 `.imgtmp` 图片——图片存亡只由 enforce_image_limit 决定。
/// limit==0 = 不裁剪。
pub fn enforce_gallery_limit(data_dir: &Path, limit: usize) -> Result<(), String> {
    if limit == 0 {
        return Ok(());
    }
    // 「读—裁—写」整段持锁：与 record_gallery 的追加写共用同一把锁，两处不嵌套（防重入死锁）。
    let _guard = GALLERY_IO.lock().unwrap();
    let path = gallery_path(data_dir);
    let mut recs = read_gallery_inner(&path);
    if recs.len() <= limit {
        return Ok(());
    }
    // ts 为 "%Y-%m-%d %H:%M:%S"，字典序即时间序；升序后旧的聚在前，裁剪结果确定。
    recs.sort_by(|a, b| a.ts.cmp(&b.ts));
    recs.drain(..recs.len() - limit);
    json_store::save(&path, &recs)
}

/// 组装画廊查询结果（最新在前）：逐条现算图片是否仍在 `.imgtmp`。
/// 【存在性只在查询期计算】绝不写回 gallery.json——一次查询就会污染持久清单，且「存在」
/// 在下次启动即失真。
pub fn load_gallery_items(data_dir: &Path) -> Vec<GalleryItem> {
    // .imgtmp 只扫一次建 stem→路径 表：避免按记录逐条重扫目录（记录上限可达上百条）。
    let mut by_stem: HashMap<String, PathBuf> = HashMap::new();
    if let Ok(rd) = fs::read_dir(tmp_dir(data_dir)) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    by_stem.insert(stem.to_string(), p);
                }
            }
        }
    }
    // 读清单也走同一把锁：gallery.json 的 I/O 收敛到单一串行点。save 是 tmp+rename 原子
    // 替换、读不会见到半截文件，加锁只为与写共用同一条串行纪律。
    let mut recs = {
        let _guard = GALLERY_IO.lock().unwrap();
        read_gallery_inner(&gallery_path(data_dir))
    };
    recs.sort_by(|a, b| b.ts.cmp(&a.ts));
    recs.into_iter()
        .map(|record| {
            let path = by_stem.get(&record.id).cloned();
            GalleryItem {
                image_available: path.is_some(),
                path: path.map(|p| p.to_string_lossy().to_string()),
                record,
            }
        })
        .collect()
}

// ===================== 单元测试：双闸门 / 会话清理 / 记录往返 / 并发写入 ========================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_ai::{new_id, Config};
    use std::time::Duration;

    /// 在 .imgtmp 造一张假图文件（图片闸门按文件本身处理，扩展名任意）。TempDir 必须先绑定再使用。
    fn fake_image(root: &Path, id: &str) -> PathBuf {
        fs::create_dir_all(tmp_dir(root)).unwrap();
        let p = tmp_dir(root).join(format!("{id}.png"));
        fs::write(&p, b"img").unwrap();
        p
    }

    /// 把文件 mtime 设为给定秒数：图片闸门按 mtime 判新旧，测试必须显式控制顺序，
    /// 不能依赖「写入先后」在文件系统时间粒度下是否可分。
    fn set_mtime(p: &Path, secs: u64) {
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    /// 造一条指定秒数时间戳的画廊记录（"%H:%M:%S" 字典序即时间序，秒数递增即时间递增）。
    fn gallery_rec(id: &str, sec: u32) -> GalleryRecord {
        GalleryRecord {
            id: id.into(),
            ts: format!("2026-09-26 10:00:{sec:02}"),
            uid: Some("u1".into()),
            name: Some("甲".into()),
            model: "m1".into(),
            prompt: "p".into(),
            negative_prompt: "n".into(),
            template_id: String::new(),
        }
    }

    /// 测试里读回画廊清单（LoadError 未实现 Debug，不能对 json_store::load 直接 unwrap）。
    fn load_gallery(root: &Path) -> Vec<GalleryRecord> {
        serde_json::from_str(&fs::read_to_string(gallery_path(root)).unwrap()).unwrap()
    }

    /// 造一条生成请求：画廊记录字段取自请求，故只需补齐这几个字段。
    fn gen_req() -> GenRequest {
        GenRequest {
            provider_id: "openai".into(),
            model: "gpt-image-1".into(),
            prompt: "一张脸".into(),
            size: "1024x1024".into(),
            n: 2,
            ..crate::image_ai::test_req_base()
        }
    }

    #[test]
    fn enforce_image_limit_deletes_only_images() {
        // 按 mtime 判新旧：删最旧、留最新；只碰图片字节、绝不触碰 gallery.json；
        // 文件数等于上限时早退。
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3).map(|_| new_id()).collect();
        for (i, id) in ids.iter().enumerate() {
            let p = fake_image(dir.path(), id);
            set_mtime(&p, 1_000_000 + i as u64); // ids[0] 最旧
        }
        let recs: Vec<GalleryRecord> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| gallery_rec(id, i as u32))
            .collect();
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();
        let before = fs::read_to_string(gallery_path(dir.path())).unwrap();

        enforce_image_limit(dir.path(), 2).unwrap();

        // 最旧 1 张图被删，最新 2 张保留
        assert!(!tmp_dir(dir.path()).join(format!("{}.png", ids[0])).exists());
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[1])).exists());
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[2])).exists());
        // 记录行一字未动：图片闸门绝不触碰 gallery.json
        assert_eq!(
            before,
            fs::read_to_string(gallery_path(dir.path())).unwrap()
        );
        assert_eq!(load_gallery(dir.path()).len(), 3);
        // 文件数已等于上限：再次执行早退，一张不删
        enforce_image_limit(dir.path(), 2).unwrap();
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[1])).exists());
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[2])).exists());

        // 同刻 tie-break：mtime 完全相同必须按路径名确定性地删「较小者」，否则该分支被改坏
        //（比较反转、丢掉 then_with）时结果会随目录遍历顺序漂移，测试却照过。
        let tie_dir = tempfile::tempdir().unwrap();
        let first = fake_image(tie_dir.path(), "tieA");
        let second = fake_image(tie_dir.path(), "tieB");
        set_mtime(&first, 2_000_000);
        set_mtime(&second, 2_000_000); // 同刻
        enforce_image_limit(tie_dir.path(), 1).unwrap();
        // 同刻按路径名升序：tieA.png < tieB.png → 删 tieA、留 tieB
        assert!(!first.exists());
        assert!(second.exists());
    }

    #[test]
    fn zero_limit_disables_both_gates() {
        // 两道闸门同一口径：limit==0 一律视为「不裁剪」。同一目录内同时验证，
        // 顺带锁定两闸互不越界（图片闸不碰记录、记录闸不碰图）。
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3).map(|_| new_id()).collect();
        for id in &ids {
            fake_image(dir.path(), id);
        }
        let recs: Vec<GalleryRecord> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| gallery_rec(id, i as u32))
            .collect();
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();

        enforce_image_limit(dir.path(), 0).unwrap();
        enforce_gallery_limit(dir.path(), 0).unwrap();

        // 三张图全留（图片闸门早返回）
        for id in &ids {
            assert!(tmp_dir(dir.path()).join(format!("{}.png", id)).exists());
        }
        // 三行全留（记录闸门早返回）
        assert_eq!(load_gallery(dir.path()).len(), 3);
    }

    #[test]
    fn enforce_gallery_limit_deletes_only_rows() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3).map(|_| new_id()).collect();
        for id in &ids {
            fake_image(dir.path(), id);
        }
        let recs: Vec<GalleryRecord> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| gallery_rec(id, i as u32))
            .collect();
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();

        enforce_gallery_limit(dir.path(), 2).unwrap();

        // 断言完整的保留集合（ts 升序 = 由旧到新）：只查首元素会漏判「保留最旧而非最新」这类排序错误。
        let kept: Vec<String> = load_gallery(dir.path()).into_iter().map(|r| r.id).collect();
        assert_eq!(kept, vec![ids[1].clone(), ids[2].clone()]);
        // 三张图一个不少：记录闸门绝不删图
        for id in &ids {
            assert!(tmp_dir(dir.path()).join(format!("{}.png", id)).exists());
        }
    }

    #[test]
    fn prepare_session_wipes_tmp_dir_and_keeps_gallery() {
        // 会话清理只针对 .imgtmp：旧临时图片被清、目录被重建；持久画廊记录逐字节原样保留。
        let dir = tempfile::tempdir().unwrap();
        let stale = fake_image(dir.path(), "stale");
        let recs = vec![gallery_rec("a", 0), gallery_rec("b", 1)];
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();
        let before = fs::read_to_string(gallery_path(dir.path())).unwrap();

        Config::prepare_session(dir.path()).unwrap();

        assert!(!stale.exists());
        assert!(tmp_dir(dir.path()).is_dir());
        assert_eq!(
            before,
            fs::read_to_string(gallery_path(dir.path())).unwrap()
        );
    }

    #[test]
    fn gallery_record_round_trips_all_fields() {
        let r = gallery_rec("id1", 0);
        let s = serde_json::to_string(&r).unwrap();
        for key in [
            "\"id\"",
            "\"ts\"",
            "\"uid\"",
            "\"name\"",
            "\"model\"",
            "\"prompt\"",
            "\"negativePrompt\"",
            "\"templateId\"",
        ] {
            assert!(s.contains(key), "序列化缺少字段 {key}: {s}");
        }
        let back: GalleryRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, r.id);
        assert_eq!(back.ts, r.ts);
        assert_eq!(back.uid, r.uid);
        assert_eq!(back.name, r.name);
        assert_eq!(back.model, r.model);
        assert_eq!(back.prompt, r.prompt);
        assert_eq!(back.negative_prompt, r.negative_prompt);
        assert_eq!(back.template_id, r.template_id);
        // 除 id/ts 外均可缺省：精简输入（旧文件）也能解析
        let sparse: GalleryRecord = serde_json::from_str(r#"{"id":"x","ts":"t"}"#).unwrap();
        assert_eq!(sparse.uid, None);
        assert!(sparse.model.is_empty());
        assert!(sparse.template_id.is_empty());
    }

    #[test]
    fn record_gallery_wires_requests_and_query_orders_newest_first() {
        // 画廊查询契约：缺清单按空表；请求字段落进记录；存在性逐条现算；结果最新在前。
        let dir = tempfile::tempdir().unwrap();
        assert!(load_gallery_items(dir.path()).is_empty());
        let images = vec![
            GenImage {
                id: "a".into(),
                path: String::new(),
                bytes: 0,
            },
            GenImage {
                id: "b".into(),
                path: String::new(),
                bytes: 0,
            },
        ];
        let mut req = gen_req();
        req.uid = Some("u9".into());
        req.name = Some("乙".into());
        req.template_id = Some("tpl9".into());
        record_gallery(dir.path(), &images, &req, 100).unwrap();
        fake_image(dir.path(), "a"); // 只留 a 的图：b 的图不存在，查询期应现算为「无图」

        let items = load_gallery_items(dir.path());
        assert_eq!(items.len(), 2);
        let a = items.iter().find(|i| i.record.id == "a").unwrap();
        let b = items.iter().find(|i| i.record.id == "b").unwrap();
        assert!(a.image_available);
        assert!(a.path.is_some());
        assert!(!b.image_available);
        assert!(b.path.is_none());
        // 请求字段落进记录（含模板 id）
        assert_eq!(a.record.uid.as_deref(), Some("u9"));
        assert_eq!(a.record.name.as_deref(), Some("乙"));
        assert_eq!(a.record.model, "gpt-image-1");
        assert_eq!(a.record.prompt, "一张脸");
        assert_eq!(a.record.template_id, "tpl9");
        // 存在性只活在查询期：落盘的 gallery.json 必须**不含** imageAvailable / path 键，
        // 否则一旦有人误把 GalleryItem 写回文件，读回时会被当成持久字段而掩盖。
        let raw = fs::read_to_string(gallery_path(dir.path())).unwrap();
        assert!(
            !raw.contains("imageAvailable"),
            "落盘不应含存在性字段: {raw}"
        );
        assert!(!raw.contains("\"path\""), "落盘不应含图片路径字段: {raw}");

        // 排序：a/b 的 ts 相同（同一秒写入）验不出顺序，另写三条不同 ts 的记录。
        json_store::save(
            &gallery_path(dir.path()),
            &[
                gallery_rec("old", 0),
                gallery_rec("new", 2),
                gallery_rec("mid", 1),
            ],
        )
        .unwrap();
        fake_image(dir.path(), "old"); // 只有旧图还在 .imgtmp（其余已被会话清理）
        let items = load_gallery_items(dir.path());
        let ids: Vec<&str> = items.iter().map(|i| i.record.id.as_str()).collect();
        assert_eq!(ids, ["new", "mid", "old"], "查询结果最新在前");
        assert!(!items[0].image_available, "无图记录应标记不可用");
        assert!(items[2].image_available && items[2].path.is_some());
        // 序列化契约：存在性字段与记录字段同层（flatten），前端按此读取。
        let j = serde_json::to_string(&items[2]).unwrap();
        assert!(j.contains(r#""imageAvailable":true"#), "{j}");
        assert!(j.contains(r#""id":"old""#), "{j}");
    }

    #[test]
    fn concurrent_record_gallery_keeps_all_rows() {
        // 回归 H1：非原子读改写（read→push→save）在并发（这里放大到 8 线程）下会丢更新；
        // 断言最终 8 条一条不少——无锁实现下大概率只剩 1~3 条而失败。
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let n = 8usize;
        let mut handles = Vec::new();
        for i in 0..n {
            let d = data_dir.clone();
            handles.push(std::thread::spawn(move || {
                let mut req = gen_req();
                req.template_id = Some(format!("t{i}"));
                let images = vec![GenImage {
                    id: format!("img{i}"),
                    path: String::new(),
                    bytes: 0,
                }];
                record_gallery(&d, &images, &req, 1000).unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let mut ids: Vec<String> = load_gallery(dir.path()).into_iter().map(|r| r.id).collect();
        ids.sort();
        let expect: Vec<String> = (0..n).map(|i| format!("img{i}")).collect();
        assert_eq!(ids, expect, "并发写入不得丢更新");
    }

    #[test]
    fn new_id_and_now_ts_formats_are_stable() {
        // 两者都是持久化判据的一部分：id 作 .imgtmp 词干与画廊记录键（16 位 hex），
        // ts 作画廊排序键（定长 `%Y-%m-%d %H:%M:%S`，字典序即时间序）。
        let a = new_id();
        assert_ne!(a, new_id());
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "{a}");
        let ts = now_ts();
        assert_eq!(ts.len(), 19, "{ts}");
        assert_eq!(&ts[10..11], " ", "{ts}");
    }
}
