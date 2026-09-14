//! Replace Faces 流水线（自旧版 services/replace_service.py 移植）：
//! 「识别并解析 RTF → 布局判定与拦截 → 逐球员抽图 → 写 config.xml」四阶段串行编排。
//!
//! 【约定】进度经 progress 回调上报：阶段锚点值沿用 v1（0/20/40/80/90/100），阶段内部再按
//! 「第 n 个 / 共 total 个」线性折算进该阶段占用的百分比区间（见下方 STAGE_* 常量与 item()），
//! 于是状态栏既有总进度条、又能显示「映射球员头像… 3500/30000 · 51%」。取消经 AtomicBool
//! 在阶段之间与映射循环内部逐项生效；关键步骤经 LogSink 记 INFO（模块 replace）——
//! 生产实现转发 log 门面，由官方插件分发给 stdout/滚动文件/内存缓冲（内存侧再批量
//! emit 成 log:batch 推给前端）。
use crate::face_mapper::{FaceMapper, MapStep, Mode};
use crate::facepack_scanner::PackLayout;
use crate::logger::LogSink;
use crate::templates::{IMPLEMENTED_STANDARDS, RuleConfig};
use crate::rtf_scanner::{PlayerRecord, RosterSummary, RtfScanner};
use crate::xml_parser::{self, MappingEntry};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// 进度回调：status = 阶段文案；value = 总进度百分比；current/total = 阶段内的
/// 「第 n 个 / 共 total 个」（纯阶段锚点传 0/0，状态栏据此不显示计数）。
pub type ProgressFn<'a> = &'a mut dyn FnMut(&str, u8, usize, usize);

/// 各阶段在总进度条上占用的百分比区间（左端是该阶段的起点，也就是它对外报的锚点值）。
/// 阶段内的逐项上报用 item() 把 done/total 折算进这个区间，所以区间必须相邻不重叠，
/// 否则后一阶段的 0% 会把进度条从前面拉回来。
const STAGE_RTF: (u8, u8) = (20, 40); // 阶段2：解析 RTF 数据行
const STAGE_INDEX: (u8, u8) = (40, 46); // 阶段3a：读头像包索引（扫盘或复用「扫描头像」缓存）
const STAGE_XML: (u8, u8) = (46, 52); // 阶段3b：读现有 config.xml（Preserve/Overwrite）
const STAGE_PLAYERS: (u8, u8) = (52, 80); // 阶段3c：逐球员抽图（通常是大头，留最宽）

/// 阶段内逐项上报：把 done/total 折算进 [from, to] 区间，并带上原始计数。
/// done 超界时夹到区间右端（换算误差下别越过下一阶段的起点）。
/// 【total == 0】正常走不到（扫描器在总数为 0 时一个回调都不发），这里兜底给区间右端。
fn item(progress: ProgressFn, status: &str, (from, to): (u8, u8), done: usize, total: usize) {
    let span = (to - from) as usize;
    let value = if total == 0 {
        to
    } else {
        from + (span * done / total).min(span) as u8
    };
    progress(status, value, done, total);
}

/// 一次替换所需的全部参数（由 commands 层从前端收集后组装）。
pub struct ReplaceParams {
    pub rtf_path: String,       // RTF 名单文件路径
    pub img_dir: String,        // 头像包目录
    pub mode: Mode,             // 替换模式
    pub filter_newgen: bool,    // 是否只处理随机人
    pub allow_duplicates: bool, // 是否允许图片复用
    pub save_backup: bool,      // 写入前是否备份
    /// 「扫描头像」命令缓存的包索引（按 img_dir 命中即复用，避免重复扫盘）。
    /// Arc 共享缓存里的同一份——克隆只涨引用计数，不复制数万图片名表；None = 现场新扫描。
    pub pre_scan: Option<Arc<crate::facepack_scanner::PackScan>>,
    /// 标准规则配置（命令层已从 data/rules 加载；测试用 RuleConfig::builtin()）。
    pub rule_config: RuleConfig,
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
    /// 名单摘要（格式/语言/条数）：解析成功后带回，随 replace:finished 一并回传前端，
    /// 让名单预览不必再点一次「扫描RTF」就知道名单格式；失败/取消为 None。
    pub roster: Option<RosterSummary>,
}

/// 执行整条替换流水线。run 只做编排（进度与取消检查点），各阶段实现在下方 *_stage。
/// progress 是进度回调：阶段锚点报状态文字 + 百分比，阶段内部再报「第 n 个 / 共 total 个」（emit 给前端）。
/// logger 是日志出口（LogSink）：生产 = log 门面（官方插件承接），测试 = ChannelLogger 捕获。
/// cancel 是跨线程取消标志：阶段之间、以及映射循环内部都会检查它。
/// 【为什么阶段返回 Result<_, RunResult>】阶段内失败/取消统一用 Err(RunResult) 短路，
/// 替代散落在各处的 `return failed(...)`，主干一眼看清「四个阶段排队走」。
pub fn run(
    params: &ReplaceParams,
    progress: ProgressFn,
    cancel: &AtomicBool,
    logger: &impl LogSink,
) -> RunResult {
    let started = Instant::now();
    // 开始日志：把本次任务的关键参数与两个输入路径落进日志，事后可完整复盘当时跑的是什么。
    logger.info("replace", format!(
        "开始替换：模式「{}」｜仅随机人={}｜允许复用={}｜写前备份={}",
        mode_label(&params.mode), yes_no(params.filter_newgen),
        yes_no(params.allow_duplicates), yes_no(params.save_backup)));
    logger.info("replace", format!("头像包：{}", params.img_dir));
    logger.info("replace", format!("RTF 名单：{}", params.rtf_path));

    // 阶段1+2：校验并解析 RTF（0 → 20%），得到结构化球员列表与名单摘要（内部会做中文国籍翻译）。
    let (rtf_data, roster) = match parse_stage(params, progress, cancel, logger) {
        Ok(d) => d,
        Err(r) => return r,
    };
    // 阶段3：映射（40%）。优先用「扫描头像」的缓存索引建 mapper，否则现场扫描。
    let mapping = match map_stage(params, &rtf_data, progress, cancel, logger) {
        Ok(m) => m,
        Err(r) => return r,
    };
    if is_cancelled(cancel) {
        return cancelled();
    }
    // 阶段4：写 config.xml（80%）。把映射结果一次性写入头像包目录。
    if let Err(r) = write_stage(params, &mapping, progress, logger) {
        return r;
    }
    // 阶段5：完成（90 → 100）。上报最后两个进度，返回成功与数据。
    progress("保存存档元数据…", 90, 0, 0);
    logger.info("replace", format!(
        "替换完成：共 {} 条映射，总耗时 {:.1?}", mapping.len(), started.elapsed()));
    progress("完成！", 100, 0, 0);
    RunResult {
        outcome: Outcome::Finished,
        rtf_data,
        mapping,
        roster: Some(roster),
    }
}

/// 阶段1+2：识别 RTF 格式并解析出结构化名单（0 → 20%），连同名单摘要一并返回。
/// 格式识别失败 → failed("RTF 格式不合法：…")（带首个不合法字段定位，前端弹窗展示）。
fn parse_stage(
    params: &ReplaceParams,
    progress: ProgressFn,
    cancel: &AtomicBool,
    logger: &impl LogSink,
) -> Result<(Vec<PlayerRecord>, RosterSummary), RunResult> {
    progress("识别 RTF 格式…", 0, 0, 0);
    let mut scanner = RtfScanner::new();
    if let Err(e) = scanner.detect_format(Path::new(&params.rtf_path), logger) {
        return Err(failed(&e));
    }
    if is_cancelled(cancel) {
        return Err(cancelled());
    }
    progress("解析 RTF 文件…", 20, 0, 0);
    let t = Instant::now(); // 计时只覆盖解析（识别在上一步，不计入本行耗时）
    // 逐行上报（大名单上万行；订阅方按百分比去重后才 emit，不会刷爆 IPC），
    // 百分比落在 STAGE_RTF 区间上，计数用原始行数（含表头行，与「扫描RTF」同口径）。
    let rtf_data = scanner
        .parse_roster_progress(
            Path::new(&params.rtf_path),
            params.filter_newgen,
            logger,
            &mut |done, total| item(&mut *progress, "解析 RTF 文件…", STAGE_RTF, done, total),
        )
        .map_err(|e| failed(&e))?;
    // 解析完立即检查取消（原版口径）：不等头像包扫描跑完才停，大包扫描可能耗时很久。
    if is_cancelled(cancel) {
        return Err(cancelled());
    }
    logger.info("replace", format!(
        "解析名单完成：{} 名球员，耗时 {:.1?}", rtf_data.len(), t.elapsed()));
    // 摘要就地从扫描器状态 + 记录列表汇总（趁 rtf_data 交出去之前），随结果回传前端。
    let summary = RosterSummary::from_records(&scanner, &rtf_data);
    Ok((rtf_data, summary))
}

/// 把 face_mapper 报上来的子步骤计数折算进对应区间（MapStep → 区间 + 阶段文案的唯一定点）。
/// total == 0 表示「该子步骤刚开始、还没有计数」，用区间左端当锚点，让状态栏先亮出阶段名——
/// 否则第一项计数到达前（或该步骤无事可做时）状态栏还停在上一个阶段。
fn map_item(progress: ProgressFn, step: MapStep, done: usize, total: usize) {
    let (window, label) = match step {
        MapStep::ReadXml => (STAGE_XML, "读取现有映射…"),
        MapStep::Players => (STAGE_PLAYERS, "映射球员头像…"),
    };
    if total == 0 {
        progress(label, window.0, 0, 0);
    } else {
        item(progress, label, window, done, total);
    }
}

/// 阶段3：识别头像包布局 → 拦截未实现标准 → 逐球员抽图（40% → 80%）。
fn map_stage(
    params: &ReplaceParams,
    rtf_data: &[PlayerRecord],
    progress: ProgressFn,
    cancel: &AtomicBool,
    logger: &impl LogSink,
) -> Result<Vec<MappingEntry>, RunResult> {
    progress("读取头像包索引…", STAGE_INDEX.0, 0, 0);
    let t = Instant::now();
    // 借用 Arc 里的扫描结果（&Arc<PackScan> 自动解引用成 &PackScan），零克隆建 mapper。
    // 只有没命中「扫描头像」缓存时才现场扫盘，扫盘按一级子目录逐项上报（落 STAGE_INDEX 区间）；
    // 命中缓存这一步只是内存转换，无计数可报，锚点就是上面那条。
    let mut mapper = match &params.pre_scan {
        Some(scan) => FaceMapper::from_scan(&params.img_dir, scan, &params.rule_config),
        None => FaceMapper::new(&params.img_dir, &params.rule_config, &mut |done, total| {
            item(&mut *progress, "读取头像包索引…", STAGE_INDEX, done, total)
        }),
    }
    .map_err(|e| failed(&e))?;
    logger.info("replace", format!("头像包识别：{}", layout_label(&mapper.layout)));
    // 泛化拦截：标准规则清单外（尚未实现抽图规则）的标准明确报错（如未来的标准四/五/六）。
    if let PackLayout::Standard { standard, .. } = mapper.layout {
        if !IMPLEMENTED_STANDARDS.contains(&standard) {
            return Err(failed(&format!("标准{standard}的抽图规则暂未实现，请检查标准规则配置")));
        }
    }
    // 逐球员抽图（内部同样逐项检查 cancel）；返回 None 可能是被取消，也可能是其他失败，据标志区分。
    // 进度按子步骤落到不同区间（读现有映射 STAGE_XML / 逐球员 STAGE_PLAYERS），两段窗口分开，
    // 否则前一段走满 100% 之后后一段又从 0% 起算，状态栏会看到进度往回跳。
    let mapping = mapper
        .generate_mapping(
            rtf_data,
            params.mode,
            params.allow_duplicates,
            cancel,
            &mut |step, done, total| map_item(&mut *progress, step, done, total),
        )
        .ok_or_else(|| if is_cancelled(cancel) { cancelled() } else { failed("映射失败") })?;
    logger.info("replace", format!(
        "映射完成：{} 条（模式「{}」），耗时 {:.1?}", mapping.len(), mode_label(&params.mode), t.elapsed()));
    Ok(mapping)
}

/// 阶段4：把映射结果一次性写入头像包目录的 config.xml（80%）。
fn write_stage(
    params: &ReplaceParams,
    mapping: &[MappingEntry],
    progress: ProgressFn,
    logger: &impl LogSink,
) -> Result<(), RunResult> {
    progress("生成 config.xml…", 80, 0, 0);
    let t = Instant::now();
    xml_parser::write_xml(mapping, Path::new(&params.img_dir), params.save_backup)
        .map_err(|e| failed(&e))?;
    logger.info("replace", format!(
        "写入 config.xml：{} 条映射，写前备份={}，耗时 {:.1?}",
        mapping.len(), yes_no(params.save_backup), t.elapsed()));
    Ok(())
}

// 下面是编排用的小工具：取消检查点、日志用中文标签，以及两个 RunResult 构造器（失败/取消时数据为空）。

/// 阶段间统一取消检查点（对 cancel.load(Relaxed) 的语义化封装：只需原子性，不需同步顺序）。
fn is_cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

/// 模式的中文名（日志用）。
fn mode_label(mode: &Mode) -> &'static str {
    match mode {
        Mode::Generate => "生成",
        Mode::Preserve => "保留",
        Mode::Overwrite => "覆写",
    }
}

/// 布局判定的中文描述（日志用）。
fn layout_label(layout: &PackLayout) -> String {
    match layout {
        PackLayout::Flat => "标准一：图片平铺，无种族子目录".into(),
        PackLayout::Empty => "空包（无子目录且无图片）".into(),
        PackLayout::Standard { standard, complete } => format!(
            "标准{standard}，{}",
            if *complete { "配置目录齐全" } else { "配置目录不全" }),
        PackLayout::Unknown => "无法识别标准：子目录与 FolderList 均不匹配，将递归全池随机抽图".into(),
    }
}

/// 布尔值的中文（日志用）。
fn yes_no(v: bool) -> &'static str {
    if v { "是" } else { "否" }
}

fn failed(msg: &str) -> RunResult {
    RunResult {
        outcome: Outcome::Failed(msg.into()),
        rtf_data: Vec::new(),
        mapping: Vec::new(),
        roster: None,
    }
}

fn cancelled() -> RunResult {
    RunResult {
        outcome: Outcome::Cancelled,
        rtf_data: Vec::new(),
        mapping: Vec::new(),
        roster: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::{ChannelLogger, LogLine};
    use std::collections::BTreeSet;
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
            pre_scan: None,
            rule_config: RuleConfig::builtin(),
        };
        let cancel = AtomicBool::new(false);
        let mut steps = Vec::new();
        let (logger, log_rx) = ChannelLogger::channel();
        let result = run(&params, &mut |s, v, _, _| steps.push((s.to_string(), v)), &cancel, &logger);
        assert_eq!(result.outcome, Outcome::Finished);
        assert_eq!(result.rtf_data.len(), 2);
        assert_eq!(result.mapping.len(), 2);
        // 进度值单调且终值 100
        assert_eq!(steps.last().unwrap().1, 100);
        // 产物可用 xml_parser 解析回来
        let xml = xml_parser::parse_xml(Path::new(&params.img_dir).join("config.xml").as_path()).unwrap();
        assert!(xml.contains_key("r-2000472008"));
        // 关键步骤日志全部发出（同序在通道里，前端经 log:batch 实时回显）；末条应为完成行。
        let logs: Vec<LogLine> = log_rx.try_iter().collect();
        assert!(logs.len() >= 5, "关键步骤日志缺失：{logs:?}");
        assert!(logs.iter().all(|l| l.module == "replace"));
        assert!(logs.last().unwrap().message.contains("替换完成"), "末条日志应为完成行：{logs:?}");
    }

    #[test]
    fn progress_carries_item_counts_and_never_goes_backwards() {
        // 阶段内逐项上报：解析名单要带「第 n 行 / 共 total 行」，逐球员抽图要带「第 n 个 / 共 total 个」。
        // 两段窗口（解析 20→40、抽图 52→80）分开，正是为了抽图不会从 0% 起算把进度条拉回去。
        let (_dir, rtf, img) = setup();
        let params = ReplaceParams {
            rtf_path: rtf, img_dir: img, mode: Mode::Generate,
            filter_newgen: true, allow_duplicates: true, save_backup: false,
            pre_scan: None,
            rule_config: RuleConfig::builtin(),
        };
        let cancel = AtomicBool::new(false);
        let (logger, _log_rx) = ChannelLogger::channel();
        let mut steps: Vec<(String, u8, usize, usize)> = Vec::new();
        let result = run(&params, &mut |s, v, c, t| steps.push((s.to_string(), v, c, t)), &cancel, &logger);
        assert_eq!(result.outcome, Outcome::Finished);

        // 总百分比单调不减，终值 100
        assert!(steps.windows(2).all(|w| w[0].1 <= w[1].1), "进度出现回退：{steps:?}");
        assert_eq!(steps.last().unwrap().1, 100);

        // 解析阶段：带行数计数，且最后一条走满（done == total）
        let parse: Vec<_> = steps.iter().filter(|s| s.0.starts_with("解析 RTF 文件")).collect();
        let last_parse = parse.last().expect("解析阶段应上报逐行进度");
        assert!(last_parse.3 > 0, "解析阶段应带总行数：{steps:?}");
        assert_eq!(last_parse.2, last_parse.3, "解析进度应走满：{steps:?}");

        // 抽图阶段：Generate 模式没有「读现有映射」那一步，计数就是名单人数
        let players: Vec<_> = steps.iter().filter(|s| s.0.starts_with("映射球员头像")).collect();
        let last_players = players.last().expect("抽图阶段应上报逐球员进度");
        assert_eq!(last_players.3, 2, "总人数应为名单条数：{steps:?}");
        assert_eq!(last_players.2, 2, "抽图进度应走满：{steps:?}");
        assert!(!steps.iter().any(|s| s.0.starts_with("读取现有映射")), "Generate 模式不该报读映射阶段：{steps:?}");
    }

    #[test]
    fn cancel_after_parse_skips_pack_scan() {
        // 在 progress(20)（解析名单中）置位取消：应在解析完成后立即停，
        // 不进入头像包扫描。img_dir 故意指向不存在的目录——若取消检查失效，
        // 流程会走到扫描并以 Failed（目录不存在）收场，用结果状态区分两条路径。
        let (dir, rtf, _img) = setup();
        let params = ReplaceParams {
            rtf_path: rtf,
            img_dir: dir.path().join("nonexistent").to_string_lossy().to_string(),
            mode: Mode::Generate, filter_newgen: true, allow_duplicates: true, save_backup: false,
            pre_scan: None,
            rule_config: RuleConfig::builtin(),
        };
        let cancel = AtomicBool::new(false);
        let (logger, _log_rx) = ChannelLogger::channel();
        let result = run(&params, &mut |_, v, _, _| { if v == 20 { cancel.store(true, Ordering::Relaxed); } }, &cancel, &logger);
        assert_eq!(result.outcome, Outcome::Cancelled);
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
            pre_scan: None,
            rule_config: RuleConfig::builtin(),
        };
        let cancel = AtomicBool::new(false);
        let (logger, _log_rx) = ChannelLogger::channel();
        let result = run(&params, &mut |_, _, _, _| {}, &cancel, &logger);
        assert!(matches!(result.outcome, Outcome::Failed(m) if m.contains("RTF 格式不合法")));
    }

    #[test]
    fn cancel_before_write_leaves_no_config() {
        let (_dir, rtf, img) = setup();
        let params = ReplaceParams {
            rtf_path: rtf, img_dir: img, mode: Mode::Generate,
            filter_newgen: true, allow_duplicates: true, save_backup: false,
            pre_scan: None,
            rule_config: RuleConfig::builtin(),
        };
        let cancel = AtomicBool::new(false);
        let (logger, _log_rx) = ChannelLogger::channel();
        // 解析完成后立刻取消（在进度 40 时置位）
        let result = run(&params, &mut |_, v, _, _| { if v == 40 { cancel.store(true, Ordering::Relaxed); } }, &cancel, &logger);
        assert_eq!(result.outcome, Outcome::Cancelled);
        assert!(!Path::new(&params.img_dir).join("config.xml").exists());
    }

    #[test]
    fn standard3_pack_maps_via_rule_blocks() {
        // 标准三（FolderList 62 目录）+ 规则块检索，三条路径各覆盖一名球员：
        //   ESP 码 1 → 国家块的 "default" → 本国基础池 Spain；
        //   JAM 码 1 → 族群块 Afrocaribbean 的 directly → Mestizo；
        //   USA 码 0 → 族群块 CaucasianNA 的 "default" → CaucasianNA（包内该目录为空）
        //              → 第一级回退仍是它 → 第二级 Fallback=random，从递归全池抽到图。
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("pack");
        let rule_config = RuleConfig::builtin();
        for name in &rule_config.standard3.folder_list {
            fs::create_dir_all(img.join(name)).unwrap();
        }
        fs::write(img.join("Spain").join("sp1.png"), b"png").unwrap();
        fs::write(img.join("Spain").join("sp2.png"), b"png").unwrap();
        fs::write(img.join("Mestizo").join("me1.png"), b"png").unwrap();
        fs::write(img.join("France").join("fr1.png"), b"png").unwrap();
        let rtf = dir.path().join("list.rtf");
        fs::write(
            &rtf,
            "| UID       | Nat  | 2nd Nat | Name   |    |    |   |\n\
             | 2000472008| ESP  |         | Pepe   | 1  | 12 | 1 |\n\
             | 2000472009| JAM  |         | John   | 1  | 12 | 1 |\n\
             | 2000472010| USA  |         | Jack   | 1  | 12 | 0 |\n",
        ).unwrap();
        let params = ReplaceParams {
            rtf_path: rtf.to_string_lossy().to_string(),
            img_dir: img.to_string_lossy().to_string(),
            mode: Mode::Generate, filter_newgen: true, allow_duplicates: true, save_backup: false,
            pre_scan: None,
            rule_config,
        };
        let cancel = AtomicBool::new(false);
        let (logger, _log_rx) = ChannelLogger::channel();
        let result = run(&params, &mut |_, _, _, _| {}, &cancel, &logger);
        assert_eq!(result.outcome, Outcome::Finished);
        assert_eq!(result.mapping.len(), 3);
        let by_uid = |u: &str| result.mapping.iter().find(|m| m.uid == u).unwrap().clone();
        // 国家块 "default" → 本国基础池。
        let esp = by_uid("r-2000472008");
        assert_eq!(esp.ethnicity, "Spain");
        assert!(["sp1", "sp2"].contains(&esp.image.as_str()));
        // 族群块 directly → 指定目录。
        assert_eq!(by_uid("r-2000472009").ethnicity, "Mestizo");
        // 目标目录为空 → 回退链末端 Fallback=random，仍产出可定位的真实文件。
        let usa = by_uid("r-2000472010");
        assert!(["Spain", "Mestizo", "France"].contains(&usa.ethnicity.as_str()),
                "兜底只应在有图的目录里抽，实际：{}", usa.ethnicity);
        assert!(xml_parser::resolve_image_file(Path::new(&params.img_dir), &usa.ethnicity, &usa.image).is_some());
        // config.xml 解析回目录相对路径口径。
        let xml = xml_parser::parse_xml(Path::new(&params.img_dir).join("config.xml").as_path()).unwrap();
        assert_eq!(xml["r-2000472008"].ethnicity, "Spain");
        assert_eq!(xml["r-2000472009"].ethnicity, "Mestizo");
    }

    #[test]
    fn flat_pack_end_to_end() {
        // 标准一平铺端到端：无子目录、只有根图（含一张点号文件名）。
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("pack");
        fs::create_dir_all(&img).unwrap();
        for i in 0..4 {
            fs::write(img.join(format!("face{i}.png")), b"png").unwrap();
        }
        fs::write(img.join("x.1.png"), b"png").unwrap();
        let rtf = dir.path().join("list.rtf");
        fs::write(
            &rtf,
            "| UID       | Nat  | 2nd Nat | Name   |    |    |   |\n\
             | 2000472008| ESP  |         | Pepe   | 1  | 12 | 1 |\n\
             | 2000472009| USA  |         | John   | 1  | 12 | 0 |\n\
             | 2000472010| FRA  |         | Pierre | 1  | 12 | 3 |\n",
        ).unwrap();
        let params = ReplaceParams {
            rtf_path: rtf.to_string_lossy().to_string(),
            img_dir: img.to_string_lossy().to_string(),
            mode: Mode::Generate, filter_newgen: true, allow_duplicates: true, save_backup: false,
            pre_scan: None,
            rule_config: RuleConfig::builtin(),
        };
        let cancel = AtomicBool::new(false);
        let (logger, _log_rx) = ChannelLogger::channel();
        let result = run(&params, &mut |_, _, _, _| {}, &cancel, &logger);
        assert_eq!(result.outcome, Outcome::Finished);
        assert_eq!(result.mapping.len(), 3);
        for m in &result.mapping {
            assert!(m.ethnicity.is_empty()); // 平铺种族为空串
            // 每条映射的图名都能在包根命中真实文件（file_stem 规则贯通）。
            assert!(xml_parser::resolve_image_file(Path::new(&params.img_dir), "", &m.image).is_some());
        }
        // 全程未创建任何子目录。
        let subdirs = fs::read_dir(&params.img_dir).unwrap()
            .filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count();
        assert_eq!(subdirs, 0);
        // 写出的 config.xml 每行 from="图名"，解析回 ethnicity==""。
        let xml = xml_parser::parse_xml(Path::new(&params.img_dir).join("config.xml").as_path()).unwrap();
        assert_eq!(xml.len(), 3);
        assert!(xml.values().all(|e| e.ethnicity.is_empty()));
    }

    #[test]
    fn unknown_nested_pack_end_to_end() {
        // 无法识别布局（含孙目录）→ 递归全池全随机：
        // from 统一写相对路径（根图 "r"、子目录 "weirdA/i"、孙目录 "weirdA/sub/j"），解析回相对目录。
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("pack");
        fs::create_dir_all(img.join("weirdA/sub")).unwrap();
        fs::write(img.join("weirdA/i.png"), b"png").unwrap();
        fs::write(img.join("weirdA/sub/j.png"), b"png").unwrap();
        fs::write(img.join("r.png"), b"png").unwrap();
        let rtf = dir.path().join("list.rtf");
        fs::write(
            &rtf,
            "| UID       | Nat  | 2nd Nat | Name   |    |    |   |\n\
             | 2000472008| ESP  |         | Pepe   | 1  | 12 | 1 |\n\
             | 2000472009| USA  |         | John   | 1  | 12 | 0 |\n\
             | 2000472010| FRA  |         | Pierre | 1  | 12 | 3 |\n",
        ).unwrap();
        let params = ReplaceParams {
            rtf_path: rtf.to_string_lossy().to_string(),
            img_dir: img.to_string_lossy().to_string(),
            // 禁重复：池恰好 3 张、名单 3 人 → from 全集必等于递归池全集（确定性断言）。
            mode: Mode::Generate, filter_newgen: true, allow_duplicates: false, save_backup: false,
            pre_scan: None,
            rule_config: RuleConfig::builtin(),
        };
        let cancel = AtomicBool::new(false);
        let (logger, _log_rx) = ChannelLogger::channel();
        let result = run(&params, &mut |_, _, _, _| {}, &cancel, &logger);
        assert_eq!(result.outcome, Outcome::Finished);
        assert_eq!(result.mapping.len(), 3);
        // 禁重复 + 池恰好 3 张 → from 全集 = 递归池全集（根/子/孙全部进入结果）。
        let froms: BTreeSet<String> = result.mapping.iter()
            .map(|m| xml_parser::record_from(&m.ethnicity, &m.image)).collect();
        assert_eq!(froms, BTreeSet::from([
            "r".to_string(), "weirdA/i".to_string(), "weirdA/sub/j".to_string()
        ]));
        // config.xml 解析回 (相对目录, 词干)，且 resolve 能按同口径命中真实文件。
        let xml = xml_parser::parse_xml(Path::new(&params.img_dir).join("config.xml").as_path()).unwrap();
        assert_eq!(xml.len(), 3);
        for e in xml.values() {
            assert!(xml_parser::resolve_image_file(Path::new(&params.img_dir), &e.ethnicity, &e.image).is_some());
        }
    }
}
