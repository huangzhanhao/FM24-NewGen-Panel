//! Replace Faces 流水线（自旧版 services/replace_service.py 移植）：
//! 「识别并解析 RTF → 布局判定与拦截 → 逐球员抽图 → 写 config.xml」四阶段串行编排。
//!
//! 【约定】进度经 progress 回调上报：阶段锚点与阶段占比**全部由下方 STAGE_* 常量定义**
//! （0 起步，解析 20→30、索引 30→70、读映射 70→74、抽图 74→92、写盘 92、元数据 96、完成 100），
//! 阶段内部再按「第 n 个 / 共 total 个」线性折算进该阶段占用的百分比区间（见 item()），
//! 于是状态栏既有总进度条、又能显示「映射球员头像… 3500/30000 · 51%」。取消经 AtomicBool
//! 在阶段之间与映射循环内部逐项生效；关键步骤经 LogSink 记 INFO（模块 replace）——
//! 生产实现转发 log 门面，由官方插件分发给 stdout/滚动文件/内存缓冲（内存侧再批量
//! emit 成 log:batch 推给前端）。
use crate::face_mapper::{FaceMapper, MapStep, Mode};
use crate::facepack_scanner::{self, PackLayout, PackScan};
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
///
/// 【区间按实测量级分，不按阶段序号均分】重排依据是 2026-09-17 的实测：同一头像包
/// NG_Regens（标准三/28 万图），**优化前**冷启动是「索引 14.6–39 s ▸ 读现有映射 1.5 s ▸
/// 抽图 0.24 s ▸ 解析 0.02 s」——索引独占 70–94% 的时长，而旧分配只给它 6%（40→46），
/// 用户在那几十秒里看到进度条几乎不动；只花 0.02 s 的解析反倒占了 20%。
///
/// 【优化后这套权重偏保守，但故意不再动】并行扫盘 + `file_type()` 把索引压到 0.13 s，
/// 同一场景的耗时大头换成了「读现有映射」1.2 s（整包 1.4 s），此时索引区间偏宽、读映射区间偏窄。
/// 但整条流水线已进 1.5 s 量级，分配误差用户感知不到；而真正需要重调的场景（超大包 + 冷盘 +
/// 首次替换）恰恰又是索引占大头——现状在两种极端之间更稳。实测明细见
/// `docs/替换流程-优化空间详析.md` §8。
const STAGE_RTF: (u8, u8) = (20, 30); // 阶段2：解析 RTF 数据行（实测量级最小，区间同步收窄）
const STAGE_INDEX: (u8, u8) = (30, 70); // 阶段3a：读头像包索引（扫盘或复用缓存）——冷启动的真正大头
const STAGE_XML: (u8, u8) = (70, 74); // 阶段3b：读现有 config.xml（Preserve/Overwrite）
const STAGE_PLAYERS: (u8, u8) = (74, 92); // 阶段3c：逐球员抽图（大名单时占两成上下）

/// 阶段4（写 config.xml）与阶段5（存档元数据 / 完成）的锚点。
/// 【为何提成常量】它们原本是散在 write_stage / run_body 里的字面量 80/90/100；
/// 改 STAGE_* 右端时极易忘记同步，进度条就会在写盘那一步往回跳（92 → 80）。
/// `progress_carries_item_counts_and_never_goes_backwards` 会拦住这类回退。
const STAGE_WRITE: u8 = 92;
const STAGE_SAVE_META: u8 = 96;
const STAGE_DONE: u8 = 100;

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
    /// 本轮**现场扫盘**得到的头像包索引（命中「扫描头像」缓存时是 None）。
    /// 【为什么要带回来】它原来在 map_stage 结束时就丢掉了，于是每次替换都要重扫整包
    /// （实测 28 万图：旧口径单线程 14.6–39 s；改并行 + `file_type()` 后约 0.13 s）。
    /// 命令层拿它回填缓存后，下一次替换同一目录只需比对目录签名（实测约 0.15 s）即可复用。
    /// 失败/取消沿用 None（不刷新缓存，免得把半途的结果当成现状）。
    pub scan: Option<Arc<PackScan>>,
}

/// 执行整条替换流水线，并在收尾统一记录「总耗时」——成功/取消/失败三种结局都记，
/// 不再只在成功分支打印（取消/失败同样要能复盘跑了多久、断在哪一步）。
/// 编排主体在 run_body（四个阶段排队走 + 进度与取消检查点），本函数只负责开始日志与收尾计时。
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
    // 开始日志：把本次任务的关键参数与两个输入路径落进日志，事后可完整复盘当时跑的是什么。
    logger.info("replace", format!(
        "开始替换：模式「{}」｜仅随机人={}｜允许复用={}｜写前备份={}",
        mode_label(&params.mode), yes_no(params.filter_newgen),
        yes_no(params.allow_duplicates), yes_no(params.save_backup)));
    logger.info("replace", format!("头像包：{}", params.img_dir));
    logger.info("replace", format!("RTF 名单：{}", params.rtf_path));

    // 计时只覆盖流水线主体（run_body）：开始那三行日志本身的开销不该算进替换耗时。
    let started = Instant::now();
    let result = run_body(params, progress, cancel, logger);
    // 失败的具体原因由命令层记 ERROR，这里只补一条带耗时的 INFO 收尾行（错误信息不重复两遍）。
    let elapsed = started.elapsed();
    match &result.outcome {
        Outcome::Finished => logger.info("replace", format!(
            "替换完成：共 {} 条映射，总耗时 {:.1?}", result.mapping.len(), elapsed)),
        Outcome::Cancelled => logger.info("replace", format!("替换已取消，总耗时 {:.1?}", elapsed)),
        Outcome::Failed(_) => logger.info("replace", format!("替换失败中止，总耗时 {:.1?}", elapsed)),
    }
    result
}

/// run 的编排主体：四个阶段串行，任一阶段失败/取消都用 Err(RunResult) 短路返回，
/// 「总耗时」收尾日志由外层 run 统一补（这样取消/失败路径也不会漏掉）。
fn run_body(
    params: &ReplaceParams,
    progress: ProgressFn,
    cancel: &AtomicBool,
    logger: &impl LogSink,
) -> RunResult {
    // 阶段1+2：校验并解析 RTF（0 → 20%），得到结构化球员列表与名单摘要（内部会做中文国籍翻译）。
    let (rtf_data, roster) = match parse_stage(params, progress, cancel, logger) {
        Ok(d) => d,
        Err(r) => return r,
    };
    // 阶段3：映射。优先用「扫描头像」的缓存索引建 mapper，否则现场扫描（并把扫出来的索引带回来）。
    let (mapping, fresh_scan) = match map_stage(params, &rtf_data, progress, cancel, logger) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if is_cancelled(cancel) {
        return cancelled();
    }
    // 阶段4：写 config.xml。把映射结果一次性写入头像包目录。
    if let Err(r) = write_stage(params, &mapping, progress, logger) {
        return r;
    }
    // 阶段5：完成（96 → 100）。上报最后两个进度，返回成功与数据（「总耗时」由外层 run 记）。
    progress("保存存档元数据…", STAGE_SAVE_META, 0, 0);
    progress("完成！", STAGE_DONE, 0, 0);
    RunResult {
        outcome: Outcome::Finished,
        rtf_data,
        mapping,
        roster: Some(roster),
        // 只有「成功且本轮是现场扫盘」时才有值：命令层据此回填缓存（失败/取消不回填）。
        scan: fresh_scan,
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
    let t_detect = Instant::now();
    let mut scanner = RtfScanner::new();
    if let Err(e) = scanner.detect_format(Path::new(&params.rtf_path), logger) {
        return Err(failed(&e));
    }
    // 识别只读文件头（HEAD_BYTES），耗时应远小于解析；单独记出来才能确认它确实不是瓶颈。
    let detect_elapsed = t_detect.elapsed();
    if is_cancelled(cancel) {
        return Err(cancelled());
    }
    progress("解析 RTF 文件…", 20, 0, 0);
    let t = Instant::now(); // 计时只覆盖解析（识别在上一步，另行计时）
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
    let parse_elapsed = t.elapsed();
    logger.info("replace", format!(
        "解析名单完成：{} 名球员，耗时 {:.1?}（识别 {:.1?} + 解析 {:.1?}）",
        rtf_data.len(), detect_elapsed + parse_elapsed, detect_elapsed, parse_elapsed));
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

/// 阶段3：识别头像包布局 → 拦截未实现标准 → 逐球员抽图（30% → 92%）。
/// 返回 (映射, 本轮现场扫盘得到的索引)：后者交给命令层回填「扫描头像」缓存，
/// 命中缓存时是 None（缓存里已经是同一份，不必重写）。
fn map_stage(
    params: &ReplaceParams,
    rtf_data: &[PlayerRecord],
    progress: ProgressFn,
    cancel: &AtomicBool,
    logger: &impl LogSink,
) -> Result<(Vec<MappingEntry>, Option<Arc<PackScan>>), RunResult> {
    progress("读取头像包索引…", STAGE_INDEX.0, 0, 0);
    // 【计时口径】索引阶段 = 扫盘（或复用缓存）+ classify + 规则构造。这段以前混在「映射完成」的
    // 耗时里（旧的 t 起于 FaceMapper 构造之前），看不出扫盘单独花了多久；拆开后它的真实成本才可见。
    let cached_index = params.pre_scan.is_some();
    let t_index = Instant::now();
    // 借用 Arc 里的扫描结果（&Arc<PackScan> 自动解引用成 &PackScan），零克隆建 mapper。
    // 只有没命中「扫描头像」缓存时才现场扫盘，扫盘按一级子目录逐项上报（落 STAGE_INDEX 区间）；
    // 命中缓存这一步只是内存转换，无计数可报，锚点就是上面那条。
    //
    // 【为什么这里不再用 FaceMapper::new】它扫完就把索引丢了，于是每次替换都得重扫整包
    // （实测 28 万图 14.6–39 s，而命中缓存只要 10 ms 量级）。拆成两步后，扫出来的 PackScan
    // 可以原样交回命令层入缓存，同样的错误不再犯第二次。
    let (mut mapper, fresh_scan) = match &params.pre_scan {
        Some(scan) => (
            FaceMapper::from_scan(&params.img_dir, scan, &params.rule_config).map_err(|e| failed(&e))?,
            None,
        ),
        None => {
            let scan = facepack_scanner::scan_pack_progress(&params.img_dir, &mut |done, total| {
                item(&mut *progress, "读取头像包索引…", STAGE_INDEX, done, total)
            })
            .map_err(|e| failed(&e))?;
            let mapper = FaceMapper::from_scan(&params.img_dir, &scan, &params.rule_config)
                .map_err(|e| failed(&e))?;
            (mapper, Some(Arc::new(scan)))
        }
    };
    logger.info("replace", format!(
        "头像包索引完成：{}，耗时 {:.1?}（{}）",
        layout_label(&mapper.layout), t_index.elapsed(),
        if cached_index { "命中「扫描头像」缓存，未扫盘" } else { "现场扫盘" }));
    // 泛化拦截：标准规则清单外（尚未实现抽图规则）的标准明确报错（如未来的标准四/五/六）。
    if let PackLayout::Standard { standard, .. } = mapper.layout {
        if !IMPLEMENTED_STANDARDS.contains(&standard) {
            return Err(failed(&format!("标准{standard}的抽图规则暂未实现，请检查标准规则配置")));
        }
    }
    // 索引到此为止；下面这段「读现有映射 + 逐球员抽图」另起一个计时器。
    let t_map = Instant::now();
    // 【抽图起点】generate_mapping 的第一件事就是报 Players 步骤锚点（0/0），即「读现有映射已跑完、
    // 真正开始逐球员抽图」的那一刻；在此置位就能把 ReadXml 与 Players 两段拆开，
    // 不必为此改动 face_mapper 的签名（进度回调本来就是唯一的分段信号）。
    let mut t_players: Option<Instant> = None;
    // 逐球员抽图（内部同样逐项检查 cancel）；返回 None 可能是被取消，也可能是其他失败，据标志区分。
    // 进度按子步骤落到不同区间（读现有映射 STAGE_XML / 逐球员 STAGE_PLAYERS），两段窗口分开，
    // 否则前一段走满 100% 之后后一段又从 0% 起算，状态栏会看到进度往回跳。
    let mapping = mapper
        .generate_mapping(
            rtf_data,
            params.mode,
            params.allow_duplicates,
            cancel,
            &mut |step, done, total| {
                if step == MapStep::Players && t_players.is_none() {
                    t_players = Some(Instant::now());
                }
                map_item(&mut *progress, step, done, total);
            },
        )
        .ok_or_else(|| if is_cancelled(cancel) { cancelled() } else { failed("映射失败") })?;
    // 抽图耗时 = 首个 Players 锚点 → 映射结束；读现有映射耗时 = 两者差额
    //（Generate 模式没有读 XML 这一步，差额≈0，正好自证口径）。
    let map_elapsed = t_map.elapsed();
    let players_elapsed = t_players.map(|p| p.elapsed()).unwrap_or_default();
    logger.info("replace", format!(
        "映射完成：{} 条（模式「{}」），读现有映射 {:.1?}｜逐球员抽图 {:.1?}｜小计 {:.1?}",
        mapping.len(), mode_label(&params.mode),
        map_elapsed.saturating_sub(players_elapsed), players_elapsed, map_elapsed));
    Ok((mapping, fresh_scan))
}

/// 阶段4：把映射结果一次性写入头像包目录的 config.xml。
fn write_stage(
    params: &ReplaceParams,
    mapping: &[MappingEntry],
    progress: ProgressFn,
    logger: &impl LogSink,
) -> Result<(), RunResult> {
    progress("生成 config.xml…", STAGE_WRITE, 0, 0);
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
        scan: None,
    }
}

fn cancelled() -> RunResult {
    RunResult {
        outcome: Outcome::Cancelled,
        rtf_data: Vec::new(),
        mapping: Vec::new(),
        roster: None,
        scan: None,
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
        // 两段窗口（解析 20→30、抽图 74→92）分开，正是为了抽图不会从 0% 起算把进度条拉回去。
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
        // 取消时机用**阶段文案**而不是魔法数字：进度区间按实测量级重排过一次，写死 20/40 会随之失效。
        // 「解析 RTF 文件…」是解析阶段的锚点，在此置位等价于「解析过程中点取消」。
        let result = run(
            &params,
            &mut |s, _, _, _| {
                if s.starts_with("解析 RTF 文件") { cancel.store(true, Ordering::Relaxed); }
            },
            &cancel, &logger);
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
        // 在「读取头像包索引…」锚点置位：扫盘照样跑完，但紧接着的映射阶段一进门就发现已取消，
        // 于是不写 config.xml（等价于原来的「进度 40 时置位」，且不依赖具体百分比）。
        let result = run(
            &params,
            &mut |s, _, _, _| {
                if s.starts_with("读取头像包索引") { cancel.store(true, Ordering::Relaxed); }
            },
            &cancel, &logger);
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
