//! 【新手导读】Replace Faces 流水线（自旧版 services/replace_service.py 移植）。
//! 把「校验 RTF → 解析 → 映射 → 写 config.xml」四个阶段串成一条完整流程。
//! 进度通过回调上报（值与 v1 对齐：20/40/80/90/100），取消经 AtomicBool 逐球员生效。
use crate::assets::{ETH_CFG, NAT_TRANSLATION};
use crate::face_mapper::{FaceMapper, Mode};
use crate::rtf_parser::{PlayerRecord, RtfParser};
use crate::xml_parser::{self, MappingEntry};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// 一次替换所需的全部参数（由 commands 层从前端收集后组装）。
pub struct ReplaceParams {
    pub rtf_path: String,       // RTF 名单文件路径
    pub img_dir: String,        // 头像包目录
    pub mode: Mode,             // 替换模式
    pub filter_newgen: bool,    // 是否只处理随机人
    pub allow_duplicates: bool, // 是否允许图片复用
    pub save_backup: bool,      // 写入前是否备份
}

/// 流水线最终结果：成功 / 被取消 / 失败（带错误信息）。
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Finished,
    Cancelled,
    Failed(String),
}

/// run() 的返回体：除了结果状态，还把解析出的球员与映射带回（供前端预览）。
pub struct RunResult {
    pub outcome: Outcome,
    pub rtf_data: Vec<PlayerRecord>,
    pub mapping: Vec<MappingEntry>,
}

/// 【核心函数】执行整条替换流水线。
/// progress 是一个回调函数（&mut dyn FnMut）：每进入一个阶段就调用它上报状态文字和百分比。
/// cancel 是跨线程取消标志：阶段之间、以及映射循环内部都会检查它。
pub fn run(params: &ReplaceParams, progress: &mut dyn FnMut(&str, u8), cancel: &AtomicBool) -> RunResult {
    // 阶段1：校验 RTF（0%）。check_rtf_valid 返回 Ok(true)/Ok(false)/Err 三种，分别处理。
    progress("Validating RTF file...", 0);
    let mut parser = RtfParser::new();
    match parser.check_rtf_valid(Path::new(&params.rtf_path)) {
        Ok(true) => {}
        Ok(false) => return failed("The RTF file is invalid!"),
        Err(e) => return failed(&e),
    }
    if cancel.load(Ordering::Relaxed) {
        return cancelled();
    }
    // 阶段2：解析 RTF（20%），得到结构化球员列表（内部会做中文国籍翻译）。
    progress("Parsing RTF file...", 20);
    let rtf_data = match parser.parse_rtf(Path::new(&params.rtf_path), params.filter_newgen, &NAT_TRANSLATION) {
        Ok(d) => d,
        Err(e) => return failed(&e),
    };
    if cancel.load(Ordering::Relaxed) {
        return cancelled();
    }
    // 阶段3：映射（40%）。先扫描头像包建索引，再为每个球员选图。
    progress("Mapping player to image...", 40);
    let mut mapper = match FaceMapper::new(&params.img_dir) {
        Ok(m) => m,
        Err(e) => return failed(&e),
    };
    let mapping = match mapper.generate_mapping(
        &rtf_data,
        params.mode,
        params.allow_duplicates,
        cancel,
        &ETH_CFG,
    ) {
        Some(m) => m,
        // 返回 None 可能是被取消，也可能是其他失败；根据 cancel 标志区分。
        None => return if cancel.load(Ordering::Relaxed) { cancelled() } else { failed("映射失败") },
    };
    if cancel.load(Ordering::Relaxed) {
        return cancelled();
    }
    // 阶段4：写 config.xml（80%）。把映射结果一次性写入头像包目录。
    progress("Generating config.xml...", 80);
    if let Err(e) = xml_parser::write_xml(&mapping, Path::new(&params.img_dir), params.save_backup) {
        return failed(&e);
    }
    // 阶段5：完成（90 → 100）。上报最后两个进度，返回成功与数据。
    progress("Saving profile metadata...", 90);
    progress("Finished! :)", 100);
    RunResult { outcome: Outcome::Finished, rtf_data, mapping }
}

// 下面两个是构造 RunResult 的小工具，避免重复写字段（失败/取消时数据为空）。
fn failed(msg: &str) -> RunResult {
    RunResult { outcome: Outcome::Failed(msg.into()), rtf_data: Vec::new(), mapping: Vec::new() }
}

fn cancelled() -> RunResult {
    RunResult { outcome: Outcome::Cancelled, rtf_data: Vec::new(), mapping: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (tempfile::TempDir, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("pack");
        for eth in ["SpanMed", "Caucasian", "Central European", "African", "MENA"] {
            fs::create_dir_all(img.join(eth)).unwrap();
            for i in 0..2 {
                fs::write(img.join(eth).join(format!("e{i}.png")), b"png").unwrap();
            }
        }
        let rtf = dir.path().join("list.rtf");
        fs::write(
            &rtf,
            "| UID       | Nat  | 2nd Nat | Name   |    |    |   |\n\
             | 2000472008| ESP  |         | Pepe   | 1  | 12 | 1 |\n\
             | 2000472009| USA  |         | John   | 1  | 12 | 0 |\n",
        ).unwrap();
        (dir, rtf.to_string_lossy().to_string(), img.to_string_lossy().to_string())
    }

    #[test]
    fn happy_path_writes_config_xml() {
        let (_dir, rtf, img) = setup();
        let params = ReplaceParams {
            rtf_path: rtf, img_dir: img, mode: Mode::Generate,
            filter_newgen: true, allow_duplicates: true, save_backup: false,
        };
        let cancel = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run(&params, &mut |s, v| steps.push((s.to_string(), v)), &cancel);
        assert_eq!(result.outcome, Outcome::Finished);
        assert_eq!(result.rtf_data.len(), 2);
        assert_eq!(result.mapping.len(), 2);
        // 进度值单调且终值 100
        assert_eq!(steps.last().unwrap().1, 100);
        // 产物可用 xml_parser 解析回来
        let xml = xml_parser::parse_xml(Path::new(&params.img_dir).join("config.xml").as_path()).unwrap();
        assert!(xml.contains_key("r-2000472008"));
    }

    #[test]
    fn invalid_rtf_fails() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.rtf");
        fs::write(&bad, "not an rtf").unwrap();
        let params = ReplaceParams {
            rtf_path: bad.to_string_lossy().to_string(),
            img_dir: dir.path().to_string_lossy().to_string(),
            mode: Mode::Generate, filter_newgen: true, allow_duplicates: true, save_backup: false,
        };
        let cancel = AtomicBool::new(false);
        let result = run(&params, &mut |_, _| {}, &cancel);
        assert!(matches!(result.outcome, Outcome::Failed(m) if m.contains("invalid")));
    }

    #[test]
    fn cancel_before_write_leaves_no_config() {
        let (_dir, rtf, img) = setup();
        let params = ReplaceParams {
            rtf_path: rtf, img_dir: img, mode: Mode::Generate,
            filter_newgen: true, allow_duplicates: true, save_backup: false,
        };
        let cancel = AtomicBool::new(false);
        // 解析完成后立刻取消（在进度 40 时置位）
        let result = run(&params, &mut |_, v| { if v == 40 { cancel.store(true, Ordering::Relaxed); } }, &cancel);
        assert_eq!(result.outcome, Outcome::Cancelled);
        assert!(!Path::new(&params.img_dir).join("config.xml").exists());
    }
}
