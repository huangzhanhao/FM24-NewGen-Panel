//! Tauri 命令层：对等 services/* + app.py。数据留在 Rust，IPC 只传摘要/分页
use crate::assets::ETH_CFG;
use crate::face_mapper::Mode;
use crate::logger::{LogPage, Logger};
use crate::paths::norm_dir;
use crate::profile_manager::{GlobalCfg, GroupCfg, ProfileManager};
use crate::replace_flow::{self, Outcome, ReplaceParams};
use crate::rtf_parser::PlayerRecord;
use crate::xml_parser::{self, MappingEntry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

/// 14 个种族目录（与 v1 facepack_dirs 一致，含历史拼写 Seasian）
pub const FACEPACK_DIRS: [&str; 14] = [
    "African", "Asian", "Caucasian", "Central European", "EECA", "Italmed",
    "MENA", "MESA", "SAMed", "Scandinavian", "Seasian", "South American",
    "SpanMed", "YugoGreek",
];

pub struct AppState {
    pub pm: Mutex<ProfileManager>,
    pub logger: Arc<Logger>,           // Logger 字段全部是 Mutex/Sender，包 Arc 即可跨线程
    pub cancel: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    pub rtf_cache: Arc<Mutex<Vec<PlayerRecord>>>,
    pub mapping_cache: Arc<Mutex<Vec<MappingEntry>>>,
}

// ---------------- 传给前端的类型（统一 camelCase） ----------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo { pub img_dir: String, pub rtf: String }

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub profiles: Vec<String>,
    pub current_profile: String,
    pub groups: Vec<GroupInfo>,
    pub current_group: Option<String>,
    pub current_rtf: String,
    pub replacing: bool,
    pub log_path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPayload { pub status: String, pub value: u8 }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceArgs {
    pub mode: String,
    pub filter_newgan: bool,
    pub allow_duplicates: bool,
    pub save_backup: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerQuery {
    pub offset: usize,
    pub limit: usize,
    pub search: String,
    pub sort_key: String,
    pub sort_desc: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageResult<T> { pub total: usize, pub items: Vec<T> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPreview {
    pub record: Option<PlayerRecord>,
    pub mapping: Option<MappingEntry>,
    pub image_path: Option<String>,
}

// ---------------- 命令实现 ----------------

fn snapshot(state: &State<'_, AppState>) -> AppSnapshot {
    let pm = state.pm.lock().unwrap();
    AppSnapshot {
        profiles: pm.config.profile.keys().cloned().collect(),
        current_profile: pm.cur_prf.clone(),
        groups: pm
            .prf_cfg
            .groups
            .iter()
            .map(|(dir, g)| GroupInfo { img_dir: dir.clone(), rtf: g.rtf.clone() })
            .collect(),
        current_group: pm.cur_group.clone(),
        current_rtf: pm.get_cur_rtf(),
        replacing: state.running.load(Ordering::SeqCst),
        log_path: state.logger.path().to_string_lossy().to_string(),
    }
}

#[tauri::command]
pub fn get_app_state(state: State<'_, AppState>) -> AppSnapshot { snapshot(&state) }

#[tauri::command]
pub fn create_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    state.pm.lock().unwrap().create_profile(&name)?;
    state.logger.info("profile", format!("创建并切换 Profile: {name}"));
    Ok(snapshot(&state))
}

#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.delete_profile(&name)? {
        return Err("Can't delete 'No Profile'".into());
    }
    drop(pm);
    Ok(snapshot(&state))
}

#[tauri::command]
pub fn switch_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    pm.load_profile(&name)?;
    crate::json_store::save(
        &pm.user_cfg_path().join("cfg.json"),
        &pm.config,
    )?;
    drop(pm);
    state.logger.info("profile", format!("切换 Profile: {name}"));
    Ok(snapshot(&state))
}

#[tauri::command]
pub fn select_image_directory(state: State<'_, AppState>, path: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if pm.cur_prf == "No Profile" {
        return Err("请先创建并选择一个 Profile".into());
    }
    pm.ensure_group(&path).ok_or("目录不可用")?;
    drop(pm);
    Ok(snapshot(&state))
}

#[tauri::command]
pub fn set_current_rtf(state: State<'_, AppState>, path: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.set_cur_rtf(&path)? {
        return Err("请先选择头像包目录（当前无组）".into());
    }
    drop(pm);
    Ok(snapshot(&state))
}

#[tauri::command]
pub fn set_current_group(state: State<'_, AppState>, img_dir: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.set_cur_group(&img_dir)? {
        return Err("组不存在".into());
    }
    drop(pm);
    Ok(snapshot(&state))
}

#[tauri::command]
pub fn delete_group(state: State<'_, AppState>, img_dir: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.delete_group(&img_dir)? {
        return Err("组不存在".into());
    }
    drop(pm);
    Ok(snapshot(&state))
}

/// 校验头像包目录，返回缺失的种族子目录名（前端弹一次确认框后批量创建）
#[tauri::command]
pub fn validate_image_dirs(img_dir: String) -> Vec<String> {
    let existing: std::collections::HashSet<String> = std::fs::read_dir(&img_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    FACEPACK_DIRS
        .iter()
        .filter(|d| !existing.contains(**d))
        .map(|d| d.to_string())
        .collect()
}

#[tauri::command]
pub fn create_missing_dirs(img_dir: String, names: Vec<String>) -> Result<(), String> {
    for name in names {
        std::fs::create_dir_all(Path::new(&img_dir).join(&name))
            .map_err(|e| format!("创建目录 {name} 失败: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn start_replace(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ReplaceArgs,
) -> Result<(), String> {
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("已有替换任务在执行".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let (rtf, img_dir) = {
        let pm = state.pm.lock().unwrap();
        (pm.get_cur_rtf(), pm.cur_group.clone())
    };
    let Some(img_dir) = img_dir else {
        state.running.store(false, Ordering::SeqCst);
        return Err("请先选择头像包目录".into());
    };
    if rtf.is_empty() {
        state.running.store(false, Ordering::SeqCst);
        return Err("请先选择 RTF 名单".into());
    }
    let mode = Mode::parse(&args.mode)?;
    let params = ReplaceParams {
        rtf_path: rtf,
        img_dir,
        mode,
        filter_newgen: args.filter_newgan,
        allow_duplicates: args.allow_duplicates,
        save_backup: args.save_backup,
    };
    let (cancel, running) = (state.cancel.clone(), state.running.clone());
    let (rtf_cache, mapping_cache) = (state.rtf_cache.clone(), state.mapping_cache.clone());
    let logger = state.logger.clone();
    std::thread::spawn(move || {
        let emit_app = app.clone();
        let result = replace_flow::run(
            &params,
            &mut |status: &str, value: u8| {
                let _ = emit_app.emit("replace:progress", ProgressPayload { status: status.into(), value });
            },
            &cancel,
        );
        *rtf_cache.lock().unwrap() = result.rtf_data.clone();
        *mapping_cache.lock().unwrap() = result.mapping.clone();
        let result_str = match result.outcome {
            Outcome::Finished => "finished",
            Outcome::Cancelled => "cancelled",
            Outcome::Failed(ref m) => { logger.error("replace", m.clone()); "failed" }
        };
        let _ = app.emit("replace:finished", serde_json::json!({ "result": result_str }));
        running.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// 单人换脸（对等 PlayerService.replace_single_face）
#[tauri::command]
pub fn replace_single_face(
    state: State<'_, AppState>,
    uid: String,
    image_path: String,
    save_backup: bool,
) -> Result<bool, String> {
    let uid = uid.trim().to_string();
    let Some((pack, image)) = xml_parser::parse_image_path(&image_path) else {
        return Err("图片路径不合法，需形如 ...\\<种族目录>\\<图片名>.png".into());
    };
    let img_dir = {
        let pm = state.pm.lock().unwrap();
        pm.cur_group.clone().ok_or("当前没有选中组")?
    };
    let hit = xml_parser::single_replacement(&uid, &pack, &image, Path::new(&img_dir), save_backup)?;
    if hit {
        // 同步内存映射缓存（对等 update_mapping_cache）
        let mut cache = state.mapping_cache.lock().unwrap();
        for m in cache.iter_mut() {
            if m.uid == uid {
                m.ethnicity = pack.clone();
                m.image = image.clone();
                break;
            }
        }
    }
    state.logger.info("viewer", format!("单人换脸 {uid} → {pack}/{image}"));
    Ok(hit)
}

/// 查询/预览（对等 PlayerService 查询链）
#[tauri::command]
pub fn preview_player(state: State<'_, AppState>, uid: String) -> Result<PlayerPreview, String> {
    let uid = uid.trim().to_string();
    let variants = xml_parser::with_newgan_prefix_variants(&uid);
    // 1) 最近一次批量映射缓存
    let mut mapping = {
        let cache = state.mapping_cache.lock().unwrap();
        cache.iter().find(|m| variants.contains(&m.uid)).cloned()
    };
    let pm = state.pm.lock().unwrap();
    // 2) 当前组 config.xml
    if mapping.is_none() {
        if let Some(g) = &pm.cur_group {
            if let Ok(xml) = xml_parser::parse_xml(&Path::new(g).join("config.xml")) {
                mapping = variants.iter().find_map(|u| xml.get(u).cloned());
            }
        }
    }
    // 3) 解析实际图片文件路径
    let image_path = mapping.as_ref().and_then(|m| {
        let g = pm.cur_group.as_ref()?;
        xml_parser::resolve_image_file(Path::new(g), &m.ethnicity, &m.image)
    });
    // 4) RTF 球员资料缓存
    let record = {
        let cache = state.rtf_cache.lock().unwrap();
        cache.iter().find(|p| variants.contains(&p.uid)).cloned()
    };
    Ok(PlayerPreview { record, mapping, image_path })
}

/// RTF 名单分页查询（大名单预览；数据留 Rust，IPC 只传当前页）
#[tauri::command]
pub fn query_players(state: State<'_, AppState>, query: PlayerQuery) -> PageResult<PlayerRecord> {
    let mut rows = state.rtf_cache.lock().unwrap().clone();
    let search = query.search.to_lowercase();
    if !search.is_empty() {
        rows.retain(|r| {
            r.uid.to_lowercase().contains(&search)
                || r.name.to_lowercase().contains(&search)
                || r.primary_nat.to_lowercase().contains(&search)
        });
    }
    rows.sort_by(|a, b| {
        let ord = match query.sort_key.as_str() {
            "name" => a.name.cmp(&b.name),
            "primaryNat" => a.primary_nat.cmp(&b.primary_nat),
            "ethnicityCode" => a.ethnicity_code.cmp(&b.ethnicity_code),
            _ => a.uid.cmp(&b.uid),
        };
        if query.sort_desc { ord.reverse() } else { ord }
    });
    let total = rows.len();
    PageResult { total, items: rows.into_iter().skip(query.offset).take(query.limit).collect() }
}

/// 读取预览图片字节（前端转 blob URL 显示）
#[tauri::command]
pub fn read_image(path: String) -> Result<tauri::ipc::Response, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取图片失败: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub fn query_logs(state: State<'_, AppState>, level: String, only: bool, offset: usize, limit: usize) -> LogPage {
    state.logger.query(&level, only, offset, limit)
}

#[tauri::command]
pub fn cancel_replace(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
    state.logger.info("replace", "用户请求取消");
}
