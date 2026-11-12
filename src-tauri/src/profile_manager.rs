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
use crate::logger::LogSinkDyn; // 档景（快照）存取异常要进日志（不能静默失败）
use crate::paths::{group_xml_path, norm_dir};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap; // 有序字典：保证组/存档的遍历顺序稳定
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 单个组的配置。RTF 与 .fm 存档路径并列，各自独立选择（#[serde(default)] 表示旧文件缺字段时补默认值，
/// 老副本不手工改、由反序列化兜底为空串）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GroupCfg {
    #[serde(default)]
    pub rtf: String,
    #[serde(default)]
    pub fm: String,
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
pub struct ProfileManager {
    user_dir: PathBuf,   // <data>/.user 目录
    pub config: GlobalCfg,       // 全局配置（哪些存档、哪个激活）
    pub cur_prf: String,         // 当前激活存档名
    pub prf_cfg: PrfCfg,         // 当前存档的配置（组、当前组）
    pub cur_group: Option<String>, // 当前选中的组
    /// 日志出口（**后置注入**：lib.rs 先建 logger 再建本管理器，测试则不注入＝不写日志）。
    /// 只用来记「档景（快照）存取异常」——这类失败会让某个档的现有映射静默变旧，
    /// 是替换判据出错的源头（见 FaceMapper::existing_xml），必须能在日志里看见。
    logger: Option<Arc<dyn LogSinkDyn>>,
}

/// 手写 Debug：`dyn LogSinkDyn` 没有 Debug，不能 derive（日志器不该出现在调试输出里）。
impl std::fmt::Debug for ProfileManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileManager")
            .field("user_dir", &self.user_dir)
            .field("cur_prf", &self.cur_prf)
            .field("prf_cfg", &self.prf_cfg)
            .field("cur_group", &self.cur_group)
            .finish_non_exhaustive()
    }
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
        Ok(Self { user_dir, config, cur_prf, prf_cfg, cur_group, logger: None })
    }

    /// 注入日志出口（lib.rs 在构造后调用；测试注入 `ChannelLogger` 以便断言告警）。
    /// 【为什么不做成构造参数】本管理器的绝大多数用例（含单元测试）与日志无关，
    /// 后置注入让 `new` 的签名与既有调用点（8 处测试）保持不变。
    pub fn set_logger(&mut self, logger: Arc<dyn LogSinkDyn>) {
        self.logger = Some(logger);
        // 注入后即可记录：确认后续存档/组操作都会被日志覆盖（含当前档与组数）。
        self.log(
            log::Level::Debug,
            format!(
                "存档模块日志出口已就绪：当前档 {}，组 {} 个",
                self.cur_prf,
                self.prf_cfg.groups.len()
            ),
        );
    }

    /// 记一条日志（未注入 logger 时静默）。模块名用 profile，与命令层的建档/切档日志同源，
    /// 方便在日志抽屉里按模块串起来看。
    fn log(&self, level: log::Level, msg: String) {
        if let Some(l) = &self.logger {
            l.write(level, "profile", msg);
        }
    }

    /// 记一条 WARNING（未注入 logger 时静默）。转发给 `log`，保留给既有的告警调用点。
    fn warn(&self, msg: String) {
        self.log(log::Level::Warn, msg);
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
                        // 兼容清理：No Profile 旧布局可能残留的子目录，空则移除。
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
                groups.insert(img_dir.clone(), GroupCfg { rtf, ..Default::default() });
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

    // ---- Profile 管理 -----------------------------------------------------------------------------------------------
    /// 新建档：建好空档文件并立即切档（v1 交互：新建即激活）。
    pub fn create_profile(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            self.log(log::Level::Warn, "新建存档被拒绝：Profile 名不能为空".into());
            return Err("Profile 名不能为空".into());
        }
        // 【目录名闸门】存档名会直接当作 <user_dir> 下的子目录名用（档配置与档景快照都存在里面），
        // 所以 `..` 这类名字必须拒在入口：若放过去，删除该存档时会拿 user_dir/.. 去 remove_dir_all，
        // 整个便携数据目录（全部存档、快照、规则、日志）一次清光且不可恢复。
        if !crate::paths::is_safe_profile_name(name) {
            self.log(
                log::Level::Warn,
                format!("新建存档被拒绝：Profile 名「{name}」不能用作目录名"),
            );
            return Err(
                "存档名不能全部由点号组成，也不能以点号或空格结尾（Windows 目录名限制）".into(),
            );
        }
        // 重名保护：已存在则报错，避免覆写别人的存档。
        if self.config.profile.contains_key(name) {
            self.log(
                log::Level::Warn,
                format!("新建存档被拒绝：Profile「{name}」已存在"),
            );
            return Err(format!("Profile「{name}」已存在"));
        }
        self.log(log::Level::Debug, format!("开始新建存档：{name}"));
        self.config.profile.insert(name.to_string(), true); // 加入全局配置
        json_store::save(&Self::global_cfg_path(&self.user_dir), &self.config)?; // 落盘全局配置
        // 建空档文件（存档子目录下，与 XML 快照同目录）。
        let dir = self.user_dir.join(crate::paths::sanitize(name));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        json_store::save(&Self::prf_path(&self.user_dir, name), &PrfCfg::default())?;
        self.load_profile(name)?; // 立即切到新档
        self.log(log::Level::Info, format!("存档已新建并激活：{name}"));
        Ok(())
    }

    /// 删除档。'No Profile' 不可删（返回 false）；删除「当前激活档」时自动切回 'No Profile'，
    /// 删除未激活档不影响当前档（浏览态下可安全删除下拉选中的存档）。
    pub fn delete_profile(&mut self, name: &str) -> Result<bool, String> {
        if name == "No Profile" || !self.config.profile.contains_key(name) {
            self.log(
                log::Level::Debug,
                format!("删除存档跳过：默认档或不存在（{name}）"),
            );
            return Ok(false);
        }
        self.log(log::Level::Debug, format!("开始删除存档：{name}"));
        let was_current = self.cur_prf == name;
        self.config.profile.remove(name); // 从全局配置移除
        let _ = fs::remove_file(Self::prf_path(&self.user_dir, name)); // 删档配置文件（新位置）
        let _ = fs::remove_file(Self::legacy_prf_path(&self.user_dir, name)); // 兼容删除旧位置档 json
        // 删快照目录（含 XML 与 json）。删不掉要留痕：同名存档日后重建时，残留的旧快照会被
        // 当成它的现有映射（判据是快照），等于把已删档的脸悄悄带回新档。
        let snap_dir = self.user_dir.join(crate::paths::sanitize(name));
        // 【remove_dir_all 前的两道闸】这一步不可逆，所以不信任来路：
        // ① 畸形名一律不碰文件系统。老版本的 profiles.json 里可能已经存着 `..`（当时入口没校验），
        //    sanitize 会把这类名字收敛成 "default"，照它去删就等于误删另一个存档的目录。
        // ② 解析结果必须正好落在 user_dir 的下一层，任何越界都拒绝删除并留痕。
        if !crate::paths::is_safe_profile_name(name) {
            self.warn(format!(
                "删除存档 {name}：名字不适合用作目录名，已只从配置中移除、未删除 {}（可手动清理）",
                snap_dir.display()
            ));
        } else if snap_dir.parent() != Some(self.user_dir.as_path()) {
            self.warn(format!(
                "删除存档 {name}：快照目录不在存档根目录下（{}），已跳过删除以免波及上层目录",
                snap_dir.display()
            ));
        } else if snap_dir.exists() {
            if let Err(e) = fs::remove_dir_all(&snap_dir) {
                self.warn(format!(
                    "删除存档 {name}：快照目录删除失败（{e}）：{}。同名存档重建时会沿用这份旧档景",
                    snap_dir.display()
                ));
            }
        }
        json_store::save(&Self::global_cfg_path(&self.user_dir), &self.config)?;
        if was_current {
            self.load_profile("No Profile")?; // 只在删除当前激活档时回到默认档
        }
        self.log(
            log::Level::Info,
            format!("存档已删除：{name}（原为当前档={was_current}）"),
        );
        Ok(true)
    }

    /// 切换档：保存旧档各组快照 → 加载新档 → 恢复新档各组快照 → 更新激活位。
    /// 【核心】这就是「快照交换」：离开时存下、进入时取回，保证每个档看到自己的 config.xml。
    pub fn load_profile(&mut self, name: &str) -> Result<(), String> {
        let old = self.cur_prf.clone();
        // 函数入口：Debug 级（默认不输出）。切档方向并入下面的「完成」汇总行，避免一次切档记两条 Info。
        self.log(log::Level::Debug, format!("开始切换存档：{old} → {name}"));
        // 1) 把旧档每个组的 config.xml 备份到快照目录。
        //（先 collect 成 Vec 再遍历，避免遍历中修改 groups 导致借用冲突）
        for img_dir in self.prf_cfg.groups.keys().cloned().collect::<Vec<_>>() {
            self.save_group_xml(&old, &img_dir);
        }
        // 2) 加载新档的组配置。
        self.prf_cfg = Self::load_profile_cfg(&self.user_dir, name)?;
        self.log(
            log::Level::Debug,
            format!("已加载存档「{name}」配置：{} 个组", self.prf_cfg.groups.len()),
        );
        // 3) 把新档每个组的快照恢复回头像包目录。
        for img_dir in self.prf_cfg.groups.keys().cloned().collect::<Vec<_>>() {
            self.restore_group_xml(name, &img_dir);
        }
        // 4) 更新全局激活位：只有目标档为 true，其余置 false；并落盘 profiles.json
        //    （激活位必须落盘，否则新建存档重启会回到 No Profile）。
        for k in self.config.profile.keys().cloned().collect::<Vec<_>>() {
            if let Some(flag) = self.config.profile.get_mut(&k) {
                *flag = k == name;
            }
        }
        json_store::save(&Self::global_cfg_path(&self.user_dir), &self.config)?;
        self.cur_prf = name.to_string();
        self.cur_group = Self::pick_cur_group(&self.prf_cfg);
        // 切档完成：一次切档只记这一条 Info（合并了入口的切档方向），承载关键运行状态
        // （当前档 / 当前组 / 组数），后续排查串档问题只看它即可。
        self.log(
            log::Level::Info,
            format!(
                "存档已切换：{old} → {name}，当前组 {}（{} 个组）",
                self.cur_group.as_deref().unwrap_or("无"),
                self.prf_cfg.groups.len()
            ),
        );
        Ok(())
    }

    /// 把头像包目录里的 config.xml 拷到该档的快照位置（切走时调用）。
    /// 【失败必须留痕】快照是「本档本组的现有映射」的唯一真值源（替换的保留/覆写判据取自它），
    /// 备份失败＝该档的档景停留在上一次的内容，而界面与后续替换都不会有任何提示。
    fn save_group_xml(&self, profile: &str, img_dir: &str) {
        let src = Path::new(img_dir).join("config.xml");
        if !src.is_file() {
            self.log(
                log::Level::Trace,
                format!("保存档景跳过：{img_dir} 下没有 config.xml（尚未换脸）"),
            );
            return; // 目录里没有 config.xml（还没换过脸），无需备份
        }
        let dst = group_xml_path(&self.user_dir, profile, img_dir);
        if let Some(p) = dst.parent() {
            if let Err(e) = fs::create_dir_all(p) {
                self.warn(format!(
                    "保存档景失败：无法创建快照目录（{e}）。{profile} 在 {img_dir} 的档景未更新，\
                     仍为上一次保存的内容；本次切走后该档可能按旧档景判断「谁已换过脸」"
                ));
                return;
            }
        }
        // 【原子写 + 写前备份】两处拷贝都不走截断覆写：档景是替换判据源、恢复目标又是用户的
        // config.xml。覆盖档景之前先把盘上那一份轮转备份到同目录（见 overwrite_snapshot）——
        // 一次误替换或一次被跳过的 restore 就能把它改成别档的内容，而它是唯一的记录。
        if let Err(e) = crate::xml_parser::overwrite_snapshot(&src, &dst) {
            self.warn(format!(
                "保存档景失败：{e}。{profile} 在 {img_dir} 的档景未更新（仍为上一次的内容）"
            ));
        } else {
            self.log(
                log::Level::Debug,
                format!("保存档景完成：{profile} @ {img_dir}"),
            );
        }
    }

    /// 把快照恢复回头像包目录的 config.xml（切入时调用）。
    /// 【三条跳过/失败分支都要留痕】live `config.xml` 是**全档共享**的一份，它的归属完全靠
    /// save/restore 配对维持；任一条静默跳过，live 里就会留下**上一个档**的内容——而 FM 读的正是它，
    /// 用户会看到别档的头像（替换判据已改读快照，故这只影响显示与 FM 实际读取）。
    fn restore_group_xml(&self, profile: &str, img_dir: &str) {
        let src = group_xml_path(&self.user_dir, profile, img_dir);
        let dst = Path::new(img_dir).join("config.xml");
        // 分支① 本档在此目录还没有快照（没换过脸、或建组后从未切走过）。
        // 目录里若已有 config.xml，那内容不是本档的：不恢复、也不删（删了会破坏原主的档景），
        // 但要留痕——两档共用同一头像包时，这里就是串档发生的现场。
        if !src.is_file() {
            if dst.is_file() {
                self.warn(format!(
                    "恢复档景：{profile} 在 {img_dir} 还没有快照，目录里的 config.xml 保持原样\
                    （它可能属于其他存档）；本档首次替换将按「无历史」处理"
                ));
            } else {
                self.log(
                    log::Level::Trace,
                    format!("恢复档景跳过：{profile} 在 {img_dir} 无快照，目录里也没有 config.xml"),
                );
            }
            return;
        }
        // 分支② 头像包目录已不存在（换盘/被删/NAS 离线）：保留快照，跳过恢复（对等 v1）。
        if !Path::new(img_dir).is_dir() {
            self.warn(format!(
                "恢复档景：头像包目录不可用（{img_dir}），{profile} 的档景未能恢复到 config.xml"
            ));
            return;
        }
        // 分支③ 拷贝失败：此时 live 里可能仍是上一个档的内容。
        if let Err(e) = json_store::copy_atomic(&src, &dst) {
            self.warn(format!(
                "恢复档景失败：{e}。{img_dir} 里的 config.xml 可能仍是上一个存档的内容"
            ));
        } else {
            self.log(
                log::Level::Debug,
                format!("恢复档景完成：{profile} @ {img_dir}"),
            );
        }
    }

    /// 删掉某个快照的轮转备份（`<快照词干>_backup_*.xml`，与快照同目录）。
    /// 【为什么删组时要一起删】这些备份是覆盖档景前由我们自己产出来的（最多 5 份、单份可达
    /// 数 MB）；组都没了还留着，等于在用户的数据目录里永久堆孤儿文件，且没有任何入口能清它们。
    /// 匹配前缀足够特定（含该组快照的完整词干 + 哈希），不会碰到别的组或用户自己的文件。
    fn drop_snapshot_backups(&self, snapshot_path: &Path) {
        let Some(stem) = snapshot_path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
            return;
        };
        let prefix = format!("{stem}_backup_");
        let Some(dir) = snapshot_path.parent() else { return };
        let Ok(entries) = fs::read_dir(dir) else { return }; // 目录都读不到，谈不上清理
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(&prefix) || !name.ends_with(".xml") {
                continue;
            }
            if let Err(e) = fs::remove_file(entry.path()) {
                self.warn(format!("删除组：旧档景备份清理失败（{name}）：{e}"));
            }
        }
    }

    // ---- 组管理 -----------------------------------------------------------------------------------------------------
    /// 把用户给的头像包目录对上**已登记**的组键：等价写法（大小写、正/反斜杠、尾斜杠）复用同一个键。
    /// 【为什么要这一步】组键只经 `norm_dir` 归一，`C:\FM\NG` 与 `C:\fm\ng` 会记成两个组，
    /// 而快照名是按 Windows 语义折过大小写的——两个组于是共用一份档景，
    /// 正是「一份 live config.xml 对两份档景」换个入口复发。
    /// 没有等价组时返回 `norm_dir(img_dir)`（新组的键）。
    fn match_group(&self, img_dir: &str) -> String {
        let key = norm_dir(img_dir);
        self.prf_cfg
            .groups
            .keys()
            .find(|g| crate::paths::same_dir(g, &key))
            .cloned()
            .unwrap_or(key)
    }

    /// 新头像包目录自动建组并设为当前组（对等旧版 ensure_group）。返回归一化后的目录名。
    pub fn ensure_group(&mut self, img_dir: &str) -> Option<String> {
        let dir = self.match_group(img_dir); // 等价写法先落到已登记的那个键上
        if dir.is_empty() {
            self.log(
                log::Level::Debug,
                format!("建组跳过：目录为空（{img_dir}）"),
            );
            return None;
        }
        // 组不存在就新建一个空配置；已存在则不动（保留原有 RTF）。
        if self.prf_cfg.groups.contains_key(&dir) {
            self.log(log::Level::Debug, format!("组已存在，直接复用：{dir}"));
        } else {
            self.prf_cfg.groups.insert(dir.clone(), GroupCfg::default());
            self.log(log::Level::Debug, format!("新建组：{dir}"));
        }
        self.set_cur_group(&dir).ok()?; // 设为当前组；.ok()? 把失败转为 None
        Some(dir)
    }

    /// 切换当前组（必须已存在），并落盘。
    pub fn set_cur_group(&mut self, img_dir: &str) -> Result<bool, String> {
        let dir = self.match_group(img_dir);
        if !self.prf_cfg.groups.contains_key(&dir) {
            self.log(
                log::Level::Debug,
                format!("切换当前组跳过：组不存在（{dir}）"),
            );
            return Ok(false); // 组不存在
        }
        self.cur_group = Some(dir.clone());
        self.prf_cfg.cur_group = Some(dir);
        self.persist_prf()?; // 把当前档配置写回文件
        self.log(
            log::Level::Debug,
            format!(
                "当前组已切换：{}",
                self.cur_group.as_deref().unwrap_or("无")
            ),
        );
        Ok(true)
    }

    /// 删除组（连同其快照）；若删的是当前组，自动回退到剩下的组。
    pub fn delete_group(&mut self, img_dir: &str) -> Result<bool, String> {
        let dir = self.match_group(img_dir);
        if self.prf_cfg.groups.remove(&dir).is_none() {
            self.log(
                log::Level::Debug,
                format!("删除组跳过：组不存在（{dir}）"),
            );
            return Ok(false); // 组不存在
        }
        // 删快照及其轮转备份。删不掉要留痕：组配置已移除、快照却还在，日后重新关联同一目录时，
        // 这份「该删的备份」会被当成该档的现有映射（替换的保留/覆写判据取自快照）。
        let snap = group_xml_path(&self.user_dir, &self.cur_prf, &dir);
        if snap.is_file() {
            if let Err(e) = fs::remove_file(&snap) {
                self.warn(format!(
                    "删除组：快照文件删除失败（{e}）：{}。若以后重新关联该目录，这份旧档景会被当成现有映射",
                    snap.display()
                ));
            }
            self.drop_snapshot_backups(&snap);
        }
        // 若删的正好是当前组，重新选一个当前组。
        if self.cur_group.as_deref() == Some(dir.as_str()) {
            self.cur_group = Self::pick_cur_group(&self.prf_cfg);
            self.prf_cfg.cur_group = self.cur_group.clone();
        }
        self.persist_prf()?;
        self.log(
            log::Level::Info,
            format!("组已删除：{dir}（剩余 {} 个组）", self.prf_cfg.groups.len()),
        );
        Ok(true)
    }

    /// 预览指定存档的配置（不切换后端当前档）：返回 (该档配置, 该档当前组)。
    /// 供前端「下拉浏览存档」使用；加载过程可能触发旧布局迁移/默认配置创建（幂等、无破坏）。
    pub fn preview_cfg(&self, name: &str) -> Result<(PrfCfg, Option<String>), String> {
        if !self.config.profile.contains_key(name) {
            self.log(
                log::Level::Warn,
                format!("预览存档失败：存档「{name}」不存在"),
            );
            return Err(format!("存档「{name}」不存在"));
        }
        let prf_cfg = Self::load_profile_cfg(&self.user_dir, name)?;
        let cur_group = Self::pick_cur_group(&prf_cfg);
        self.log(
            log::Level::Debug,
            format!(
                "查询存档「{name}」配置用于预览：{} 个组，当前组 {}",
                prf_cfg.groups.len(),
                cur_group.as_deref().unwrap_or("无")
            ),
        );
        Ok((prf_cfg, cur_group))
    }

    /// 取当前组的 RTF 路径（链式安全取值，任何一环为 None 都回退空串）。
    pub fn get_cur_rtf(&self) -> String {
        let rtf = self
            .cur_group
            .as_ref()
            .and_then(|g| self.prf_cfg.groups.get(g))
            .map(|g| g.rtf.clone())
            .unwrap_or_default();
        // 查询：记录当前组与取到的 RTF（Trace 级别，仅用于追踪调用链）。
        self.log(
            log::Level::Trace,
            format!(
                "查询当前组 RTF：{} → {rtf}",
                self.cur_group.as_deref().unwrap_or("无")
            ),
        );
        rtf
    }

    /// 设置当前组的 RTF 路径，并落盘。无当前组时返回 false。
    pub fn set_cur_rtf(&mut self, rtf: &str) -> Result<bool, String> {
        let Some(g) = self.cur_group.clone() else {
            self.log(
                log::Level::Debug,
                "设置 RTF 跳过：当前没有选中的组".into(),
            );
            return Ok(false);
        };
        if let Some(cfg) = self.prf_cfg.groups.get_mut(&g) {
            cfg.rtf = rtf.to_string();
        }
        self.persist_prf()?;
        self.log(
            log::Level::Debug,
            format!("当前组 RTF 已更新：{g} → {rtf}"),
        );
        Ok(true)
    }

    /// 取当前组的 .fm 存档路径（链式安全取值，任何一环为 None 都回退空串）。
    pub fn get_cur_fm(&self) -> String {
        let fm = self
            .cur_group
            .as_ref()
            .and_then(|g| self.prf_cfg.groups.get(g))
            .map(|g| g.fm.clone())
            .unwrap_or_default();
        // 查询：记录当前组与取到的 .fm 路径（Trace 级别，仅用于追踪调用链）。
        self.log(
            log::Level::Trace,
            format!(
                "查询当前组 FM 存档：{} → {fm}",
                self.cur_group.as_deref().unwrap_or("无")
            ),
        );
        fm
    }

    /// 设置当前组的 .fm 存档路径，并落盘。无当前组时返回 false。
    pub fn set_cur_fm(&mut self, fm: &str) -> Result<bool, String> {
        let Some(g) = self.cur_group.clone() else {
            self.log(
                log::Level::Debug,
                "设置 FM 存档跳过：当前没有选中的组".into(),
            );
            return Ok(false);
        };
        if let Some(cfg) = self.prf_cfg.groups.get_mut(&g) {
            cfg.fm = fm.to_string();
        }
        self.persist_prf()?;
        self.log(
            log::Level::Debug,
            format!("当前组 FM 存档已更新：{g} → {fm}"),
        );
        Ok(true)
    }

    /// 返回 .user 目录（commands 层拼 profiles.json 路径时用）。
    pub fn user_cfg_path(&self) -> PathBuf {
        self.user_dir.clone()
    }

    /// 把当前档的配置写回 .user/<当前档>.json（多处修改后统一落盘）。
    fn persist_prf(&self) -> Result<(), String> {
        // 持久化入口：追踪写盘目标；失败必须留痕（配置未落盘＝重启后丢失本次修改）。
        self.log(
            log::Level::Trace,
            format!("落盘当前档配置：{}", self.cur_prf),
        );
        let result = json_store::save(&Self::prf_path(&self.user_dir, &self.cur_prf), &self.prf_cfg);
        if let Err(e) = &result {
            self.log(
                log::Level::Error,
                format!("当前档配置落盘失败（{}）：{e}", self.cur_prf),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::ChannelLogger;

    /// 取出注入的日志出口收到的全部告警正文（模块名固定 profile）。
    /// 只挑 WARNING：正常流程也会记 INFO/DEBUG/TRACE，本辅助函数只关心告警。
    fn warns_of(rx: &std::sync::mpsc::Receiver<crate::logger::LogLine>) -> Vec<String> {
        rx.try_iter()
            .filter(|l| l.level == "WARNING")
            .map(|l| {
                assert_eq!(l.module, "profile");
                l.message
            })
            .collect()
    }

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
    fn dot_profile_name_never_wipes_the_data_dir() {
        // 【CRITICAL 回归】存档名要当作 <user_dir> 下的子目录名用，`..` 一度让「删除存档」拿着
        // user_dir/.. 去 remove_dir_all，整个便携数据目录（全部存档、快照、规则、日志）一次清光。
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        // 老版本入口无校验，profiles.json 里可能已经存着这种名字——本用例走的就是这条来路。
        fs::write(user.join("profiles.json"), r#"{"Profile": {"..": true}}"#).unwrap();
        fs::write(dir.path().join("keepme.txt"), "x").unwrap(); // 数据目录里的「受害者」
        // sanitize 会把 `..` 收敛成 "default"，所以还要盯住同名无辜档的目录不被顺手删掉。
        fs::create_dir_all(user.join("default")).unwrap();
        fs::write(user.join("default").join("graphics_a_snapshot.xml"), "档景").unwrap();

        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        assert!(pm.delete_profile("..").unwrap(), "畸形档应被摘掉名字并返回已删除");

        assert!(dir.path().exists());
        assert!(dir.path().join("keepme.txt").is_file(), "删除 .. 档把数据目录一起删了");
        assert!(
            user.join("default").join("graphics_a_snapshot.xml").is_file(),
            "畸形名收敛成的 default 误伤了另一个存档的目录"
        );
        assert!(!pm.config.profile.contains_key(".."));
    }

    #[test]
    fn create_profile_rejects_names_usable_as_directories() {
        // 畸形名必须在入口就报错，不能靠 sanitize 静默改写（改写后会与别的存档撞同一个目录）。
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        for bad in ["..", ".", "...", "档A."] {
            let e = pm.create_profile(bad).unwrap_err();
            assert!(e.contains("存档名"), "{bad} 的报错文案不符：{e}");
        }
        // 正常名不受影响：中间的点号是合法目录名。
        pm.create_profile("FM24.0").unwrap();
        assert!(pm.config.profile.contains_key("FM24.0"));
        assert!(dir.path().join(".user").join("FM24.0").is_dir());
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

    #[test]
    fn restore_warns_when_profile_has_no_snapshot_but_live_has_content() {
        // 串档现场：切入的档在该目录**没有快照**，而 live 里有内容（属于其他存档）。
        // 以前这里静默跳过、什么也不说（用户只会看到头像没变），现在必须留一条 WARN。
        let pack = tempfile::tempdir().unwrap();
        let img = pack.path().join("pack");
        fs::create_dir_all(&img).unwrap();
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        let (logger, rx) = ChannelLogger::channel();
        pm.set_logger(Arc::new(logger));
        pm.create_profile("档A").unwrap();
        pm.create_profile("档B").unwrap(); // 切到档B：档A 此时还没有组
        pm.load_profile("档A").unwrap();
        pm.ensure_group(img.to_str().unwrap()).unwrap(); // 档A 有组、无快照
        pm.load_profile("档B").unwrap(); // 切走：live 里没有 config.xml → 不产生快照
        fs::write(img.join("config.xml"), "别档留下的内容").unwrap();
        pm.load_profile("档A").unwrap(); // 切回：快照缺失 + live 有内容 → 该报 WARN

        let warns = warns_of(&rx);
        assert_eq!(warns.len(), 1, "应恰好一条 WARN：{warns:?}");
        assert!(warns[0].contains("还没有快照"), "{}", warns[0]);
        // 不恢复、也不删：原主（别档）的档景不被破坏
        assert_eq!(fs::read_to_string(img.join("config.xml")).unwrap(), "别档留下的内容");
    }

    #[test]
    fn restore_warns_when_pack_dir_unavailable() {
        // 换盘 / NAS 离线：快照在、目录不在。以前静默跳过，用户以为档景已恢复（其实 FM 读到的是旧内容）。
        let pack = tempfile::tempdir().unwrap();
        let img = pack.path().join("pack");
        fs::create_dir_all(&img).unwrap();
        fs::write(img.join("config.xml"), "档A的档景").unwrap();
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        let (logger, rx) = ChannelLogger::channel();
        pm.set_logger(Arc::new(logger));
        pm.create_profile("档A").unwrap();
        pm.ensure_group(img.to_str().unwrap()).unwrap();
        pm.create_profile("档B").unwrap(); // 切走：存档A 的快照已存下（此处不该有 WARN）
        assert!(warns_of(&rx).is_empty(), "正常切走不该产生告警");
        fs::remove_dir_all(&img).unwrap(); // 目录变得不可用
        pm.load_profile("档A").unwrap(); // 切回：快照在、目录不在 → 该报 WARN

        let warns = warns_of(&rx);
        assert_eq!(warns.len(), 1, "应恰好一条 WARN：{warns:?}");
        assert!(warns[0].contains("目录不可用"), "{}", warns[0]);
        // 快照必须保留（目录恢复后还要用它）
        assert!(group_xml_path(&dir.path().join(".user"), "档A", &norm_dir(img.to_str().unwrap())).is_file());
    }

    #[test]
    fn load_legacy_profile_json_without_fm_key() {
        // 老副本的组配置只有 rtf 键：缺 fm 必须由 #[serde(default)] 兜底为空串，
        // 既不报错也不丢既有的 rtf（老文件不手工补键）。
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("profiles.json"), r#"{"Profile": {"档A": true}}"#).unwrap();
        fs::write(
            user.join("档A.json"),
            r#"{"cur_group": "C:\\pack", "groups": {"C:\\pack": {"rtf": "C:\\l.rtf"}}}"#,
        )
        .unwrap();
        let pm = ProfileManager::new(Some("档A"), dir.path()).unwrap();
        assert_eq!(pm.get_cur_rtf(), r"C:\l.rtf");
        assert_eq!(pm.get_cur_fm(), "");
    }

    #[test]
    fn v1_single_profile_upgrade_defaults_fm() {
        // v1 单档格式（{"img_dir","rtf"}）升级走 `..Default::default()` 分支：
        // rtf 原样保留，fm 由默认值补空串。
        let dir = data_dir();
        let user = dir.path().join(".user");
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("profiles.json"), r#"{"Profile": {"档旧": true}}"#).unwrap();
        fs::write(
            user.join("档旧.json"),
            r#"{"img_dir": "C:\\pack", "rtf": "C:\\l.rtf"}"#,
        )
        .unwrap();
        let pm = ProfileManager::new(Some("档旧"), dir.path()).unwrap();
        let cfg = pm.prf_cfg.groups.get(&norm_dir(r"C:\pack")).unwrap();
        assert_eq!(cfg.rtf, r"C:\l.rtf");
        assert_eq!(cfg.fm, "");
    }

    #[test]
    fn get_set_cur_fm_persists() {
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        // 无当前组：与 set_cur_rtf 同款语义，返回 Ok(false)、不改任何东西。
        assert!(!pm.set_cur_fm(r"C:\a.fm").unwrap());
        pm.create_profile("档A").unwrap();
        pm.ensure_group(r"C:\pack1").unwrap();
        assert!(pm.set_cur_fm(r"C:\save.fm").unwrap());
        assert_eq!(pm.get_cur_fm(), r"C:\save.fm");
        // 落盘 JSON 必须含 fm 键；重启后回读一致（持久化而非仅内存态）。
        let raw = fs::read_to_string(dir.path().join(".user").join("档A").join("档A.json")).unwrap();
        assert!(raw.contains("\"fm\""), "落盘 JSON 应含 fm 键：{raw}");
        let pm2 = ProfileManager::new(Some("档A"), dir.path()).unwrap();
        assert_eq!(pm2.get_cur_fm(), r"C:\save.fm");
    }

    #[test]
    fn same_dir_in_another_spelling_reuses_the_group() {
        // 同一个目录换种写法（大小写、尾斜杠）不能登记成第二个组：快照名按 Windows 语义
        // 折过大小写，两个组于是共用一份档景——「一份 live 对两份档景」换个入口复发。
        let dir = data_dir();
        let mut pm = ProfileManager::new(None, dir.path()).unwrap();
        pm.create_profile("档A").unwrap();
        let pack = r"C:\fm\graphics\NG";
        assert_eq!(pm.ensure_group(pack), Some(norm_dir(pack)));
        assert_eq!(
            pm.ensure_group(r"C:\FM\Graphics\ng\"),
            Some(norm_dir(pack)),
            "等价写法没有复用已登记的组键"
        );
        assert_eq!(pm.prf_cfg.groups.len(), 1, "同一个目录被登记成了两个组");
    }
}
