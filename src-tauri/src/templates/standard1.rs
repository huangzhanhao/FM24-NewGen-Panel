//! 标准一：图片平铺在头像包根目录，没有种族/国籍子目录。
//! 抽图逻辑 = 完全随机（忽略球员的国籍与种族码），因为包本身不含任何分类信息。
//! 【为什么这个标准没有 json 规则文件】它没有任何可配数据：布局判定靠"无子目录 + 根目录有图"，
//! 抽图靠单一根池。所以 `RuleConfig` 里没有它的字段，`folder_lists()` 里也没有它，
//! `IMPLEMENTED_STANDARDS` 的拦截同样不经过它（那个拦截只针对 `PackLayout::Standard`）。
use super::{PickedImage, PickRule};
use crate::rtf_scanner::PlayerRecord;
use rand::Rng;
use std::collections::BTreeSet;

/// 包根全部图片为一个单一池，完全随机抽图。
pub struct Standard1Rule { pool: BTreeSet<String> }

impl Standard1Rule {
    /// 用包根图片池（`scan.root_images`）构造；`pick` 忽略球员的国籍与种族码，纯粹从池中随机取图。
    pub fn new(pool: BTreeSet<String>) -> Self { Self { pool } }
}

impl PickRule for Standard1Rule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        // 平铺条目的 from 全路径就是纯词干，直接比对。
        self.pool.retain(|s| !used.contains(s));
    }

    fn pick(&mut self, _player: &PlayerRecord, duplicates: bool) -> Option<PickedImage> {
        if self.pool.is_empty() { return None; }
        let idx = rand::rng().random_range(0..self.pool.len());
        let stem = self.pool.iter().nth(idx).cloned()?;
        if !duplicates {
            self.pool.remove(&stem); // 禁重复：选过的图不再出现
        }
        Some(PickedImage { dir: String::new(), stem })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(uid: &str, nat: &str, code: &str) -> PlayerRecord {
        PlayerRecord {
            uid: uid.into(), primary_nat: nat.into(), sec_nat: String::new(),
            name: "T".into(), hair_length: "1".into(), hair_color: "1".into(),
            ethnicity_code: code.into(), detailed: None,
        }
    }

    fn sset(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn standard1_picks_from_root_and_excludes_used() {
        let mut rule = Standard1Rule::new(sset(&["a", "b", "c"]));
        // 全随机：多次抽取都应落在池内，且 dir 为空串。
        for _ in 0..10 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), true).unwrap();
            assert!(img.dir.is_empty());
            assert!(["a", "b", "c"].contains(&img.stem.as_str()));
        }
        // 禁重复剔除：b 被占用后不再被抽到。
        rule.exclude_used(&sset(&["b"]));
        for _ in 0..10 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), true).unwrap();
            assert_ne!(img.stem, "b");
        }
    }

    #[test]
    fn standard1_empty_pool_skips_every_player() {
        let mut rule = Standard1Rule::new(BTreeSet::new());
        assert!(rule.pick(&player("r-1", "ESP", "1"), true).is_none());
        assert!(rule.pick(&player("r-1", "ESP", "1"), false).is_none());
    }
}
