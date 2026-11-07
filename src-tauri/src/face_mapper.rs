//! 球员 → 头像映射（自旧版 core/FaceMapper.py 移植，含 0-10 种族码修正规则）。
//!
//! 这是整个应用的「大脑」：按每个球员的国籍 + 种族码，经所选标准的抽图规则
//! （templates::PickRule）算出目标种族目录并随机取图，产出「球员 UID → 图片」的映射条目。
//! 三模式（保留/覆写/生成）决定与「本档该组的存档快照」的合并方式（判据来源见
//! `FaceMapper::existing_xml`，**不是**头像包里的 live config.xml——那是全档共享的一份）；包布局扫描与标准判定在
//! facepack_scanner，本模块只消费其结果。
//!
//! 【日志】关键处置经 `LogSink` 记录（模块名 facemap），一律记**上层看不到的内部信息**，
//! 不与 replace_flow 的阶段日志重复：现有映射的读取结果与失败原因（原本被 `unwrap_or_default`
//! 静默吞掉）、Preserve 的补漏人数与池剔除张数、未抽到图的球员清单按成因分组（用户「某某还是原脸」
//! 的第一手线索）、以及选图规则的回落统计（这批脸为什么看着不对的第一手线索）。
use crate::facepack_scanner::{self, PackLayout, PackScan};
use crate::logger::LogSink;
use crate::rtf_scanner::PlayerRecord;
use crate::templates::{self, PickRule, RuleConfig, SkipReason};
use crate::xml_parser::{self, MappingEntry};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// 未抽到图的球员清单在一条日志里最多列出多少个 UID（超出只报总数，避免大名单刷屏）。
const SKIP_LOG_MAX: usize = 10;

/// `generate_mapping` 内部两个「可数」子步骤。
/// 【为何要这个枚举】两段的进度要落在总进度条的不同区间上，否则前一段走满 100% 之后，
/// 后一段又从 0% 起算，状态栏会看到进度往回跳。由调用方（replace_flow）负责把
/// 「第 n 个 / 共 total 个」折算进各自区间，本模块只报原始计数、不认识百分比。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapStep {
    /// 读现有 config.xml（只有 Preserve/Overwrite 会做；Generate 从零开始，跳过）。
    ReadXml,
    /// 逐球员抽图。
    Players,
}

/// 替换模式（对应前端三个模式按钮）。用枚举而不是字符串，非法值在 parse 处即被拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Preserve,  // 保留：已换过的不动，只给新球员补图
    Overwrite, // 覆写：名单内球员全部重抽，名单外保留
    Generate,  // 生成：无视现有 config.xml，从零生成全部映射
}

impl Mode {
    /// 把前端传来的字符串解析为 Mode 枚举（未知字符串返回错误）。
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "Preserve" => Ok(Mode::Preserve),
            "Overwrite" => Ok(Mode::Overwrite),
            "Generate" => Ok(Mode::Generate),
            _ => Err(format!("未知模式: {s}")),
        }
    }
}

/// 头像映射器：持有扫描判定的布局与对应标准的抽图模型，负责按模式编排并组装映射条目。
pub struct FaceMapper {
    /// 布局判定结果（facepack_scanner::classify 按 templates 的 FolderList 判定）；replace_flow 据此拦截未实现标准。
    pub layout: PackLayout,
    /// 抽图模型（标准一平铺 / 标准二种族匹配 / 标准三细分目录检索 / 退化布局递归全池），池的所有权在模型内。
    rule: Box<dyn PickRule>,
    /// **判据来源**：Preserve/Overwrite 要读的「现有映射」文件。由命令层传当前档当前组的
    /// 存档快照路径（`paths::group_xml_path(user_dir, cur_prf, cur_group)`）；`None` 或文件不存在
    /// ＝该档在该头像包目录下还没有历史 → 按空表处理。
    ///
    /// 【为什么不能读 `img_dir/config.xml`】那份 live 文件是**所有档共享**的（config.xml 躺在头像包
    /// 目录里，一个目录只有一份），它的「归属」全靠切档时 save/restore 配对维持，而
    /// `restore_group_xml` 有三条静默跳过分支（该档在此目录无快照、目录当时不可用、copy 失败）。
    /// 任一条命中，live 里就还是**上一个档**（或上次会话）的残留内容：此时按 live 判断
    /// 「哪些球员已经换过脸」，会把别档的映射当成本档的历史——Preserve 直接跳过重抽、Overwrite
    /// 把别档名单外的行原样带回，而且错结果会被收尾同步写进本档快照，第二次替换起不可逆。
    /// 快照按 (档, 组) 唯一，读它天然免疫串档（详见 `docs` 与本文件回归测试
    /// `overwrite_ignores_other_profiles_live_config_xml`）。
    existing_xml: Option<PathBuf>,
}

impl FaceMapper {
    /// 扫描头像包并建立索引（「扫盘 + 判定 + 选规则」一体的便捷入口）。
    ///
    /// 【仅测试】生产路径不再经过它：替换流水线要**拿到扫出来的索引本身**去回填「扫描头像」缓存
    /// （见 `replace_flow::map_stage` 与 `RunResult::scan`），所以那边自己调
    /// `scan_pack_progress` + `from_scan`。留这个包装只是让测试少写两行，故标 cfg(test)
    /// 免得发布构建报 dead_code。
    #[cfg(test)]
    pub fn new(
        img_dir: &str,
        config: &RuleConfig,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Self, String> {
        let scan = crate::facepack_scanner::scan_pack_progress(
            img_dir,
            progress,
            &AtomicBool::new(false),
        )?;
        // 测试沿用「判据 = 头像包里的 config.xml」的老口径：fixture 把旧映射直接写在包目录里，
        // 于是这些用例不必为快照另造一份文件。生产由 commands 层传当前档的快照路径。
        let existing = Some(PathBuf::from(img_dir).join("config.xml"));
        Self::from_scan(&scan, config, existing)
    }

    /// 用现成的扫描结果构造（「扫描头像」命令的缓存复用入口）。
    /// 【借用而非所有权】内部只读 scan（classify 与 for_layout 都吃 &PackScan），
    /// 大包索引（数万图片名）一份克隆就是几十 MB 白复制，不强制调用方克隆。
    /// `existing_xml` 见字段注释——传当前档的档快照路径，**不要传 `img_dir/config.xml`**。
    /// 【为什么不收 img_dir】布局判定与抽图池全部来自 scan，img_dir 只在扫描那一步用得到；
    /// 留着这个字段只会诱使后来者再写一次 `img_dir/config.xml`——那正是串档的来源。
    pub fn from_scan(
        scan: &PackScan,
        config: &RuleConfig,
        existing_xml: Option<PathBuf>,
    ) -> Result<Self, String> {
        // classify 输入剔除垃圾目录（__MACOSX 等不参与判定）——与「扫描头像」共用同一前导。
        let layout = facepack_scanner::classify_scan(scan, &config.folder_lists());
        Ok(Self {
            layout,
            rule: templates::for_layout(layout, scan, config),
            existing_xml,
        })
    }

    /// 三模式映射总入口；返回 None 表示被取消（对等旧版 generate_mapping）。
    /// cancel 是跨线程的「取消标志」，处理大名单时逐项检查，用户点取消能及时停下。
    /// progress 只报原始计数（MapStep + 第 n 个 / 共 total 个），百分比折算由调用方负责。
    /// logger 记本模块的内部处置（现有映射读取结果、补漏人数、未抽到图的球员），模块名 facemap。
    pub fn generate_mapping(
        &mut self,
        rtf_data: &[PlayerRecord],
        mode: Mode,
        duplicates: bool, // 是否允许图片复用
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(MapStep, usize, usize),
        logger: &impl LogSink,
    ) -> Option<Vec<MappingEntry>> {
        // 开始前先看一眼是否已取消（Ordering::Relaxed 是最宽松的内存顺序，这里只需原子性）。
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        // Preserve/Overwrite 需要「本档现有映射」作为基础；Generate 从零开始，用空表。
        // 读进来的过程按条上报（大快照可上万行）。Generate 没有这一步，连 ReadXml
        // 都不报——否则状态栏会先亮一下「读取现有映射」又立刻切到下一阶段。
        // 【读的是档快照，不是 img_dir/config.xml】原因见 existing_xml 字段注释；快照缺失
        // （该档在此目录还没换过脸）就当空表，等价于「本档无历史」，不再继承别档残留。
        let xml_data = match mode {
            Mode::Preserve | Mode::Overwrite => {
                progress(MapStep::ReadXml, 0, 0); // 步骤锚点：先让状态栏亮出「读取现有映射」，再逐条报
                match self.existing_xml.as_deref() {
                    // 无来源 / 快照尚未产生（该档在此目录第一次换脸）：都等价于「本档无历史」，
                    // 属正常情况，来历已由 replace_flow 记过一行，此处不再重复。
                    None => HashMap::new(),
                    Some(p) if !p.is_file() => HashMap::new(),
                    Some(path) => match xml_parser::parse_xml_progress(path, &mut |done, total| {
                        progress(MapStep::ReadXml, done, total)
                    }) {
                        Ok(map) => {
                            logger.info(
                                "facemap",
                                format!("现有映射：读取 {} 条（{}）", map.len(), path.display()),
                            );
                            map
                        }
                        Err(e) => {
                            // 【不可静默】读取失败不能 `unwrap_or_default()` 一把抹平：文件损坏读出 0 条时，
                            // Preserve 会把所有球员当成新球员重抽、Overwrite 会丢掉名单外的旧映射，
                            // 而用户只看到「头像全变了」。按空表继续（不阻断流程），但要留痕说明原因。
                            logger.warn(
                                "facemap",
                                format!(
                                    "读取现有映射失败（{}）：{}，按空表处理",
                                    path.display(),
                                    e
                                ),
                            );
                            HashMap::new()
                        }
                    },
                }
            }
            Mode::Generate => HashMap::new(),
        };
        // Preserve + 禁重复：把本档现有映射已占用的图片从池中剔除（避免新球员抽到已被占用的图）。
        // used 用 from 全路径做键（种族目录非空 → "种族/图名"，平铺 → "图名"），由规则内部精确比对。
        if !duplicates && mode == Mode::Preserve {
            let used: BTreeSet<String> = xml_data
                .values()
                .map(|e| xml_parser::record_from(&e.ethnicity, &e.image))
                .collect();
            // 记剔除张数：后续若报「未抽到图」，这条能直接说明池是被历史映射压空的
            //（used 里可能含包中不存在的图，规则按全路径比对，池里没有的自然剔不掉）。
            let n = used.len();
            self.rule.exclude_used(&used);
            if n > 0 {
                logger.info(
                    "facemap",
                    format!("禁重复：已按现有映射从抽图池剔除已占用的 {n} 张图"),
                );
            }
        }
        // 按模式分发到具体处理函数。抽图前先报一次步骤锚点（0/0）：Generate 没有读 XML 那一步，
        // 这个锚点是它唯一的机会把状态栏从「读取头像包索引」切到「映射球员头像」。
        progress(MapStep::Players, 0, 0);
        // 【xml_data 按值交出】Preserve/Overwrite 都要把「旧映射原样带回」，把所有权传下去后即可
        // 直接 `into_values()` 搬进结果 Vec；沿用 & 引用就必须 `values().cloned()` 整表克隆一遍
        // （15 万条实测约 0.1 s，纯白复制）。上面的 exclude_used 只借用了一次，此处移动不冲突。
        let result = match mode {
            Mode::Preserve => {
                self.process_preserve(rtf_data, xml_data, duplicates, cancel, progress, logger)
            }
            Mode::Overwrite => {
                self.process_overwrite(rtf_data, xml_data, duplicates, cancel, progress, logger)
            }
            Mode::Generate => self.process_generate(rtf_data, duplicates, cancel, progress, logger),
        };
        // 取消（None）时回落计数只跑了一半，打出来只会误导，故只在跑完全程时汇总。
        if result.is_some() {
            report_rule_stats(&*self.rule, logger);
        }
        result
    }

    /// Preserve：跳过 XML 已存在的 UID，只给新球员建映射，旧映射原样保留。
    /// `xml_data` 按值接收：旧映射要原样带回结果，拿所有权才能 `into_values()` 搬运而不克隆。
    fn process_preserve(
        &mut self,
        rtf_data: &[PlayerRecord],
        xml_data: HashMap<String, MappingEntry>,
        duplicates: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(MapStep, usize, usize),
        logger: &impl LogSink,
    ) -> Option<Vec<MappingEntry>> {
        let mut mapping = Vec::new();
        // filter 只留下 XML 中还没有的 UID（即新球员）。先收集成 Vec：进度分母要的是「本次真正要抽图的
        // 球员数」（过滤后的），拿 rtf_data.len() 当分母会让进度条永远走不满。
        // 该闭包只借用 xml_data 一小会儿，collect 结束即释放，不影响下面把它移走。
        let targets: Vec<&PlayerRecord> = rtf_data
            .iter()
            .filter(|p| !xml_data.contains_key(&p.uid))
            .collect();
        let total = targets.len();
        // 「哪些被跳过、要补几个」是本模块独有的信息：上层只见到最终条数，看不出补了谁。
        logger.info(
            "facemap",
            format!(
                "保留模式：名单 {} 人中 {} 人已有映射，本次补 {} 名",
                rtf_data.len(),
                rtf_data.len() - total,
                total
            ),
        );
        let mut skipped = Vec::new(); // 抽不到图的球员（汇总见 report_skipped）
        for (i, p) in targets.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return None; // 逐项检查取消，及时中断
            }
            match self.build_player_mapping(p, duplicates) {
                Some(m) => mapping.push(m),
                None => skipped.push(SkippedPlayer::of(p, self.rule.last_skip_reason())),
            }
            progress(MapStep::Players, i + 1, total);
        }
        report_skipped(&skipped, logger);
        // 原样保留 XML 中所有旧映射（into_values 直接搬所有权，不克隆整表）。
        mapping.extend(xml_data.into_values());
        Some(mapping)
    }

    /// Overwrite：名单内球员全部重抽，名单外的旧映射保留。
    fn process_overwrite(
        &mut self,
        rtf_data: &[PlayerRecord],
        xml_data: HashMap<String, MappingEntry>,
        duplicates: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(MapStep, usize, usize),
        logger: &impl LogSink,
    ) -> Option<Vec<MappingEntry>> {
        let mut rest = xml_data; // 直接接管旧映射的所有权，逐个剔除名单内的（不再整表克隆一份）
        let mut mapping = Vec::new();
        let mut skipped = Vec::new(); // 抽不到图的球员（汇总见 report_skipped）
        let total = rtf_data.len();
        for (i, p) in rtf_data.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(m) = self.build_player_mapping(p, duplicates) {
                mapping.push(m);
                rest.remove(&p.uid); // 名单内球员全部重抽（从 rest 中移除旧的）
            } else {
                // 【为什么不删掉旧的】重抽失败就保留他原本的映射，至少不会把已换好的人打回原脸。
                skipped.push(SkippedPlayer::of(p, self.rule.last_skip_reason()));
            }
            progress(MapStep::Players, i + 1, total);
        }
        // 【重抽与保留的各自数量要可见】上层只拿到最终总条数，看不出这轮实际重抽了几个、
        // 名单外带回了多少条旧映射——后者正是「误把别档残留夹带进结果」时能一眼发现的地方。
        logger.info(
            "facemap",
            format!(
                "覆写模式：名单 {} 名中已重抽 {} 名，名单外保留 {} 条旧映射",
                rtf_data.len(),
                mapping.len(),
                rest.len()
            ),
        );
        report_skipped(&skipped, logger);
        // 剩下的（名单外）保留。注意：HashMap 的迭代顺序本就未定义，所以保留项在结果里的相对顺序
        // 与改造前 `values().cloned()` 同属「不确定但等价」——不要写依赖该顺序的断言。
        // 重抽失败的那些 UID 也没被 remove，所以他们的旧映射同样在这里被带回。
        mapping.extend(rest.into_values());
        Some(mapping)
    }

    /// Generate：从零为名单里每个球员生成映射，不管现有 config.xml。
    fn process_generate(
        &mut self,
        rtf_data: &[PlayerRecord],
        duplicates: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(MapStep, usize, usize),
        logger: &impl LogSink,
    ) -> Option<Vec<MappingEntry>> {
        let mut mapping = Vec::new();
        let mut skipped = Vec::new(); // 抽不到图的球员（汇总见 report_skipped）
        let total = rtf_data.len();
        // 【Generate 的副作用要说在前面】它从零生成、结果整份写回 config.xml：现有映射里**不在本名单
        // 中**的条目（退役/转会走的球员）会被直接丢弃。这一点上层看不见（replace_flow 只报最终条数），
        // 而「旧头像怎么全没了」恰是最需要能在日志里自证的反馈。
        // 【为什么放在循环前】中途取消时这一行也不会漏——取消后返回 None，循环后的日志压根跑不到。
        let ignored = match &self.existing_xml {
            Some(p) if p.is_file() => format!(
                "，现有映射（{}）不作判据且其名单外条目将被丢弃",
                p.display()
            ),
            _ => String::new(),
        };
        logger.info(
            "facemap",
            format!("生成模式：名单 {total} 名全部重抽{ignored}"),
        );
        for (i, p) in rtf_data.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            match self.build_player_mapping(p, duplicates) {
                Some(m) => mapping.push(m),
                None => skipped.push(SkippedPlayer::of(p, self.rule.last_skip_reason())),
            }
            progress(MapStep::Players, i + 1, total);
        }
        report_skipped(&skipped, logger);
        Some(mapping)
    }

    /// 单球员映射：按扫描出的标准规则抽图（规则选择见 templates::for_layout）。
    /// 返回 None 表示该球员无法映射（跳过）。
    fn build_player_mapping(&mut self, p: &PlayerRecord, duplicates: bool) -> Option<MappingEntry> {
        // PickedImage.dir = 图片相对头像包根的目录（"" = 根，可多级 "a/b"），原样写入 ethnicity 字段；
        // config.xml 的 from 由 write_xml 经 record_from 拼成统一相对路径。
        let img = self.rule.pick(p, duplicates)?;
        Some(MappingEntry {
            uid: p.uid.clone(),
            ethnicity: img.dir,
            image: img.stem,
        })
    }
}

/// 一名没抽到图的球员：UID + 姓名/国籍/种族码 + 成因。
/// 【为什么带姓名】用户手里有的是名单上的姓名，日志只给裸 UID 根本对不上号。
struct SkippedPlayer {
    uid: String,
    name: String,
    nat: String,
    code: String,
    reason: SkipReason,
}

impl SkippedPlayer {
    fn of(p: &PlayerRecord, reason: SkipReason) -> Self {
        Self {
            uid: p.uid.clone(),
            name: p.name.clone(),
            nat: p.primary_nat.clone(),
            code: p.ethnicity_code.clone(),
            reason,
        }
    }

    /// 日志样例格式：`UID 姓名（国籍/种族码）`。
    fn label(&self) -> String {
        format!("{} {}（{}/{}）", self.uid, self.name, self.nat, self.code)
    }
}

/// 汇总「抽不到图」的球员：按成因分组，每组一条 WARN 报总数 + 至多 SKIP_LOG_MAX 个样例
/// （对等 rtf_scanner 的丢行日志口径）。
/// 【为什么不逐条报】「池耗尽」是大名单 + 禁重复时的常规结局，几百上千条 WARN 会把日志刷爆，
/// 而用户真正要的是「有多少人没图、因为什么、从哪几个开始断的」——分组带样例的汇总就够。
/// 【为什么分组】不同成因的处置完全不同（改规则 / 改兜底策略 / 常规结局 / 换头像包），
/// 一句固定文案只会把排查的人带偏。
fn report_skipped(skipped: &[SkippedPlayer], logger: &impl LogSink) {
    if skipped.is_empty() {
        return;
    }
    let mut groups: std::collections::BTreeMap<SkipReason, Vec<&SkippedPlayer>> =
        std::collections::BTreeMap::new();
    for s in skipped {
        groups.entry(s.reason).or_default().push(s);
    }
    for (reason, group) in groups {
        let head: Vec<String> = group.iter().take(SKIP_LOG_MAX).map(|s| s.label()).collect();
        let tail = if group.len() > SKIP_LOG_MAX {
            format!(" 等 {} 名", group.len())
        } else {
            String::new()
        };
        let samples = format!("{}{}", head.join("、"), tail);
        let msg = match reason {
            SkipReason::NoNationRule => format!(
                "{} 名球员因国籍未配置在规则的 Ethnics 表内而被跳过\
                 （在「标准规则」里补配该国国籍后重跑即可配图）：{samples}",
                group.len()
            ),
            SkipReason::RuleSkip => format!(
                "{} 名球员因标准三兜底策略为「跳过」而未配图\
                 （目标目录在头像包内不可用；把规则 Fallback 改为 random 可改为全池取图）：{samples}",
                group.len()
            ),
            // 标准三里这个成因只剩「FolderList 内所有目录的牌都发完、且已发牌也空」
            // 这一种（Deck 一轮发完会原地重洗，不会真的抽干），等价于空包；
            // 标准一/标准二/Unknown 仍可能在禁重复下真的抽干，所以文案两义都留着。
            SkipReason::PoolExhausted => format!(
                "{} 名球员没抽到图：可用目录里已无剩余图片\
                 （空包或禁重复下已发完；需要全覆盖可勾选「允许复用」或换头像包）：{samples}",
                group.len()
            ),
            SkipReason::EmptyPack => format!(
                "{} 名球员没抽到图：头像包内没有可用图片（请核对头像包目录是否选对）：{samples}",
                group.len()
            ),
        };
        logger.warn("facemap", msg);
    }
}

/// 抽图统计：选图规则内部的回落（首选目录不可用、改走他处取图）与类别命中数对上层本来不可见，
/// 映射结束后一次性亮出计数。两类都是设计内行为、不算异常，所以用 INFO；
/// 但量级异常时（如整包目录缺失、进去的全是 WF）这条就是「这批脸为什么看着不对」的第一手线索。
fn report_rule_stats(rule: &dyn PickRule, logger: &impl LogSink) {
    let parts: Vec<String> = rule
        .stats()
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| format!("{} {n} 名", stat_label(k)))
        .collect();
    if !parts.is_empty() {
        logger.info("facemap", format!("抽图统计：{}", parts.join("，")));
    }
}

/// stats 事件名 → 中文短语（未知键原样透出：新规则忘了登记也不至于丢信息）。
fn stat_label(key: &str) -> &str {
    match key {
        "target_dir_fallback" => "目标目录不可用、改从全池取图",
        "owner_fallback" => "候选目录不可用、改从本国基础池取图",
        "pool_fallback" => "候选与本国目录均不可用、改从 FolderList 内的其他目录取图",
        "wf_family" => "进女球员类别（WF）",
        "staff_family" => "进职员类别（Staff）",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::{ChannelLogger, LogLine};
    use crate::templates::RuleConfig;
    use std::path::Path;

    /// 大多数用例不看日志：接收端一丢，ChannelLogger 发出的行被直接丢弃（send 失败忽略）。
    /// 要断言日志内容的用例另行 `let (logger, log_rx) = ChannelLogger::channel();` 留住接收端。
    fn sink() -> ChannelLogger {
        ChannelLogger::channel().0
    }

    /// 测试用规则配置：内置出厂默认。
    fn config() -> RuleConfig {
        RuleConfig::builtin()
    }

    fn facepack(dir: &tempfile::TempDir) -> String {
        for eth in [
            "SpanMed",
            "Caucasian",
            "Central European",
            "MENA",
            "MESA",
            "Seasian",
            "South American",
            "Asian",
            "African",
            "SAMed",
            "EECA",
            "Italmed",
            "YugoGreek",
            "Scandinavian",
        ] {
            let d = dir.path().join(eth);
            std::fs::create_dir_all(&d).unwrap();
            for i in 0..3 {
                std::fs::write(d.join(format!("e{i}.png")), b"png").unwrap();
            }
        }
        dir.path().to_string_lossy().to_string()
    }

    fn player(uid: &str, nat: &str, code: &str) -> PlayerRecord {
        PlayerRecord {
            uid: uid.into(),
            primary_nat: nat.into(),
            sec_nat: String::new(),
            name: "T".into(),
            hair_length: "1".into(),
            hair_color: "1".into(),
            ethnicity_code: code.into(),
            ..Default::default()
        }
    }

    #[test]
    fn unknown_layout_uses_recursive_pool() {
        // 主题：Unknown 布局（递归全池全随机）。① 部分抽取不重复；② 池恰好用尽时三层级全可达；
        // ③ 映射的 ethnicity 为相对目录路径（可多级），from/resolve 全程用相对路径口径。
        // ① 禁重复 + 6 张图、3 名球员：3 条映射互不重复（无重复语义与标准无关）。
        let mixed = tempfile::tempdir().unwrap();
        for d in ["ESP", "SpanMed"] {
            let dir = mixed.path().join(d);
            std::fs::create_dir_all(&dir).unwrap();
            for i in 0..3 {
                std::fs::write(dir.join(format!("e{i}.png")), b"png").unwrap();
            }
        }
        let mixed_img = mixed.path().to_string_lossy().to_string();
        let mut mapper = FaceMapper::new(&mixed_img, &config(), &mut |_, _| {}).unwrap();
        assert_eq!(mapper.layout, PackLayout::Unknown);
        let data: Vec<PlayerRecord> = (0..3).map(|i| player(&format!("r-30{i}"), "ESP", "1")).collect();
        let cancel = AtomicBool::new(false);
        let logger = sink();
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, false, &cancel, &mut |_, _, _| {}, &logger)
            .unwrap();
        assert_eq!(mapping.len(), 3);
        // 【按 from 全路径比，不能只比词干】两个目录的词干集相同（e0..e2），
        // 只看 `m.image` 会把 ESP/e0 与 SpanMed/e0 当成同一张图，断言随时假失败。
        let images: BTreeSet<_> = mapping
            .iter()
            .map(|m| xml_parser::record_from(&m.ethnicity, &m.image))
            .collect();
        assert_eq!(images.len(), 3); // 无重复
        // ② 无法识别的包（含孙目录）→ 递归全池：池恰好 3 张、球员 3 名 ⇒ 必然抽完全部图片
        //（确定性断言，不依赖随机）。
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().to_string_lossy().to_string();
        std::fs::create_dir_all(dir.path().join("weirdA/sub")).unwrap();
        std::fs::write(dir.path().join("weirdA/i.png"), b"png").unwrap();
        std::fs::write(dir.path().join("weirdA/sub/j.png"), b"png").unwrap();
        std::fs::write(dir.path().join("root.png"), b"png").unwrap();
        let mut mapper = FaceMapper::new(&img, &config(), &mut |_, _| {}).unwrap();
        assert_eq!(mapper.layout, PackLayout::Unknown);
        let data: Vec<PlayerRecord> = (0..3).map(|i| player(&format!("r-50{i}"), "ZZZ", "1")).collect();
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, false, &cancel, &mut |_, _, _| {}, &logger)
            .unwrap();
        assert_eq!(mapping.len(), 3);
        let froms: BTreeSet<String> = mapping
            .iter()
            .map(|m| xml_parser::record_from(&m.ethnicity, &m.image))
            .collect();
        // 根、子、孙三个层级的图片全部进入抽图结果。
        assert_eq!(
            froms,
            BTreeSet::from([
                "root".to_string(),
                "weirdA/i".to_string(),
                "weirdA/sub/j".to_string()
            ])
        );
        // ③ 相对路径口径：resolve 用同一 ethnicity 字符串即可命中真实文件。
        for m in &mapping {
            assert!(
                xml_parser::resolve_image_file(Path::new(&img), &m.ethnicity, &m.image).is_some()
            );
        }
    }

    #[test]
    fn preserve_keeps_existing_entries_and_excludes_occupied_images() {
        // 主题：Preserve。① 已有映射原样保留、只补新球员；② 未勾选「允许复用」时 config.xml
        // 已占用的图先按 from 全路径从池中剔除（旧映射本身不重抽）。
        let cancel = AtomicBool::new(false);
        let logger = sink();
        // ① 现有 config.xml 已有 r-2001（African/old）：保留旧值，r-2002 新建。
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        crate::xml_parser::write_xml(
            &[MappingEntry {
                uid: "r-2001".into(),
                ethnicity: "African".into(),
                image: "old".into(),
            }],
            Path::new(&img),
            false,
        )
        .unwrap();
        let mut mapper = FaceMapper::new(&img, &config(), &mut |_, _| {}).unwrap();
        let data = vec![player("r-2001", "ESP", "1"), player("r-2002", "ESP", "1")];
        let mapping = mapper
            .generate_mapping(&data, Mode::Preserve, true, &cancel, &mut |_, _, _| {}, &logger)
            .unwrap();
        let kept = mapping.iter().find(|m| m.uid == "r-2001").unwrap();
        assert_eq!((kept.ethnicity.as_str(), kept.image.as_str()), ("African", "old"));
        assert!(mapping.iter().any(|m| m.uid == "r-2002"));
        // ② 另一份包：SpanMed 的 e0/e1 已被旧映射占用（该目录 3 张图），禁重复下新球员只能拿 e2。
        let dir2 = tempfile::tempdir().unwrap();
        let img2 = facepack(&dir2);
        crate::xml_parser::write_xml(
            &[
                MappingEntry { uid: "r-2001".into(), ethnicity: "SpanMed".into(), image: "e0".into() },
                MappingEntry { uid: "r-2002".into(), ethnicity: "SpanMed".into(), image: "e1".into() },
            ],
            Path::new(&img2),
            false,
        )
        .unwrap();
        let mut mapper = FaceMapper::new(&img2, &config(), &mut |_, _| {}).unwrap();
        let data = vec![player("r-2001", "ESP", "1"), player("r-2003", "ESP", "1")];
        let mapping = mapper
            .generate_mapping(&data, Mode::Preserve, false, &cancel, &mut |_, _, _| {}, &logger)
            .unwrap();
        let new = mapping.iter().find(|m| m.uid == "r-2003").unwrap();
        assert_eq!(
            (new.ethnicity.as_str(), new.image.as_str()),
            ("SpanMed", "e2"),
            "旧映射占用的 e0/e1 不得再发给新球员"
        );
        let old: Vec<_> = mapping.iter().filter(|m| m.uid == "r-2001").collect();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].image, "e0");
    }

    #[test]
    fn overwrite_redraws_roster_and_keeps_entries_outside_it() {
        // Overwrite 的正面场景（串档回归那条只覆盖了「名单外保留 0 条」）：快照里比名单多出的旧映射
        // 必须原样带回（process_overwrite 的 rest.into_values() 回填），
        // 退役/转会走的球员会静默变回原脸。
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let snap_dir = dir.path().join("snap");
        std::fs::create_dir_all(&snap_dir).unwrap();
        let snap = snap_dir.join("group.xml");
        // 本档快照：r-7001（名单内 → 会被重抽）与 r-7002（名单外 → 必须原样保留）。
        std::fs::write(
            &snap,
            config_xml_text(&[("r-7001", "SpanMed/e0"), ("r-7002", "African/e1")]),
        )
        .unwrap();
        let scan = crate::facepack_scanner::scan_pack(&img).unwrap();
        let mut mapper = FaceMapper::from_scan(&scan, &config(), Some(snap.clone())).unwrap();
        let roster = vec![player("r-7001", "ESP", "1")]; // 名单只有一人
        let cancel = AtomicBool::new(false);
        let (logger, log_rx) = ChannelLogger::channel();
        let mapping = mapper
            .generate_mapping(
                &roster,
                Mode::Overwrite,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert_eq!(mapping.len(), 2, "重抽 1 名 + 保留 1 条：{mapping:?}");
        let kept = mapping
            .iter()
            .find(|m| m.uid == "r-7002")
            .expect("名单外旧映射应被带回");
        assert_eq!(
            (kept.ethnicity.as_str(), kept.image.as_str()),
            ("African", "e1"),
            "名单外那条必须逐字保留"
        );
        let redrawn = mapping.iter().find(|m| m.uid == "r-7001").unwrap();
        assert_eq!(redrawn.ethnicity, "SpanMed", "名单内那条按 ESP 重抽");
        // 日志如实报出「重抽 1 / 保留 1」——上层只看到总数，这两个数只有本模块知道。
        let logs: Vec<LogLine> = log_rx.try_iter().collect();
        assert!(
            logs.iter().any(|l| l
                .message
                .contains("名单 1 名中已重抽 1 名，名单外保留 1 条旧映射")),
            "覆写应如实报出重抽与保留条数：{logs:?}"
        );
    }

    #[test]
    fn cancel_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let mut mapper = FaceMapper::new(&img, &config(), &mut |_, _| {}).unwrap();
        let cancel = AtomicBool::new(true); // 已取消
        let logger = sink();
        let data = vec![player("r-2001", "ESP", "1")];
        assert!(mapper
            .generate_mapping(
                &data,
                Mode::Generate,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger
            )
            .is_none());
    }

    /// 标准一平铺包 fixture：根目录 5 张普通图 + 1 张点号文件名（锁定 file_stem 词干规则）。
    fn flatpack(dir: &tempfile::TempDir) -> String {
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("face{i}.png")), b"png").unwrap();
        }
        std::fs::write(dir.path().join("x.1.png"), b"png").unwrap();
        dir.path().to_string_lossy().to_string()
    }

    #[test]
    fn standard2_pack_maps_by_ethnic_and_keeps_dotted_stem() {
        // 主题：标准二映射。① 判定 + correct_ethnic 进种族目录 + 每条 from 能定位真实文件；
        // ② 含点号词干（x.1）按 file_stem 唯一口径写出，from 仍能定位到真实文件。
        let cancel = AtomicBool::new(false);
        let logger = sink();
        // ① facepack() 恰为 14 个合法种族目录 → 按内置 FolderList 判标准二且走种族匹配规则。
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let mut mapper = FaceMapper::new(&img, &config(), &mut |_, _| {}).unwrap();
        assert_eq!(
            mapper.layout,
            PackLayout::Standard {
                standard: 2,
                complete: true
            }
        );
        let data = vec![player("r-2001", "ESP", "1"), player("r-2002", "USA", "0")];
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, true, &cancel, &mut |_, _, _| {}, &logger)
            .unwrap();
        assert_eq!(mapping.len(), 2);
        assert_eq!(mapping[0].ethnicity, "SpanMed"); // 按 correct_ethnic 进种族目录
        assert_eq!(mapping[1].ethnicity, "Caucasian");
        for m in &mapping {
            assert!(
                xml_parser::resolve_image_file(Path::new(&img), &m.ethnicity, &m.image).is_some()
            );
        }
        // ② 另一份包：SpanMed 池只剩 x.1.png ⇒ from 必须是 "SpanMed/x.1"。
        let dir2 = tempfile::tempdir().unwrap();
        let img2 = facepack(&dir2);
        let spanmed = dir2.path().join("SpanMed");
        for i in 0..3 {
            std::fs::remove_file(spanmed.join(format!("e{i}.png"))).unwrap();
        }
        std::fs::write(spanmed.join("x.1.png"), b"png").unwrap();
        let mut mapper = FaceMapper::new(&img2, &config(), &mut |_, _| {}).unwrap();
        let mapping = mapper
            .generate_mapping(
                &[player("r-2001", "ESP", "1")],
                Mode::Generate,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert_eq!(
            (mapping[0].ethnicity.as_str(), mapping[0].image.as_str()),
            ("SpanMed", "x.1")
        );
        assert!(
            xml_parser::resolve_image_file(Path::new(&img2), "SpanMed", "x.1").is_some(),
            "from 应能定位到真实文件 SpanMed/x.1.png"
        );
    }

    #[test]
    fn flat_pack_covers_generate_exhaust_and_preserve() {
        // 主题：标准一平铺包。① Generate 忽略国籍/种族码；② 禁重复抽干后跳过并留日志；
        // ③ Preserve 保留旧平铺条目、只补新球员。
        let cancel = AtomicBool::new(false);
        let logger = sink();
        // ① Generate：未知国籍同样被映射，ethnicity 恒为空串，图名能在包根命中真实文件。
        let dir = tempfile::tempdir().unwrap();
        let img = flatpack(&dir);
        let mut mapper = FaceMapper::new(&img, &config(), &mut |_, _| {}).unwrap();
        assert_eq!(mapper.layout, PackLayout::Flat);
        let data = vec![player("r-3001", "ZZZ", "1"), player("r-3002", "ESP", "9")];
        let mapping = mapper
            .generate_mapping(
                &data,
                Mode::Generate,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert_eq!(mapping.len(), 2);
        for m in &mapping {
            assert_eq!(m.ethnicity, "");
            // 每条映射的图名都能在包根命中真实文件（file_stem 贯通写入/解析/定位）。
            assert!(xml_parser::resolve_image_file(Path::new(&img), "", &m.image).is_some());
        }
        // ② 禁重复：池共 6 张、8 名球员 → 只映射 6 条、全不重复；跳过 2 人留 WARNING 带样例。
        let dir2 = tempfile::tempdir().unwrap();
        let img2 = flatpack(&dir2);
        let mut mapper = FaceMapper::new(&img2, &config(), &mut |_, _| {}).unwrap();
        let data: Vec<PlayerRecord> = (0..8)
            .map(|i| player(&format!("r-40{i}"), "ESP", "1"))
            .collect();
        let (logger2, log_rx) = ChannelLogger::channel();
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, false, &cancel, &mut |_, _, _| {}, &logger2)
            .unwrap();
        assert_eq!(mapping.len(), 6); // 池耗尽后其余球员被跳过
        let images: BTreeSet<_> = mapping.iter().map(|m| m.image.clone()).collect();
        assert_eq!(images.len(), 6); // 全不重复
        // 【日志可见性】「谁没被抽到图」只有本模块知道：这两名球员在前台只会表现为「还是原脸」。
        let logs: Vec<LogLine> = log_rx.try_iter().collect();
        let skip = logs
            .iter()
            .find(|l| l.message.contains("没抽到图"))
            .unwrap_or_else(|| panic!("抽不到图的球员应留下一行日志：{logs:?}"));
        assert_eq!(skip.level, "WARNING");
        assert_eq!(skip.module, "facemap");
        assert!(skip.message.contains("2 名球员"), "{:?}", skip.message);
        // 样例必须带姓名与国籍/种族码，不是裸 UID。这里不钉"第几个人"——把 SKIP_LOG_MAX
        // 与样例取值顺序一起焊进断言，改日志上限就会红得和"日志功能坏了"分不清。
        assert!(
            skip.message.contains("T（ESP/1）"),
            "样例格式应为 UID 姓名（国籍/码）：{:?}",
            skip.message
        );
        // ③ Preserve：config.xml 已有平铺条目 from="face0"，保留旧值并只补新球员。
        let dir3 = tempfile::tempdir().unwrap();
        let img3 = flatpack(&dir3);
        crate::xml_parser::write_xml(
            &[MappingEntry {
                uid: "r-3001".into(),
                ethnicity: "".into(),
                image: "face0".into(),
            }],
            Path::new(&img3),
            false,
        )
        .unwrap();
        let mut mapper = FaceMapper::new(&img3, &config(), &mut |_, _| {}).unwrap();
        let data = vec![player("r-3001", "ESP", "1"), player("r-3002", "ESP", "1")];
        let mapping = mapper
            .generate_mapping(&data, Mode::Preserve, true, &cancel, &mut |_, _, _| {}, &logger)
            .unwrap();
        // r-3001 保留旧平铺映射；r-3002 新建且种族为空。
        let kept = mapping.iter().find(|m| m.uid == "r-3001").unwrap();
        assert_eq!((kept.ethnicity.as_str(), kept.image.as_str()), ("", "face0"));
        assert!(mapping
            .iter()
            .any(|m| m.uid == "r-3002" && m.ethnicity.is_empty()));
    }

    #[test]
    fn logs_report_fallback_stats_read_counts_and_mode_effects() {
        // 主题：日志可见性 —— 跑完全程后，只有本模块知道的几件事都要留下记录：
        // ① 抽图统计；② 快照读不出来时的 WARN；③ 正常读取条数与 Preserve 补人数；
        // ④ Generate 明确写出「现有映射不作判据、名单外条目会被丢弃」。
        let cancel = AtomicBool::new(false);
        let (logger, log_rx) = ChannelLogger::channel();
        // ① 标准二子集包只建 SpanMed（判 fixable）：NGA 码 3 的目标目录不可用 → 回落到全池取图。
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().to_string_lossy().to_string();
        let d = dir.path().join("SpanMed");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("s0.png"), b"png").unwrap();
        let mut mapper = FaceMapper::new(&img, &config(), &mut |_, _| {}).unwrap();
        let mapping = mapper
            .generate_mapping(
                &[player("r-1", "NGA", "3")],
                Mode::Generate,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert_eq!(mapping.len(), 1, "缺目录不再跳过，应回落出图：{mapping:?}");
        // ② 快照存在但读不出来（非 UTF-8）→ WARN + 按空表继续（球员仍被正常映射）。
        let dir2 = tempfile::tempdir().unwrap();
        let img2 = facepack(&dir2);
        let scan = crate::facepack_scanner::scan_pack(&img2).unwrap();
        let broken = dir2.path().join("broken.xml");
        std::fs::write(&broken, [0xff, 0xfe, 0x00]).unwrap();
        let mut mapper = FaceMapper::from_scan(&scan, &config(), Some(broken)).unwrap();
        let mapping = mapper
            .generate_mapping(
                &[player("r-2001", "ESP", "1")],
                Mode::Preserve,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert_eq!(mapping.len(), 1, "读出失败也应按空表继续：{mapping:?}");
        // ③ 正常快照 → INFO 记条数，Preserve 再报一次「已有映射多少、本次补多少」。
        let snap = dir2.path().join("group.xml");
        std::fs::write(&snap, config_xml_text(&[("r-2001", "SpanMed/e0")])).unwrap();
        let mut mapper = FaceMapper::from_scan(&scan, &config(), Some(snap.clone())).unwrap();
        let mapping = mapper
            .generate_mapping(
                &[player("r-2001", "ESP", "1")],
                Mode::Preserve,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert_eq!(
            (mapping[0].ethnicity.as_str(), mapping[0].image.as_str()),
            ("SpanMed", "e0")
        );
        // ④ Generate：同一份快照，但明确不作判据（只重抽名单内的，其余条目会被抛弃）。
        let mut mapper = FaceMapper::from_scan(&scan, &config(), Some(snap.clone())).unwrap();
        let mapping = mapper
            .generate_mapping(
                &[player("r-2001", "ESP", "1")],
                Mode::Generate,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert_eq!(mapping.len(), 1);

        // 汇总断言：四段的日志都落在 facemap 模块下。
        let logs: Vec<LogLine> = log_rx.try_iter().collect();
        assert!(logs.iter().all(|l| l.module == "facemap"), "{logs:?}");
        let find =
            |needle: &str, level: &str| logs.iter().any(|l| l.level == level && l.message.contains(needle));
        assert!(
            logs.iter().any(|l| l.message.contains("抽图统计")
                && l.message.contains("目标目录不可用、改从全池取图 1 名")),
            "回落统计应落日志：{logs:?}"
        );
        assert!(find("读取现有映射失败", "WARNING"), "损坏的快照必须留下 WARN：{logs:?}");
        assert!(find("现有映射：读取 1 条", "INFO"), "正常读取要记条数：{logs:?}");
        assert!(find("本次补 0 名", "INFO"), "Preserve 要报本次实际补的人数：{logs:?}");
        assert!(
            find("生成模式：名单 1 名全部重抽", "INFO"),
            "Generate 要报名单规模：{logs:?}"
        );
        assert!(
            find("不作判据且其名单外条目将被丢弃", "INFO"),
            "Generate 要写明会丢弃现有映射的名单外条目：{logs:?}"
        );
        // 反面：这几轮都没有球员被跳过，不该出现「没抽到图」的告警。
        assert!(!logs.iter().any(|l| l.message.contains("没抽到图")), "{logs:?}");
    }

    /// 手写一份 config.xml 文本：快照文件不在头像包目录里，用不了 `write_xml`（它只会写
    /// `<img_dir>/config.xml`，而本用例恰恰要让「快照」与「live」内容不同）。
    fn config_xml_text(rows: &[(&str, &str)]) -> String {
        let body: Vec<String> = rows
            .iter()
            .map(|(uid, from)| {
                format!(r#"<record from="{from}" to="graphics/pictures/person/{uid}/portrait"/>"#)
            })
            .collect();
        format!(
            "<root>\n                {}\n</root>\n",
            body.join("\n                ")
        )
    }

    #[test]
    fn preserve_and_overwrite_ignore_other_profiles_live_config_xml() {
        // 【串档回归】两档共用同一个头像包时，live config.xml 里可能残留**别档**的映射
        // （切档 restore 有三条静默跳过分支：该档在此目录无快照／目录不可用／copy 失败）。
        // 判据必须取自本档快照：否则会把别档的 r-8001 当成本档「已换过」——Preserve 跳过重抽、
        // Overwrite 把它原样带回，且错结果会被收尾同步写进本档快照，第二次替换起不可逆。
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let snap_dir = dir.path().join("snap");
        std::fs::create_dir_all(&snap_dir).unwrap();
        let snap = snap_dir.join("group.xml");
        // 本档快照：只有 r-7001（本档上次换脸的结果，图片钉死 e0 便于断言「原样保留」）。
        std::fs::write(&snap, config_xml_text(&[("r-7001", "SpanMed/e0")])).unwrap();
        // live（别档残留）：只有 r-8001，本档名单里没有它 → 绝不能出现在结果里。
        xml_parser::write_xml(
            &[MappingEntry {
                uid: "r-8001".into(),
                ethnicity: "SpanMed".into(),
                image: "e0".into(),
            }],
            Path::new(&img),
            false,
        )
        .unwrap();
        let roster = vec![player("r-7001", "ESP", "1")];
        let cancel = AtomicBool::new(false);
        let (logger, log_rx) = ChannelLogger::channel(); // 这一段要断言「带来了几条旧映射」的日志
                                                         // 每次生成映射都会改动规则内部状态（禁重复的池），所以每轮都重新扫包建 mapper。
        let scan_pack = || crate::facepack_scanner::scan_pack(&img).unwrap();

        for mode in [Mode::Preserve, Mode::Overwrite] {
            let scan = scan_pack();
            let mut mapper = FaceMapper::from_scan(&scan, &config(), Some(snap.clone())).unwrap();
            let mapping = mapper
                .generate_mapping(&roster, mode, true, &cancel, &mut |_, _, _| {}, &logger)
                .unwrap();
            assert!(
                !mapping.iter().any(|m| m.uid == "r-8001"),
                "别档残留的映射被带进了本档结果（mode={mode:?}）：{mapping:?}"
            );
            assert_eq!(
                mapping.len(),
                1,
                "mode={mode:?} 结果只应剩本档那一条：{mapping:?}"
            );
        }
        // 【日志口径 = 结果口径】覆写把自己重抽的个数与名单外保留的条数分开报：这里名单把快照里
        // 唯一的旧映射覆盖掉了，所以「名单外保留 0 条」——日志也必须这么写，否则说明判据又读串了档。
        let logs: Vec<LogLine> = log_rx.try_iter().collect();
        assert!(
            logs.iter().any(|l| l
                .message
                .contains("覆写模式：名单 1 名中已重抽 1 名，名单外保留 0 条旧映射")),
            "覆写应如实报出「重抽几人 / 保留几条」：{logs:?}"
        );
        // Preserve 的原样保留：快照里那条的 (种族, 图名) 必须逐字不变（Overwrite 会重抽，不查）。
        let scan = scan_pack();
        let mut mapper = FaceMapper::from_scan(&scan, &config(), Some(snap.clone())).unwrap();
        let kept = mapper
            .generate_mapping(
                &roster,
                Mode::Preserve,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert_eq!(
            (
                kept[0].uid.as_str(),
                kept[0].ethnicity.as_str(),
                kept[0].image.as_str()
            ),
            ("r-7001", "SpanMed", "e0")
        );
        // 反向确认：判据换成 live（修复前的口径）确实会带进 r-8001 —— 守着「这条断言不是靠巧合成立」。
        let scan = scan_pack();
        let live = Path::new(&img).join("config.xml");
        let mut mapper = FaceMapper::from_scan(&scan, &config(), Some(live)).unwrap();
        let wrong = mapper
            .generate_mapping(
                &roster,
                Mode::Preserve,
                true,
                &cancel,
                &mut |_, _, _| {},
                &logger,
            )
            .unwrap();
        assert!(
            wrong.iter().any(|m| m.uid == "r-8001"),
            "老口径（读 live）本应带进别档映射，说明本用例没有真正区分两个来源：{wrong:?}"
        );
    }
}
