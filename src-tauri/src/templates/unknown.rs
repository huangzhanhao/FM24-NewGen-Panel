//! 退化布局：空包（Empty）与无法识别的包（Unknown）。
//! 这两种布局**不是标准**——它们没有目录组织约定可循，也没有 json 规则文件；
//! 抽图逻辑 = 把包内所有子/孙目录（含根）的图片归入一个大池，完全随机抽。
//! 对外的标准号映射（Empty → 1、Unknown → 0）只在 `PackLayout::standard()` 一处。
use super::{pick_from_pools_flat, PickedImage, PickRule};
use crate::rtf_scanner::PlayerRecord;
use crate::xml_parser::record_from;
use std::collections::{BTreeMap, BTreeSet};

/// 包内所有子/孙目录（含根）的全部图片归入一个大池，完全随机抽图。
/// 选中的 dir 原样写入映射（可空、可多级），config.xml 的 from 即相对路径。
pub struct UnknownRule { pools: BTreeMap<String, BTreeSet<String>> }

impl UnknownRule {
    /// 用递归全池（`scan.recursive_pools`，含根与全部子/孙目录）构造；`pick` 把全池展平后随机取图。
    pub fn new(pools: BTreeMap<String, BTreeSet<String>>) -> Self { Self { pools } }
}

impl PickRule for UnknownRule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        for (dir, pool) in self.pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
    }

    fn pick(&mut self, _player: &PlayerRecord, duplicates: bool) -> Option<PickedImage> {
        pick_from_pools_flat(&mut self.pools, duplicates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtf_scanner::PlayerRecord;

    fn player(uid: &str, nat: &str, code: &str) -> PlayerRecord {
        PlayerRecord {
            uid: uid.into(), primary_nat: nat.into(), sec_nat: String::new(),
            name: "T".into(), hair_length: "1".into(), hair_color: "1".into(),
            ethnicity_code: code.into(), detailed: None,
        }
    }

    fn sset(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn unknown_picks_everything_and_excludes_by_full_path() {
        // 池：根 {r1}、A {x1, x2}、A/sub {d1} —— 共 4 张。
        let mut rule = UnknownRule::new(BTreeMap::from([
            (String::new(), sset(&["r1"])),
            ("A".into(), sset(&["x1", "x2"])),
            ("A/sub".into(), sset(&["d1"])),
        ]));
        // 禁重复抽满 4 次（池恰 4 张）→ 确定性断言：根、子、孙三个层级的图片全部可达。
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for _ in 0..4 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), false).unwrap();
            seen.insert(record_from(&img.dir, &img.stem));
        }
        assert_eq!(seen, sset(&["r1", "A/x1", "A/x2", "A/sub/d1"]));
        assert!(rule.pick(&player("r-1", "ZZZ", "1"), false).is_none()); // 池耗尽
        // 禁重复剔除按全路径：A/x1 与孙目录 d1 被占用后不再出现，其余仍在（重置池后验证）。
        let mut rule = UnknownRule::new(BTreeMap::from([
            (String::new(), sset(&["r1"])),
            ("A".into(), sset(&["x1", "x2"])),
            ("A/sub".into(), sset(&["d1"])),
        ]));
        rule.exclude_used(&sset(&["A/x1", "A/sub/d1"]));
        assert_eq!(rule.pools["A"], sset(&["x2"]));
        assert!(rule.pools["A/sub"].is_empty());
        assert_eq!(rule.pools.get("").unwrap(), &sset(&["r1"]));
    }

    /// 空包（Empty）走的是同一套规则：池为空 → 每个球员都跳过。
    #[test]
    fn unknown_empty_pack_skips_every_player() {
        let mut rule = UnknownRule::new(BTreeMap::from([(String::new(), BTreeSet::new())]));
        assert!(rule.pick(&player("r-1", "ESP", "1"), true).is_none());
    }
}
