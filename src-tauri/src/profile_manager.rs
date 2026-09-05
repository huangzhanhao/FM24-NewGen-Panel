//! Profile（档）/ 组管理：JSON 原子读写、切换时快照交换、旧版单档格式自动升级
//! 自 core/ProfileManager.py 移植；groups 用 BTreeMap 保证稳定顺序
use crate::json_store::{self, LoadError};
use crate::paths::{group_xml_path, norm_dir};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GroupCfg {
    #[serde(default)]
    pub rtf: String,
}

pub type GroupsMap = BTreeMap<String, GroupCfg>;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PrfCfg {
    #[serde(default)]
    pub cur_group: Option<String>,
    #[serde(default)]
    pub groups: GroupsMap,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlobalCfg {
    #[serde(rename = "Profile")]
    pub profile: BTreeMap<String, bool>,
}

#[derive(Debug)]
pub struct ProfileManager {
    user_dir: PathBuf, // <data>/.user
    pub config: GlobalCfg,
    pub cur_prf: String,
    pub prf_cfg: PrfCfg,
    pub cur_group: Option<String>,
}

impl ProfileManager {
    pub fn new(name: Option<&str>, data_dir: &Path) -> Result<Self, String> {
        let user_dir = data_dir.join(".user");
        fs::create_dir_all(&user_dir).map_err(|e| e.to_string())?;
        let cfg_path = user_dir.join("cfg.json");
        let config: GlobalCfg = match json_store::load(&cfg_path) {
            Ok(c) => c,
            Err(LoadError::Missing) => {
                let c = GlobalCfg {
                    profile: [("No Profile".to_string(), true)].into_iter().collect(),
                };
                json_store::save(&cfg_path, &c)?;
                c
            }
            Err(LoadError::Corrupt(e)) => return Err(e),
        };
        // 对等 v1：name 缺失/无效时回退到 cfg.json 中激活的档（get_latest_prf 语义）
        let cur_prf = name
            .filter(|n| config.profile.contains_key(*n))
            .map(|n| n.to_string())
            .or_else(|| {
                config
                    .profile
                    .iter()
                    .find(|(_, v)| **v)
                    .map(|(k, _)| k.clone())
            })
            .unwrap_or_else(|| "No Profile".to_string());
        let prf_cfg = Self::load_profile_cfg(&user_dir, &cur_prf)?;
        let cur_group = Self::pick_cur_group(&prf_cfg);
        Ok(Self { user_dir, config, cur_prf, prf_cfg, cur_group })
    }

    fn pick_cur_group(cfg: &PrfCfg) -> Option<String> {
        cfg.cur_group
            .clone()
            .filter(|g| cfg.groups.contains_key(g))
            .or_else(|| cfg.groups.keys().next().cloned())
    }

    fn prf_path(user_dir: &Path, name: &str) -> PathBuf {
        user_dir.join(format!("{name}.json"))
    }

    /// 加载档配置；缺失兜底空配置；旧版单档格式自动升级（含快照迁移）
    fn load_profile_cfg(user_dir: &Path, name: &str) -> Result<PrfCfg, String> {
        let path = Self::prf_path(user_dir, name);
        let raw = match json_store::load::<serde_json::Value>(&path) {
            Ok(v) => v,
            Err(LoadError::Missing) => {
                let cfg = PrfCfg::default();
                json_store::save(&path, &cfg)?;
                return Ok(cfg);
            }
            Err(LoadError::Corrupt(e)) => return Err(e),
        };
        // 旧版单档：{"img_dir": "X", "rtf": "Y"} 且无 groups
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
            json_store::save(&path, &cfg)?;
            // 旧快照 .user/<name>.xml → .user/<name>/<sanitized>.xml
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
        serde_json::from_value(raw).map_err(|e| e.to_string())
    }

    // ---------------- Profile 管理 ----------------
    pub fn create_profile(&mut self, name: &str) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("Profile 名不能为空".into());
        }
        self.config.profile.insert(name.to_string(), true);
        json_store::save(&self.user_dir.join("cfg.json"), &self.config)?;
        json_store::save(&Self::prf_path(&self.user_dir, name), &PrfCfg::default())?;
        self.load_profile(name)
    }

    pub fn delete_profile(&mut self, name: &str) -> Result<bool, String> {
        if name == "No Profile" {
            return Ok(false);
        }
        self.config.profile.remove(name);
        let _ = fs::remove_file(Self::prf_path(&self.user_dir, name));
        let _ = fs::remove_dir_all(self.user_dir.join(crate::paths::sanitize(name)));
        json_store::save(&self.user_dir.join("cfg.json"), &self.config)?;
        self.load_profile("No Profile")?;
        Ok(true)
    }

    /// 切换档：保存旧档各组快照 → 加载新档 → 恢复新档各组快照 → 更新激活位
    pub fn load_profile(&mut self, name: &str) -> Result<(), String> {
        let old = self.cur_prf.clone();
        for img_dir in self.prf_cfg.groups.keys().cloned().collect::<Vec<_>>() {
            self.save_group_xml(&old, &img_dir);
        }
        self.prf_cfg = Self::load_profile_cfg(&self.user_dir, name)?;
        for img_dir in self.prf_cfg.groups.keys().cloned().collect::<Vec<_>>() {
            self.restore_group_xml(name, &img_dir);
        }
        for k in self.config.profile.keys().cloned().collect::<Vec<_>>() {
            if let Some(flag) = self.config.profile.get_mut(&k) {
                *flag = k == name;
            }
        }
        self.cur_prf = name.to_string();
        self.cur_group = Self::pick_cur_group(&self.prf_cfg);
        Ok(())
    }

    fn save_group_xml(&self, profile: &str, img_dir: &str) {
        let src = Path::new(img_dir).join("config.xml");
        if !src.is_file() {
            return;
        }
        let dst = group_xml_path(&self.user_dir, profile, img_dir);
        if let Some(p) = dst.parent() {
            let _ = fs::create_dir_all(p);
        }
        let _ = fs::copy(&src, &dst);
    }

    fn restore_group_xml(&self, profile: &str, img_dir: &str) {
        let src = group_xml_path(&self.user_dir, profile, img_dir);
        let dst = Path::new(img_dir).join("config.xml");
        // 头像包目录已不存在（换盘/被删）：保留快照，跳过恢复（对等 v1）
        if src.is_file() && Path::new(img_dir).is_dir() {
            let _ = fs::copy(&src, &dst);
        }
    }

    // ---------------- 组管理 ----------------
    pub fn list_groups(&self) -> Vec<String> {
        self.prf_cfg.groups.keys().cloned().collect()
    }

    /// 新头像包目录自动建组并设为当前组（对等 ensure_group）
    pub fn ensure_group(&mut self, img_dir: &str) -> Option<String> {
        let dir = norm_dir(img_dir);
        if dir.is_empty() {
            return None;
        }
        if !self.prf_cfg.groups.contains_key(&dir) {
            self.prf_cfg.groups.insert(dir.clone(), GroupCfg::default());
        }
        self.set_cur_group(&dir).ok()?;
        Some(dir)
    }

    pub fn set_cur_group(&mut self, img_dir: &str) -> Result<bool, String> {
        let dir = norm_dir(img_dir);
        if !self.prf_cfg.groups.contains_key(&dir) {
            return Ok(false);
        }
        self.cur_group = Some(dir.clone());
        self.prf_cfg.cur_group = Some(dir);
        self.persist_prf()?;
        Ok(true)
    }

    pub fn delete_group(&mut self, img_dir: &str) -> Result<bool, String> {
        let dir = norm_dir(img_dir);
        if self.prf_cfg.groups.remove(&dir).is_none() {
            return Ok(false);
        }
        let snap = group_xml_path(&self.user_dir, &self.cur_prf, &dir);
        if snap.is_file() {
            let _ = fs::remove_file(&snap);
        }
        if self.cur_group.as_deref() == Some(dir.as_str()) {
            self.cur_group = Self::pick_cur_group(&self.prf_cfg);
            self.prf_cfg.cur_group = self.cur_group.clone();
        }
        self.persist_prf()?;
        Ok(true)
    }

    pub fn get_group_rtf(&self, img_dir: &str) -> String {
        self.prf_cfg
            .groups
            .get(&norm_dir(img_dir))
            .map(|g| g.rtf.clone())
            .unwrap_or_default()
    }

    pub fn get_cur_rtf(&self) -> String {
        self.cur_group
            .as_ref()
            .and_then(|g| self.prf_cfg.groups.get(g))
            .map(|g| g.rtf.clone())
            .unwrap_or_default()
    }

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
        pm.create_profile("档A").unwrap();
        assert_eq!(pm.cur_prf, "档A");
        assert!(pm.config.profile["档A"]);
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
        assert_eq!(pm.list_groups().len(), 2);
        assert!(pm.delete_group(r"C:\pack1").unwrap());
        assert_eq!(pm.list_groups().len(), 1);
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
        // 切走：档A 快照应保存
        pm.create_profile("档B").unwrap();
        assert!(group_xml_path(&dir.path().join(".user"), "档A", &norm_dir(img.to_str().unwrap())).is_file());
        // 改写目录里的 config.xml，切回档A 应恢复快照
        fs::write(img.join("config.xml"), "changed").unwrap();
        pm.load_profile("档A").unwrap();
        assert!(fs::read_to_string(img.join("config.xml")).unwrap().contains("A/a"));
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
