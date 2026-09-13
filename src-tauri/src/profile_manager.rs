//! Profile（存档）与组管理：JSON 原子读写、切换时快照交换、旧版单档格式自动升级。
//! 自旧版 core/ProfileManager.py 移植；groups 用 BTreeMap 保证稳定顺序。
//!
//! 【两个核心概念】
//!   Profile（存档）：一套独立的配置集合，对应一个 FM 存档；不同存档的换脸互不干扰。
//!   组（group）：一个头像包目录。一个存档下可关联多个头像包，每个组记住自己的 RTF 名单。
//! 【快照机制】切换存档时，先把当前存档各组的 config.xml 备份到 .user/<存档>/，
//!   再把目标存档的快照恢复回头像包目录——这样不同存档共享同一头像包目录也不会互相覆盖。
//! 【线程安全】ProfileManager 自身可变且非线程安全，由 commands::AppState 包在 Mutex 里串行化访问。
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

/// 全局配置（对应 .user/profiles.json）。字段名 Profile 在 JSON 里首字母大写，用 rename 对应。
/// value 为 bool 表示该存档是否「当前激活」（运行中仅一个为 true；启动时全部归零）。
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
    /// 创建管理器：读全局 profiles.json（缺失则初始化一个只含 'No Profile' 的默认配置），并加载当前存档。
    /// name=None 时固定从默认档（No Profile）开始，不自动恢复上次激活的存档。
    pub fn new(name: Option<&str>, data_dir: &Path) -> Result<Self, String> {
        let user_dir = data_dir.join(".user");
        fs::create_dir_all(&user_dir).map_err(|e| e.to_string())?;
        let cfg_path = Self::global_cfg_path(&user_dir);
        let legacy_cfg_path = user_dir.join("cfg.json"); // 旧名 cfg.json，启动时自动迁移到新名
        // 加载全局配置：新名成功则用；新名缺失但旧名存在 → 迁移（内容沿用、删旧文件）；
        // 两者都缺失 → 写默认；损坏则报错（json_store 已自动备份 .corrupt）。
        let mut config: GlobalCfg = match json_store::load(&cfg_path) {
            Ok(c) => c,
            Err(LoadError::Missing) => match json_store::load(&legacy_cfg_path) {
                Ok(c) => {
                    let _ = fs::remove_file(&legacy_cfg_path);
                    json_store::save(&cfg_path, &c)?; // 迁移：内容写入新名
                    c
                }
                Err(LoadError::Missing) => {
                    // 首次运行：创建一个只含 'No Profile'（未激活）的默认配置并落盘。
                    let c = GlobalCfg {
                        profile: [("No Profile".to_string(), false)].into_iter().collect(),
                    };
                    json_store::save(&cfg_path, &c)?;
                    c
                }
                Err(LoadError::Corrupt(e)) => return Err(e),
            },
            Err(LoadError::Corrupt(e)) => return Err(e),
        };
        // 程序启动固定从默认档（No Profile）开始，不自动恢复上次激活的存档，
        // 需要用户在下拉框手动选择。显式 name（内部调用/测试）仍可指定具体档。
        // profiles.json 的激活位仅运行中记录当前档：启动时全部归零（所有存档「未激活」），
        // 有残留激活位则落盘，保证后端状态与前端「未选择 Profile」的展示一致。
        let cur_prf = name
            .filter(|n| config.profile.contains_key(*n))
            .map(|n| n.to_string())
            .unwrap_or_else(|| "No Profile".to_string());
        if config.profile.values().any(|v| *v) {
            for v in config.profile.values_mut() {
                *v = false;
            }
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

    /// 全局配置文件路径（存档列表）：<user_dir>/profiles.json。
    /// 旧名 cfg.json 仍会在启动时被自动迁移到该文件（见 new）。
    fn global_cfg_path(user_dir: &Path) -> PathBuf {
        user_dir.join("profiles.json")
    }

    /// 存档配置文件路径：<user_dir>/<存档名>/<存档名>.json——与该存档的 XML 快照同目录。
    /// 【例外】缺省档 'No Profile' 不套子目录，直接存 .user/No Profile.json。
    fn prf_path(user_dir: &Path, name: &str) -> PathBuf {
        if name == "No Profile" {
            return user_dir.join("No Profile.json");
        }
        user_dir
            .join(crate::paths::sanitize(name))
            .join(format!("{}.json", crate::paths::sanitize(name)))
    }

    /// 旧版存档配置文件路径（迁移前）：<user_dir>/<存档名>.json（位于 .user 根下）。
    fn legacy_prf_path(user_dir: &Path, name: &str) -> PathBuf {
        user_dir.join(format!("{name}.json"))
    }

    /// 加载档配置；缺失兑底空配置；旧版单档格式自动升级（含快照迁移）。
    /// 【位置迁移】存档 json 已统一移入存档子目录（与 XML 快照同目录）：
    /// 发现旧位置 <user>/<存档名>.json 时自动迁移到 <user>/<存档名>/<存档名>.json。
    fn load_profile_cfg(user_dir: &Path, name: &str) -> Result<PrfCfg, String> {
        let path = Self::prf_path(user_dir, name);
        // 先按位置取原始 JSON 值：新路径 → 旧路径（迁移）→ 缺失（写默认）。
        let mut migrated = false; // 本次是否从旧位置迁来（迁移内容必须写入新位置，否则重启即丢）
        let raw = match json_store::load::<serde_json::Value>(&path) {
            Ok(v) => Some(v),
            Err(LoadError::Missing) => {
                let legacy = Self::legacy_prf_path(user_dir, name);
                match json_store::load::<serde_json::Value>(&legacy) {
                    // 旧位置有文件：读出内容并删除旧文件（新位置在下方处理完成后写入）。
                    Ok(v) => {
                        migrated = true;
                        let _ = fs::remove_file(&legacy);
                        Some(v)
                    }
                    Err(LoadError::Missing) => {
                        // 全新存档：确保子目录存在并写默认配置。
                        if let Some(p) = path.parent() {
                            fs::create_dir_all(p).map_err(|e| e.to_string())?;
                        }
                        let cfg = PrfCfg::default();
                        json_store::save(&path, &cfg)?;
                        // 兼容清理：此前版本为 No Profile 套过的子目录（现已回归根直存），空则移除。
                        if name == "No Profile" {
                            let legacy_dir = user_dir.join(crate::paths::sanitize(name));
                            if legacy_dir.is_dir() {
                                let _ = fs::remove_dir(&legacy_dir); // 非空保留，空则移除
                            }
                        }
                        return Ok(cfg);
                    }
                    Err(LoadError::Corrupt(e)) => return Err(e),
                }
            }
            Err(LoadError::Corrupt(e)) => return Err(e),
        };
        let raw = raw.expect("上方已保证 raw 存在");
        // 确保存档子目录存在（json 与 XML 快照同目录）。
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        // 旧版单档格式：{"img_dir": "X", "rtf": "Y"} 且无 groups → 升级为新的多组格式。
        if raw.get("img_dir").is_some() && raw.get("groups").is_none() {
            let img_dir = norm_dir(raw["img_dir"].as_str().unwrap_or(""));
            let rtf = raw["rtf"].as_str().unwrap_or("").to_string();
            let mut groups = GroupsMap::new();
            if !img_dir.is_empty() {
                groups.insert(img_dir.clone(), GroupCfg { rtf });
            }
            let cfg = PrfCfg {
                cur_group: (!img_dir.is_empty()).then_some(img_dir),
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
        // 新格式：反序列化；若是位置迁移场景，把内容写入新位置（否则旧文件已删、配置会丢）。
        let cfg: PrfCfg = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        if migrated {
            json_store::save(&path, &cfg)?;
        }
        Ok(cfg)
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
        json_store::save(&Self::global_cfg_path(&self.user_dir), &self.config)?; // 落盘全局配置
        // 建空档文件（存档子目录下，与 XML 快照同目录）。
        let dir = self.user_dir.join(crate::paths::sanitize(name));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        json_store::save(&Self::prf_path(&self.user_dir, name), &PrfCfg::default())?;
        self.load_profile(name) // 立即切到新档
    }

    /// 删除档。'No Profile' 不可删（返回 false）；删除「当前激活档」时自动切回 'No Profile'，
    /// 删除未激活档不影响当前档（浏览态下可安全删除下拉选中的存档）。
    pub fn delete_profile(&mut self, name: &str) -> Result<bool, String> {
        if name == "No Profile" || !self.config.profile.contains_key(name) {
            return Ok(false);
        }
        let was_current = self.cur_prf == name;
        self.config.profile.remove(name); // 从全局配置移除
        let _ = fs::remove_file(Self::prf_path(&self.user_dir, name)); // 删档配置文件（新位置）
        let _ = fs::remove_file(Self::legacy_prf_path(&self.user_dir, name)); // 兼容删除旧位置档 json
        let _ = fs::remove_dir_all(self.user_dir.join(crate::paths::sanitize(name))); // 删快照目录（含 XML 与 json）
        json_store::save(&Self::global_cfg_path(&self.user_dir), &self.config)?;
        if was_current {
            self.load_profile("No Profile")?; // 只在删除当前激活档时回到默认档
        }
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
        // 4) 更新全局激活位：只有目标档为 true，其余置 false；并落盘 profiles.json
        //    （激活位必须落盘：此前漏写时新建存档重启会回到 No Profile）。
        for k in self.config.profile.keys().cloned().collect::<Vec<_>>() {
            if let Some(flag) = self.config.profile.get_mut(&k) {
                *flag = k == name;
            }
        }
        json_store::save(&Self::global_cfg_path(&self.user_dir), &self.config)?;
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

    /// 预览指定存档的配置（不切换后端当前档）：返回 (该档配置, 该档当前组)。
    /// 供前端「下拉浏览存档」使用；加载过程可能触发旧布局迁移/默认配置创建（幂等、无破坏）。
    pub fn preview_cfg(&self, name: &str) -> Result<(PrfCfg, Option<String>), String> {
        if !self.config.profile.contains_key(name) {
            return Err(format!("存档「{name}」不存在"));
        }
        let prf_cfg = Self::load_profile_cfg(&self.user_dir, name)?;
        let cur_group = Self::pick_cur_group(&prf_cfg);
        Ok((prf_cfg, cur_group))
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

    /// 返回 .user 目录（commands 层拼 profiles.json 路径时用）。
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
        assert!(dir.path().join(".user").join("profiles.json").is_file());
        // 默认配置：所有存档（含 No Profile）均为「未激活」。
        assert!(!pm.config.profile["No Profile"]);
        // 缺省档 No Profile 的配置直接存 .user 根下，不套子目录。
        assert!(dir.path().join(".user").join("No Profile.json").is_file());
        assert!(!dir.path().join(".user").join("No Profile").exists());
    }

    #[test]
    fn corrupt_cfg_backed_up() {
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("profiles.json"), "{ broken json").unwrap();
        let err = ProfileManager::new(None, dir.path()).unwrap_err();
        assert!(err.contains("损坏"));
        assert!(user.join("profiles.json.corrupt").is_file());
    }

    #[test]
    fn legacy_cfg_json_migrated_to_profiles_json() {
        // 旧名 cfg.json → 启动时自动迁移为 profiles.json（内容沿用、激活位归零、旧文件删除）。
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(
            user.join("cfg.json"),
            r#"{"Profile": {"No Profile": false, "wer": true}}"#,
        ).unwrap();
        let pm = ProfileManager::new(None, dir.path()).unwrap();
        assert!(user.join("profiles.json").is_file());
        assert!(!user.join("cfg.json").is_file());
        // 迁移后启动交互照旧：回到默认档、激活位归零
        assert_eq!(pm.cur_prf, "No Profile");
        let cfg: GlobalCfg = match json_store::load(&user.join("profiles.json")) {
            Ok(c) => c,
            Err(_) => panic!("profiles.json 应可解析"),
        };
        assert!(cfg.profile.values().all(|v| !*v));
        assert!(cfg.profile.contains_key("wer")); // 存档名单保留
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
        assert!(pm.delete_profile("档B").unwrap()); // 删非激活档：不影响当前档
        assert!(!pm.config.profile.contains_key("档B"));
        assert_eq!(pm.cur_prf, "档A"); // 当前档保持
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
    fn restart_starts_at_no_profile() {
        // 启动交互：程序启动固定回到默认档（No Profile），不自动恢复上次激活的存档，
        // 且持久化状态归零——profiles.json 中所有存档的激活位均为 false（全部「未激活」）。
        let dir = data_dir();
        let user = dir.path().join(".user");
        {
            let mut pm = ProfileManager::new(None, dir.path()).unwrap();
            pm.create_profile("wer").unwrap();
            assert_eq!(pm.cur_prf, "wer");
        }
        let pm = ProfileManager::new(None, dir.path()).unwrap();
        assert_eq!(pm.cur_prf, "No Profile");
        assert!(pm.config.profile.contains_key("wer")); // 存档保留
        let cfg: GlobalCfg = match json_store::load(&user.join("profiles.json")) {
            Ok(c) => c,
            Err(_) => panic!("profiles.json 应可解析"),
        };
        assert!(cfg.profile.values().all(|v| !*v)); // 持久化激活位全部未激活
    }

    #[test]
    fn preview_cfg_does_not_switch_current() {
        // 预览不切档：preview_cfg 返回目标档配置，但当前档/激活状态保持不变。
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        pm.create_profile("wer").unwrap();
        pm.ensure_group(r"C:\pack1").unwrap();
        // 预览另一存档（No Profile）：返回其空配置，当前档与当前组不变。
        let (cfg, cur_group) = pm.preview_cfg("No Profile").unwrap();
        assert!(cfg.groups.is_empty());
        assert_eq!(cur_group, None);
        assert_eq!(pm.cur_prf, "wer");
        assert_eq!(pm.cur_group.clone().unwrap(), norm_dir(r"C:\pack1"));
        // 不存在的存档 → 报错
        assert!(pm.preview_cfg("ghost").is_err());
    }

    #[test]
    fn legacy_single_group_profile_upgraded() {
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(
            user.join("profiles.json"),
            r#"{"Profile": {"档旧": true}}"#,
        ).unwrap();
        fs::write(
            user.join("档旧.json"),
            r#"{"img_dir": "C:\\pack", "rtf": "C:\\l.rtf"}"#,
        ).unwrap();
        fs::write(user.join("档旧.xml"), "snapshot-content").unwrap();
        // 显式指定档名：本用例验证「旧格式升级与迁移」，与「启动默认 No Profile」交互解耦。
        let pm = ProfileManager::new(Some("档旧"), dir.path()).unwrap();
        assert_eq!(pm.cur_prf, "档旧");
        assert_eq!(pm.cur_group.clone().unwrap(), norm_dir(r"C:\pack"));
        assert_eq!(pm.get_cur_rtf(), r"C:\l.rtf");
        // 旧快照已迁移到新位置
        assert!(group_xml_path(&user, "档旧", &norm_dir(r"C:\pack")).is_file());
        assert!(!user.join("档旧.xml").is_file());
        // 存档 json 也已位于存档子目录（与 XML 快照同目录）
        assert!(user.join("档旧").join("档旧.json").is_file());
    }

    #[test]
    fn legacy_prf_json_moved_into_profile_dir() {
        // 旧布局（.user 根下 <存档>.json）→ 加载后自动迁移到 <user>/<存档>/<存档>.json
        // （与该存档的 XML 快照同目录），组与 RTF 配置原样保留。
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("profiles.json"), r#"{"Profile": {"档A": true}}"#).unwrap();
        fs::write(
            user.join("档A.json"),
            r#"{"cur_group": "C:\\pack\\", "groups": {"C:\\pack\\": {"rtf": "C:\\l.rtf"}}}"#,
        ).unwrap();
        // 显式指定档名：本用例验证「旧布局 json 位置迁移」，与启动默认档交互解耦。
        let pm = ProfileManager::new(Some("档A"), dir.path()).unwrap();
        assert_eq!(pm.get_cur_rtf(), r"C:\l.rtf"); // 配置内容原样保留
        let new_path = user.join("档A").join("档A.json");
        assert!(new_path.is_file(), "存档 json 应移入存档子目录");
        assert!(!user.join("档A.json").is_file(), "旧位置不应保留");
    }
}
