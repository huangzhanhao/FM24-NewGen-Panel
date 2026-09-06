//! 【新手导读】抽图模板规则（按头像包模板独立成套）。
//! 头像包扫描出的布局（PackLayout）决定使用哪套抽图规则：
//!   模板一 Flat        → FlatRule      ：包根单一池，完全随机抽图（忽略国籍/种族码）；
//!   模板二 Standard2   → EthnicRule    ：国籍查表 → 种族码修正 → 池优先级 → 随机抽图（对等 v1）；
//!   模板三 Standard3   → （预留位，replace_flow 前置拦截，暂不可达；json 配置方案落地后在此实现）
//!   Empty / Unknown    → RecursiveRule ：把包内所有子/孙目录（含根）的全部图片归入一个大池，全随机。
//! 每套规则实现统一接口 PickRule（pick 抽图 + exclude_used 禁重复剔除），
//! 后续「模板二/三规则 json 固化、用户可配置」即按此接口参数化，不影响调用方。
use crate::facepack::{PackLayout, PackScan};
use crate::rtf_parser::PlayerRecord;
use crate::xml_parser::record_from;
use rand::Rng;
use std::collections::{BTreeMap, BTreeSet};

/// 一次抽图的结果：图片相对头像包根的目录路径（"" = 根，多级 "子目录/孙目录"）+ 文件词干。
/// 写 config.xml 时 dir + stem 经 record_from 拼成 from（统一相对路径口径）。
pub struct PickedImage { pub dir: String, pub stem: String }

/// 抽图模板规则统一接口：每种头像包模板一套实现。
pub trait PickRule {
    /// Preserve+禁重复：把已有映射占用的图片从池中剔除。
    /// used 为「from 全路径」键集合（record_from 形式：种族目录非空 → "目录/词干"，平铺 → "词干"）。
    fn exclude_used(&mut self, used: &BTreeSet<String>);
    /// 为一名球员抽图；返回 None 表示该球员无法映射（跳过）。
    /// eth_cfg 为国籍 → 种族目录映射表（仅模板二规则使用，其余规则忽略）。
    fn pick(
        &mut self,
        player: &PlayerRecord,
        duplicates: bool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<PickedImage>;
}

/// 按扫描出的布局选择对应模板规则（唯一分发入口）。
pub fn for_layout(layout: PackLayout, scan: &PackScan) -> Box<dyn PickRule> {
    match layout {
        // 模板一：包根单一池。
        PackLayout::Flat => Box::new(FlatRule { pool: scan.root_images.clone() }),
        // 模板二：种族匹配。（模板三同样落到这里属防御性兜底——replace_flow 已在前置拦截，
        // 正常流程不可达；其 json 规则落地后替换为 Standard3Rule。）
        PackLayout::Standard2 { .. } | PackLayout::Standard3 { .. } =>
            Box::new(EthnicRule { pools: scan.faces_map.clone() }),
        // 空包 / 无法识别：递归全池全随机（Empty 池为空 → 全部球员跳过，与现状等价）。
        PackLayout::Empty | PackLayout::Unknown =>
            Box::new(RecursiveRule { pools: scan.recursive_pools.clone() }),
    }
}

// ---------------- 模板一：平铺全随机 ----------------

/// 包根全部图片为一个单一池，完全随机抽图。
pub struct FlatRule { pool: BTreeSet<String> }

impl PickRule for FlatRule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        // 平铺条目的 from 全路径就是纯词干，直接比对。
        self.pool.retain(|s| !used.contains(s));
    }

    fn pick(
        &mut self,
        _player: &PlayerRecord,
        duplicates: bool,
        _eth_cfg: &BTreeMap<String, String>,
    ) -> Option<PickedImage> {
        if self.pool.is_empty() { return None; }
        let idx = rand::rng().random_range(0..self.pool.len());
        let stem = self.pool.iter().nth(idx).cloned()?;
        if !duplicates {
            self.pool.remove(&stem); // 禁重复：选过的图不再出现
        }
        Some(PickedImage { dir: String::new(), stem })
    }
}

// ---------------- 模板二：种族匹配（对等 v1，逻辑原样迁移） ----------------

/// 一级种族/国籍目录池 + FM 种族码修正规则。
pub struct EthnicRule { pools: BTreeMap<String, BTreeSet<String>> }

impl EthnicRule {
    /// FM 种族码（0-10）在两个候选种族间修正归类（对等旧版 correct_ethnic，逐分支一致）。
    /// e1=主国籍种族，e2=第二国籍种族（可选）。hit 判断某个种族名是否命中两个候选之一。
    pub fn correct_ethnic(code: &str, e1: &str, e2: Option<&str>) -> String {
        let hit = |name: &str| e1 == name || e2 == Some(name);
        match code {
            "0" => {
                // 优先 Scandinavian，其次 Caucasian，都不命中则兑底 Central European。
                if hit("Scandinavian") { "Scandinavian".into() }
                else if hit("Caucasian") { "Caucasian".into() }
                else { "Central European".into() }
            }
            "1" => ["Caucasian", "EECA", "Italmed", "SAMed", "SpanMed", "YugoGreek", "South American"]
                .iter()
                .find(|n| hit(n))            // 找第一个命中的候选
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Central European".into()), // 都不命中则兑底
            "2" => if hit("MESA") { "MESA".into() } else { "MENA".into() },
            "3" | "9" => if hit("South American") { "South American".into() } else { "African".into() },
            "6" => if hit("SAMed") { "SAMed".into() } else { "African".into() },
            "8" => if hit("Seasian") { "Seasian".into() } else { "African".into() },
            "4" => "MESA".into(),            // 固定值，不看国籍
            "5" => "Seasian".into(),
            "7" => "South American".into(),
            "10" => {
                // 链式兑底：MESA → Seasian → South American → Asian。
                if hit("MESA") { "MESA".into() }
                else if hit("Seasian") { "Seasian".into() }
                else if hit("South American") { "South American".into() }
                else { "Asian".into() }
            }
            _ => e1.to_string(), // 解析层已保证 0-10，防御性兑底
        }
    }

    /// 池优先级：国籍同名目录 > 种族目录 > 两者都不在包内时用全部目录。
    fn image_pool_dirs(&self, ethnicity: &str, nationality: &str) -> Vec<String> {
        let mut pools = Vec::new();
        let nat_in = self.pools.get(nationality).map(|s| !s.is_empty()).unwrap_or(false);
        let eth_in = self.pools.get(ethnicity).map(|s| !s.is_empty()).unwrap_or(false);
        if nat_in { pools.push(nationality.to_string()); }
        if eth_in { pools.push(ethnicity.to_string()); }
        if !self.pools.contains_key(nationality) && !self.pools.contains_key(ethnicity) {
            pools.extend(self.pools.keys().cloned());
        }
        pools
    }

    /// 依序取第一个非空池随机选图；禁重复时从池中移除（对等旧版 pick_image_from_pools）。
    fn pick_image_from_pools(&mut self, pools: &[String], duplicates: bool) -> Option<String> {
        for dir in pools {
            let Some(pool) = self.pools.get_mut(dir) else { continue };
            if pool.is_empty() { continue; }
            let idx = rand::rng().random_range(0..pool.len());
            if let Some(choice) = pool.iter().nth(idx).cloned() {
                if !duplicates { pool.remove(&choice); }
                return Some(choice);
            }
        }
        None
    }
}

impl PickRule for EthnicRule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        // 按全路径精确匹配：只剔除「本目录/词干」确实被占用的图，
        // 不再像旧口径那样跨目录误伤同名图（不同种族目录存在同名文件时）。
        for (dir, pool) in self.pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
    }

    fn pick(
        &mut self,
        player: &PlayerRecord,
        duplicates: bool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<PickedImage> {
        // 双国籍查表 → 种族码修正 → 校验目录在包内 → 选图（对等 v1 _build_player_mapping）。
        let e2 = if player.sec_nat.is_empty() {
            None
        } else {
            eth_cfg.get(&player.sec_nat).map(|s| s.as_str())
        };
        let e1 = eth_cfg.get(&player.primary_nat)?;
        let ethnicity = Self::correct_ethnic(&player.ethnicity_code, e1, e2);
        if !self.pools.contains_key(&ethnicity) {
            return None; // 头像包缺该种族目录
        }
        let dirs = self.image_pool_dirs(&ethnicity, &player.primary_nat);
        let stem = self.pick_image_from_pools(&dirs, duplicates)?;
        Some(PickedImage { dir: ethnicity, stem })
    }
}

// ---------------- Empty / Unknown：递归全池全随机 ----------------

/// 包内所有子/孙目录（含根）的全部图片归入一个大池，完全随机抽图。
/// 选中的 dir 原样写入映射（可空、可多级），config.xml 的 from 即相对路径。
pub struct RecursiveRule { pools: BTreeMap<String, BTreeSet<String>> }

impl PickRule for RecursiveRule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        for (dir, pool) in self.pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
    }

    fn pick(
        &mut self,
        _player: &PlayerRecord,
        duplicates: bool,
        _eth_cfg: &BTreeMap<String, String>,
    ) -> Option<PickedImage> {
        // 展平所有 (目录, 词干) 计数后按全局下标随机取（有序集合保证可复现）。
        let total: usize = self.pools.values().map(|s| s.len()).sum();
        if total == 0 { return None; }
        let idx = rand::rng().random_range(0..total);
        let mut passed = 0usize;
        let mut chosen: Option<(String, String)> = None;
        for (dir, set) in &self.pools {
            if idx < passed + set.len() {
                let stem = set.iter().nth(idx - passed).cloned().unwrap_or_default();
                chosen = Some((dir.clone(), stem));
                break;
            }
            passed += set.len();
        }
        let (dir, stem) = chosen?;
        if !duplicates {
            if let Some(set) = self.pools.get_mut(&dir) { set.remove(&stem); }
        }
        Some(PickedImage { dir, stem })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtf_parser::PlayerRecord;

    fn player(uid: &str, nat: &str, code: &str) -> PlayerRecord {
        PlayerRecord {
            uid: uid.into(), primary_nat: nat.into(), sec_nat: String::new(),
            name: "T".into(), hair_length: "1".into(), hair_color: "1".into(),
            ethnicity_code: code.into(), detailed: None,
        }
    }

    fn sset(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn flat_rule_picks_from_root_and_excludes_used() {
        let mut rule = FlatRule { pool: sset(&["a", "b", "c"]) };
        let eth = BTreeMap::new();
        // 全随机：三次抽取都应落在池内，且 dir 为空串。
        for _ in 0..10 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), true, &eth).unwrap();
            assert!(img.dir.is_empty());
            assert!(["a", "b", "c"].contains(&img.stem.as_str()));
        }
        // 禁重复剔除：b 被占用后不再被抽到。
        rule.exclude_used(&sset(&["b"]));
        for _ in 0..10 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), true, &eth).unwrap();
            assert_ne!(img.stem, "b");
        }
    }

    #[test]
    fn recursive_rule_picks_everything_and_excludes_by_full_path() {
        // 池：根 {r1}、A {x1, x2}、A/sub {d1} —— 共 4 张。
        let mut rule = RecursiveRule { pools: BTreeMap::from([
            (String::new(), sset(&["r1"])),
            ("A".into(), sset(&["x1", "x2"])),
            ("A/sub".into(), sset(&["d1"])),
        ]) };
        let eth = BTreeMap::new();
        // 禁重复抽满 4 次（池恰 4 张）→ 确定性断言：根、子、孙三个层级的图片全部可达。
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for _ in 0..4 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), false, &eth).unwrap();
            seen.insert(record_from(&img.dir, &img.stem));
        }
        assert_eq!(seen, sset(&["r1", "A/x1", "A/x2", "A/sub/d1"]));
        // 池已耗尽 → 再抽返回 None。
        assert!(rule.pick(&player("r-1", "ZZZ", "1"), false, &eth).is_none());
        // 禁重复剔除按全路径：A/x1 与孙目录 d1 被占用后不再出现，其余仍在（重置池后验证）。
        let mut rule = RecursiveRule { pools: BTreeMap::from([
            (String::new(), sset(&["r1"])),
            ("A".into(), sset(&["x1", "x2"])),
            ("A/sub".into(), sset(&["d1"])),
        ]) };
        rule.exclude_used(&sset(&["A/x1", "A/sub/d1"]));
        assert_eq!(rule.pools["A"], sset(&["x2"]));
        assert!(rule.pools["A/sub"].is_empty());
        assert_eq!(rule.pools.get("").unwrap(), &sset(&["r1"]));
    }

    #[test]
    fn ethnic_rule_nationality_dir_priority_and_correct_ethnic() {
        // 国籍同名目录优先于种族目录（对等 image_pool_dirs 优先级）。
        let mut rule = EthnicRule { pools: BTreeMap::from([
            ("ESP".into(), sset(&["e1"])),
            ("SpanMed".into(), sset(&["s1"])),
        ]) };
        let eth = BTreeMap::from([("ESP".to_string(), "SpanMed".to_string())]);
        for _ in 0..10 {
            let img = rule.pick(&player("r-1", "ESP", "1"), true, &eth).unwrap();
            assert_eq!(img.stem, "e1"); // 国籍目录命中，永远先于种族目录
        }
        // 种族码修正规则抽查（完整 0-10 用例见 correct_ethnic_all_codes）。
        assert_eq!(EthnicRule::correct_ethnic("2", "MESA", None), "MESA");
        assert_eq!(EthnicRule::correct_ethnic("2", "MENA", None), "MENA");
    }

    #[test]
    fn correct_ethnic_all_codes() {
        // 0：优先 Scandinavian > Caucasian > 兜底
        assert_eq!(EthnicRule::correct_ethnic("0", "African", Some("Scandinavian")), "Scandinavian");
        assert_eq!(EthnicRule::correct_ethnic("0", "African", Some("Caucasian")), "Caucasian");
        assert_eq!(EthnicRule::correct_ethnic("0", "African", None), "Central European");
        // 1：命中列表则用，否则兜底
        assert_eq!(EthnicRule::correct_ethnic("1", "EECA", None), "EECA");
        assert_eq!(EthnicRule::correct_ethnic("1", "MENA", None), "Central European");
        // 2：MESA 命中否则 MENA
        assert_eq!(EthnicRule::correct_ethnic("2", "MESA", None), "MESA");
        assert_eq!(EthnicRule::correct_ethnic("2", "MENA", None), "MENA");
        // 3/6/8/9：仅对应码命中，否则 African
        assert_eq!(EthnicRule::correct_ethnic("6", "SAMed", None), "SAMed");
        assert_eq!(EthnicRule::correct_ethnic("8", "Seasian", None), "Seasian");
        assert_eq!(EthnicRule::correct_ethnic("3", "South American", None), "South American");
        assert_eq!(EthnicRule::correct_ethnic("9", "South American", None), "South American");
        assert_eq!(EthnicRule::correct_ethnic("3", "SAMed", None), "African");
        // 固定值
        assert_eq!(EthnicRule::correct_ethnic("4", "African", None), "MESA");
        assert_eq!(EthnicRule::correct_ethnic("5", "African", None), "Seasian");
        assert_eq!(EthnicRule::correct_ethnic("7", "African", None), "South American");
        // 10：链式兜底到 Asian
        assert_eq!(EthnicRule::correct_ethnic("10", "MESA", None), "MESA");
        assert_eq!(EthnicRule::correct_ethnic("10", "African", Some("Seasian")), "Seasian");
        assert_eq!(EthnicRule::correct_ethnic("10", "Asian", None), "Asian");
    }
}
