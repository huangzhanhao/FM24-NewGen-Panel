//! 【新手导读】Profile（存档）/ 组管理：JSON 原子读写、切换时快照交换、旧版单档格式自动升级。
//! 自旧版 core/ProfileManager.py 移植；groups 用 BTreeMap 保证稳定顺序。
//!
//! 【两个核心概念】
//!   Profile（存档）：一套独立的配置集合，对应一个 FM 存档；不同存档的换脸互不干扰。
//!   组（group）：一个头像包目录。一个存档下可关联多个头像包，每个组记住自己的 RTF 名单。
//! 【快照机制】切换存档时，先把当前存档各组的 config.xml 备份到 .user/<存档>/，
//!   再把目标存档的快照恢复回头像包目录——这样不同存档共享同一头像包目录也不会互相覆盖。
use crate::json_store::{self, LoadError};
use crate::paths::{group_xml_path, norm_dir};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap; // 有序字典：保证组/存档的遍历顺序稳定
use std::fs;
use std::path::{Path, PathBuf};

/// 单个组的配置。目前只存 RTF 路径（#[serde(default)] 表示旧文件缺这个字段时用默认值）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GroupCfg {
    #[serde(default)]
    pub rtf: String,
}

/// 组集合：键 = 归一化后的头像包目录，值 = 该组配置。用类型别名简化书写。
pub type GroupsMap = BTreeMap<String, GroupCfg>;

/// 单个存档的配置（对应 .user/<存档名>.json）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PrfCfg {
    #[serde(default)]
    pub cur_group: Option<String>, // 当前选中的组
    #[serde(default)]
    pub groups: GroupsMap,         // 所有组
}

/// 全局配置（对应 .user/cfg.json）。字段名 Profile 在 JSON 里首字母大写，用 rename 对应。
/// value 为 bool 表示该存档是否「当前激活」（全局只有一个为 true）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlobalCfg {
    #[serde(rename = "Profile")]
    pub profile: BTreeMap<String, bool>,
}

/// 存档管理器：整个应用围绕它管理所有存档与组的配置。
#[derive(Debug)]
pub struct ProfileManager {
    user_dir: PathBuf,   // <data>/.user 目录
    pub config: GlobalCfg,       // 全局配置（哪些存档、哪个激活）
    pub cur_prf: String,         // 当前激活存档名
    pub prf_cfg: PrfCfg,         // 当前存档的配置（组、当前组）
    pub cur_group: Option<String>, // 当前选中的组
}

impl ProfileManager {
    /// 创建管理器：读全局 cfg.json（缺失则初始化一个只含 'No Profile' 的默认配置），并加载当前存档。
    /// name=None 时自动回退到 cfg.json 中标记为激活的存档。
    pub fn new(name: Option<&str>, data_dir: &Path) -> Result<Self, String> {
        let user_dir = data_dir.join(".user");
        fs::create_dir_all(&user_dir).map_err(|e| e.to_string())?;
        let cfg_path = user_dir.join("cfg.json");
        // 加载全局配置：成功则用；缺失则写一份默认；损坏则报错（json_store 已自动备份 .corrupt）。
        let mut config: GlobalCfg = match json_store::load(&cfg_path) {
            Ok(c) => c,
            Err(LoadError::Missing) => {
                // 首次运行：创建一个只含 'No Profile'（激活）的默认配置并落盘。
                let c = GlobalCfg {
                    profile: [("No Profile".to_string(), true)].into_iter().collect(),
                };
                json_store::save(&cfg_path, &c)?;
                c
            }
            Err(LoadError::Corrupt(e)) => return Err(e),
        };
        // 对等 v1：name 缺失/无效时回退到 cfg.json 中激活的档（get_latest_prf 语义）。
        // 【防御】历史上新建存档的落盘时序缺陷可能留下多个激活位（如 No Profile 与新档同时 true），
        // 此时优先选「非 No Profile」的激活档（用户创建的档优先于默认档），避免重启回到默认档。
        let cur_prf = name
            .filter(|n| config.profile.contains_key(*n))
            .map(|n| n.to_string())
            .or_else(|| {
                let actives: Vec<&String> = config
                    .profile
                    .iter()
                    .filter(|(_, v)| **v)
                    .map(|(k, _)| k)
                    .collect();
                match actives.len() {
                    1 => Some((*actives[0]).clone()),
                    2.. => actives
                        .iter()
                        .find(|k| k.as_str() != "No Profile")
                        .or_else(|| actives.first())
                        .map(|k| (*k).clone()),
                    _ => None,
                }
            })
            .unwrap_or_else(|| "No Profile".to_string());
        // 激活位归一化（修复历史双 true 脏数据）：只保留当前档为激活，有变化才落盘。
        let mut dirty = false;
        for (k, v) in config.profile.iter_mut() {
            let target = *k == cur_prf;
            if *v != target {
                *v = target;
                dirty = true;
            }
        }
        if dirty {
            json_store::save(&cfg_path, &config)?;
        }
        // 加载当前存档的组配置，并选定当前组。
        let prf_cfg = Self::load_profile_cfg(&user_dir, &cur_prf)?;
        let cur_group = Self::pick_cur_group(&prf_cfg);
        Ok(Self { user_dir, config, cur_prf, prf_cfg, cur_group })
    }

    /// 选定当前组：优先用配置里记录的 cur_group（且它仍存在），否则回退到第一个组。
    fn pick_cur_group(cfg: &PrfCfg) -> Option<String> {
        cfg.cur_group
            .clone()
            .filter(|g| cfg.groups.contains_key(g)) // cur_group 必须仍在组列表里
            .or_else(|| cfg.groups.keys().next().cloned()) // 否则取第一个
    }

    /// 存档配置文件路径：.user/<存档名>.json。
    fn prf_path(user_dir: &Path, name: &str) -> PathBuf {
        user_dir.join(format!("{name}.json"))
    }

    /// 加载档配置；缺失兑底空配置；旧版单档格式自动升级（含快照迁移）。
    /// 【关键】这里处理了从旧版（只有一个 img_dir/rtf）到新版（多组 groups）的数据升级。
    fn load_profile_cfg(user_dir: &Path, name: &str) -> Result<PrfCfg, String> {
        let path = Self::prf_path(user_dir, name);
        // 先读成通用 JSON 值（serde_json::Value），以便先判断是新格式还是旧格式。
        let raw = match json_store::load::<serde_json::Value>(&path) {
            Ok(v) => v,
            Err(LoadError::Missing) => {
                // 文件不存在：写一份空配置并返回。
                let cfg = PrfCfg::default();
                json_store::save(&path, &cfg)?;
                return Ok(cfg);
            }
            Err(LoadError::Corrupt(e)) => return Err(e),
        };
        // 旧版单档格式：{"img_dir": "X", "rtf": "Y"} 且无 groups → 升级为新的多组格式。
        if raw.get("img_dir").is_some() && raw.get("groups").is_none() {
            let img_dir = norm_dir(raw["img_dir"].as_str().unwrap_or(""));
            let rtf = raw["rtf"].as_str().unwrap_or("").to_string();
            let mut groups = GroupsMap::new();
            if !img_dir.is_empty() {
                groups.insert(img_dir.clone(), GroupCfg { rtf });
            }
            let cfg = PrfCfg {
                cur_group: (!img_dir.is_empty()).then_some(img_dir), // then_some：条件为真时包 Some
                groups,
            };
            json_store::save(&path, &cfg)?; // 把升级后的新格式写回
            // 旧快照 .user/<name>.xml → .user/<name>/<sanitized>.xml（搬到新的目录结构）。
            if let Some(img) = &cfg.cur_group {
                let old_snap = user_dir.join(format!("{name}.xml"));
                if old_snap.is_file() {
                    let new_snap = group_xml_path(user_dir, name, img);
                    if let Some(p) = new_snap.parent() {
                        fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                    fs::rename(&old_snap, &new_snap).map_err(|e| e.to_string())?;
                }
            }
            return Ok(cfg);
        }
        // 新格式：直接从 JSON 值反序列化为 PrfCfg。
        serde_json::from_value(raw).map_err(|e| e.to_string())
    }

    // ---------------- Profile 管理 ----------------
    /// 新建档：建好空档文件并立即切档（v1 交互：新建即激活）。
    pub fn create_profile(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Profile 名不能为空".into());
        }
        // 重名保护：已存在则报错，避免覆写别人的存档。
        if self.config.profile.contains_key(name) {
            return Err(format!("Profile「{name}」已存在"));
        }
        self.config.profile.insert(name.to_string(), true); // 加入全局配置
        json_store::save(&self.user_dir.join("cfg.json"), &self.config)?; // 落盘全局配置
        json_store::save(&Self::prf_path(&self.user_dir, name), &PrfCfg::default())?; // 建空档文件
        self.load_profile(name) // 立即切到新档
    }

    /// 删除档。'No Profile' 不可删（返回 false）；删后自动切回 'No Profile'。
    pub fn delete_profile(&mut self, name: &str) -> Result<bool, String> {
        if name == "No Profile" {
            return Ok(false);
        }
        self.config.profile.remove(name); // 从全局配置移除
        let _ = fs::remove_file(Self::prf_path(&self.user_dir, name)); // 删档配置文件
        let _ = fs::remove_dir_all(self.user_dir.join(crate::paths::sanitize(name))); // 删快照目录
        json_store::save(&self.user_dir.join("cfg.json"), &self.config)?;
        self.load_profile("No Profile")?; // 回到默认档
        Ok(true)
    }

    /// 切换档：保存旧档各组快照 → 加载新档 → 恢复新档各组快照 → 更新激活位。
    /// 【核心】这就是「快照交换」：离开时存下、进入时取回，保证每个档看到自己的 config.xml。
    pub fn load_profile(&mut self, name: &str) -> Result<(), String> {
        let old = self.cur_prf.clone();
        // 1) 把旧档每个组的 config.xml 备份到快照目录。
        //（先 collect 成 Vec 再遍历，避免遍历中修改 groups 导致借用冲突）
        for img_dir in self.prf_cfg.groups.keys().cloned().collect::<Vec<_>>() {
            self.save_group_xml(&old, &img_dir);
        }
        // 2) 加载新档的组配置。
        self.prf_cfg = Self::load_profile_cfg(&self.user_dir, name)?;
        // 3) 把新档每个组的快照恢复回头像包目录。
        for img_dir in self.prf_cfg.groups.keys().cloned().collect::<Vec<_>>() {
            self.restore_group_xml(name, &img_dir);
        }
        // 4) 更新全局激活位：只有目标档为 true，其余置 false；并落盘 cfg.json
        //    【修复】此前激活位变更不落盘，新建存档后重启会回到 No Profile。
        for k in self.config.profile.keys().cloned().collect::<Vec<_>>() {
            if let Some(flag) = self.config.profile.get_mut(&k) {
                *flag = k == name;
            }
        }
        json_store::save(&self.user_dir.join("cfg.json"), &self.config)?;
        self.cur_prf = name.to_string();
        self.cur_group = Self::pick_cur_group(&self.prf_cfg);
        Ok(())
    }

    /// 把头像包目录里的 config.xml 拷到该档的快照位置（切走时调用）。
    fn save_group_xml(&self, profile: &str, img_dir: &str) {
        let src = Path::new(img_dir).join("config.xml");
        if !src.is_file() {
            return; // 目录里没有 config.xml（还没换过脸），无需备份
        }
        let dst = group_xml_path(&self.user_dir, profile, img_dir);
        if let Some(p) = dst.parent() {
            let _ = fs::create_dir_all(p); // 确保快照子目录存在
        }
        let _ = fs::copy(&src, &dst);
    }

    /// 把快照恢复回头像包目录的 config.xml（切入时调用）。
    fn restore_group_xml(&self, profile: &str, img_dir: &str) {
        let src = group_xml_path(&self.user_dir, profile, img_dir);
        let dst = Path::new(img_dir).join("config.xml");
        // 头像包目录已不存在（换盘/被删）：保留快照，跳过恢复（对等 v1）。
        if src.is_file() && Path::new(img_dir).is_dir() {
            let _ = fs::copy(&src, &dst);
        }
    }

    // ---------------- 组管理 ----------------
    /// 新头像包目录自动建组并设为当前组（对等旧版 ensure_group）。返回归一化后的目录名。
    pub fn ensure_group(&mut self, img_dir: &str) -> Option<String> {
        let dir = norm_dir(img_dir); // 先归一化，保证同一目录只建一个组
        if dir.is_empty() {
            return None;
        }
        // 组不存在就新建一个空配置；已存在则不动（保留原有 RTF）。
        if !self.prf_cfg.groups.contains_key(&dir) {
            self.prf_cfg.groups.insert(dir.clone(), GroupCfg::default());
        }
        self.set_cur_group(&dir).ok()?; // 设为当前组；.ok()? 把失败转为 None
        Some(dir)
    }

    /// 切换当前组（必须已存在），并落盘。
    pub fn set_cur_group(&mut self, img_dir: &str) -> Result<bool, String> {
        let dir = norm_dir(img_dir);
        if !self.prf_cfg.groups.contains_key(&dir) {
            return Ok(false); // 组不存在
        }
        self.cur_group = Some(dir.clone());
        self.prf_cfg.cur_group = Some(dir);
        self.persist_prf()?; // 把当前档配置写回文件
        Ok(true)
    }

    /// 删除组（连同其快照）；若删的是当前组，自动回退到剩下的组。
    pub fn delete_group(&mut self, img_dir: &str) -> Result<bool, String> {
        let dir = norm_dir(img_dir);
        if self.prf_cfg.groups.remove(&dir).is_none() {
            return Ok(false); // 组不存在
        }
        let snap = group_xml_path(&self.user_dir, &self.cur_prf, &dir);
        if snap.is_file() {
            let _ = fs::remove_file(&snap); // 删掉快照文件
        }
        // 若删的正好是当前组，重新选一个当前组。
        if self.cur_group.as_deref() == Some(dir.as_str()) {
            self.cur_group = Self::pick_cur_group(&self.prf_cfg);
            self.prf_cfg.cur_group = self.cur_group.clone();
        }
        self.persist_prf()?;
        Ok(true)
    }

    /// 取当前组的 RTF 路径（链式安全取值，任何一环为 None 都回退空串）。
    pub fn get_cur_rtf(&self) -> String {
        self.cur_group
            .as_ref()
            .and_then(|g| self.prf_cfg.groups.get(g))
            .map(|g| g.rtf.clone())
            .unwrap_or_default()
    }

    /// 设置当前组的 RTF 路径，并落盘。无当前组时返回 false。
    pub fn set_cur_rtf(&mut self, rtf: &str) -> Result<bool, String> {
        let Some(g) = self.cur_group.clone() else {
            return Ok(false);
        };
        if let Some(cfg) = self.prf_cfg.groups.get_mut(&g) {
            cfg.rtf = rtf.to_string();
        }
        self.persist_prf()?;
        Ok(true)
    }

    /// 返回 .user 目录（commands 层拼 cfg.json 路径时用）。
    pub fn user_cfg_path(&self) -> PathBuf {
        self.user_dir.clone()
    }

    /// 把当前档的配置写回 .user/<当前档>.json（多处修改后统一落盘）。
    fn persist_prf(&self) -> Result<(), String> {
        json_store::save(&Self::prf_path(&self.user_dir, &self.cur_prf), &self.prf_cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn missing_cfg_creates_default() {
        let dir = data_dir();
        let pm = ProfileManager::new(None, dir.path()).unwrap();
        assert_eq!(pm.cur_prf, "No Profile");
        assert!(dir.path().join(".user").join("cfg.json").is_file());
    }

    #[test]
    fn corrupt_cfg_backed_up() {
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("cfg.json"), "{ broken json").unwrap();
        let err = ProfileManager::new(None, dir.path()).unwrap_err();
        assert!(err.contains("损坏"));
        assert!(user.join("cfg.json.corrupt").is_file());
    }

    #[test]
    fn create_switch_delete_profile() {
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        // 新建即激活（v1 交互），名字会被 trim
        pm.create_profile(" 档A ").unwrap();
        assert_eq!(pm.cur_prf, "档A");
        assert!(pm.config.profile["档A"]);
        // 同名档不得覆写已有存档文件
        assert!(pm.create_profile("档A").is_err());
        pm.create_profile("档B").unwrap();
        assert_eq!(pm.cur_prf, "档B");
        assert!(pm.config.profile["档B"]);
        assert!(!pm.config.profile["档A"]);
        pm.load_profile("档A").unwrap();
        assert_eq!(pm.cur_prf, "档A");
        assert!(pm.delete_profile("档B").unwrap());
        assert!(!pm.config.profile.contains_key("档B"));
        assert!(!pm.delete_profile("No Profile").unwrap());
    }

    #[test]
    fn group_lifecycle_and_rtf() {
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        pm.create_profile("档A").unwrap();
        let g = pm.ensure_group(r"C:\pack1").unwrap();
        assert_eq!(g, pm.cur_group.clone().unwrap());
        assert!(pm.set_cur_rtf(r"C:\list.rtf").unwrap());
        assert_eq!(pm.get_cur_rtf(), r"C:\list.rtf");
        pm.ensure_group(r"C:\pack2").unwrap();
        assert_eq!(pm.prf_cfg.groups.len(), 2);
        assert!(pm.delete_group(r"C:\pack1").unwrap());
        assert_eq!(pm.prf_cfg.groups.len(), 1);
        // 删除当前组后回退到剩余组
        assert_eq!(pm.cur_group.clone().unwrap(), norm_dir(r"C:\pack2"));
    }

    #[test]
    fn snapshot_swap_on_profile_switch() {
        let pack = tempfile::tempdir().unwrap();
        let img = pack.path().join("pack");
        fs::create_dir_all(&img).unwrap();
        fs::write(
            img.join("config.xml"),
            r#"<record from="A/a" to="graphics/pictures/person/r-1/portrait"/>"#,
        ).unwrap();
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        pm.create_profile("档A").unwrap();
        pm.ensure_group(img.to_str().unwrap()).unwrap();
        // 切走：新建并激活档B，档A 快照应保存
        pm.create_profile("档B").unwrap();
        assert!(group_xml_path(&dir.path().join(".user"), "档A", &norm_dir(img.to_str().unwrap())).is_file());
        // 改写目录里的 config.xml，切回档A 应恢复快照
        fs::write(img.join("config.xml"), "changed").unwrap();
        pm.load_profile("档A").unwrap();
        assert!(fs::read_to_string(img.join("config.xml")).unwrap().contains("A/a"));
    }

    #[test]
    fn created_profile_survives_restart() {
        // 复现并锁定修复：新建存档后重启（new(None)），应回到用户创建的档而非 No Profile。
        // （历史缺陷：create_profile 落盘发生在激活位更新之前，cfg.json 留下双 true。）
        let dir = data_dir();
        {
            let mut pm = ProfileManager::new(None, dir.path()).unwrap();
            pm.create_profile("wer").unwrap();
            assert_eq!(pm.cur_prf, "wer");
        }
        let pm = ProfileManager::new(None, dir.path()).unwrap();
        assert_eq!(pm.cur_prf, "wer"); // 重启后仍是用户创建的档
    }

    #[test]
    fn duplicated_active_flags_normalized() {
        // 历史脏数据：cfg.json 出现多个激活位 → 启动归一化为用户创建的档，并修正落盘。
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(
            user.join("cfg.json"),
            r#"{"Profile": {"No Profile": true, "wer": true}}"#,
        ).unwrap();
        let pm = ProfileManager::new(None, dir.path()).unwrap();
        assert_eq!(pm.cur_prf, "wer"); // 用户创建的档优先于默认档
        let cfg: GlobalCfg = match json_store::load(&user.join("cfg.json")) {
            Ok(c) => c,
            Err(_) => panic!("cfg.json 应可解析"),
        };
        assert!(cfg.profile["wer"]);
        assert!(!cfg.profile["No Profile"]); // 激活位已归一化（单 true）
    }

    #[test]
    fn legacy_single_group_profile_upgraded() {
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(
            user.join("cfg.json"),
            r#"{"Profile": {"档旧": true}}"#,
        ).unwrap();
        fs::write(
            user.join("档旧.json"),
            r#"{"img_dir": "C:\\pack", "rtf": "C:\\l.rtf"}"#,
        ).unwrap();
        fs::write(user.join("档旧.xml"), "snapshot-content").unwrap();
        let pm = ProfileManager::new(None, dir.path()).unwrap();
        assert_eq!(pm.cur_prf, "档旧");
        assert_eq!(pm.cur_group.clone().unwrap(), norm_dir(r"C:\pack"));
        assert_eq!(pm.get_cur_rtf(), r"C:\l.rtf");
        // 旧快照已迁移到新位置
        assert!(group_xml_path(&user, "档旧", &norm_dir(r"C:\pack")).is_file());
        assert!(!user.join("档旧.xml").is_file());
    }
}
