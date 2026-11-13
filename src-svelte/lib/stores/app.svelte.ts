/**
 * 全局应用状态与后端事件订阅：前端与后端交互的中枢。
 * appStore 集中持有后端快照与两条进度通道，派生 getter 单点维护核心业务规则，各组件只消费；
 * listen 订阅后端主动 emit 的事件（与命令 invoke 相反），initListeners 由 App 挂载时调用一次。
 */
import { listen } from '@tauri-apps/api/event';
import { api } from '../services/api';
import { t } from '../text';
import type { AppSnapshot, PlayerRecord, ProgressPayload, RtfScanResult, TaskProgressPayload } from '../types';
import { appendLogBatch, clearDisplay, logStore } from './log.svelte';

/** 替换流水线的最终结果（'' = 尚未发生过替换）。事件载荷不含 ''，lastResult 初始值用它。 */
export type ReplaceResult = 'finished' | 'cancelled' | 'failed' | '';

/** 全局应用状态（runes，响应式）。snapshot 由命令返回值整体替换。 */
export const appStore = $state({
  snapshot: null as AppSnapshot | null,        // 后端回传的全局快照（真实激活态；初始 null，启动后填充）
  profilePreview: null as AppSnapshot | null,  // 浏览态预览快照：下拉选档未点「激活」时的展示数据（null=无浏览态）
  selectedProfile: 'No Profile',               // 下拉选中的存档名；≠ snapshot.currentProfile 即处于浏览态
  /** 替换流水线进度通道：状态文字 + 百分比 + 阶段内「第 n 个 / 共 total 个」。
   *  与 task 同形状，StatusBar 才能用同一段计数渲染逻辑渲染两条通道。 */
  progress: { status: '', value: 0, current: 0, total: 0 } as ProgressPayload,
  /** 轻量任务（检查完整性/扫描头像/扫描RTF/读取快照）的进度通道，与替换的 progress 分开、互不干扰。
   *  生命周期由 runTask 管：点击时清空并置任务名，命令成功补 100%；「taskRunning=false 但 value>0」
   *  表示刚跑完，状态栏保留这条 100% 直到 scheduleBarClear 的 20 秒到点（或替换开始时清掉）。 */
  task: { status: '', value: 0, current: 0, total: 0 } as TaskProgressPayload,
  taskRunning: false,                          // 是否有轻量任务在跑：由 runTask 包住命令调用置位
  lastResult: '' as ReplaceResult,             // 最近一次替换结果
  saveBackup: true,                            // 「保存备份」开关：批量与单人替换共用，仅本会话内存活
  rosterVersion: 0,                            // 名单预览版本号：名单缓存填充/清空时 +1，触发 PlayersTable 重载
  /** 最近一次解析名单的摘要（null=手上没有名单）。来源：扫描RTF 成功、替换成功（随 replace:finished 回传）。
   *  名单预览/单人替换据此判断名单格式——拓展格式才有「随机人」筛选，extStaff 时第 15 列是偏好工作。 */
  lastRtfScan: null as RtfScanResult | null,
  rosterReady: false,                          // 后端名单缓存里有没有数据；名单预览据此决定是否渲染工具条
  /** 名单缓存的解析来源：kind 区分 RTF/.fm，path 是扫描当时的名单路径。
   *  路径要记——改 .fm 路径不清缓存（用户可换档不重扫），只有比对路径才能判断缓存是否还属于当前选中的 .fm。
   *  替换收尾不改它：替换消费的是现有缓存，来源不变。 */
  rosterSource: null as { kind: 'rtf' | 'fm'; path: string } | null,
  /** 名单预览的视图过滤：'all' 全部 / 'player' 仅球员 / 'staff' 仅职员。
   *  纯前端视图态：切换只重算过滤，不触发重取、不作废 rosterReady；换代复位见 resetRosterView。 */
  rosterView: 'all' as 'all' | 'player' | 'staff',
  /** 预览区当前 Tab（'table'=名单预览 / 'snapshot'=快照预览）。存档资料台与替换台共用同一份：
   *  在存档台「读取快照」后翻到快照 Tab，切到替换台仍停在快照 Tab（两处状态一致，2026-09-30 用户拍板）。 */
  rtfPreview: 'table' as 'table' | 'snapshot',
  /** 最近一次「读取快照」的快照文件路径（null=尚未读过）。快照预览只认这份：与它一致才拉数据，
   *  切档/切组后自动回引导空态（清零见 applySnapshot）。 */
  snapshotReadPath: null as string | null,
  snapshotVersion: 0,                          // 快照预览版本号：读取快照成功或替换成功后 +1，触发快照预览重载（同路径重读也生效）

  // ---- 展示态派生 getter（核心业务规则单点维护，各组件只消费） ------------------------------------------------------
  /** 展示用快照：浏览态优先，否则真实激活快照。侧栏/当前任务卡/快照预览等展示类组件统一用它。 */
  get displaySnapshot(): AppSnapshot | null {
    return this.profilePreview ?? this.snapshot;
  },
  /** 是否有替换在进行：replacing 是后端全局运行标志，读激活快照即可；各组件统一用它禁用按钮。 */
  get busy(): boolean {
    return !!this.snapshot?.replacing;
  },
  /** 浏览态：下拉选中的存档 ≠ 后端激活档（尚未激活）。展示只读、写激活档的操作应禁用。 */
  get browseMode(): boolean {
    const active = this.snapshot;
    return !!active && this.selectedProfile !== active.currentProfile;
  },
  /** 替换就绪：作用于激活档——「已选 Profile + 已选头像包 + 已设名单（RTF 或 .fm）」三者齐备才允许开始替换。 */
  get replaceReady(): boolean {
    const s = this.snapshot;
    return !!s && s.currentProfile !== 'No Profile' && !!s.currentGroup && !!(s.currentRtf || s.currentFm);
  },
  /** 替换台顶部的缺失项清单：未选 Profile / 未选组 / 未设名单（RTF 或 .fm），缺哪条列哪条文案。
   *  与 replaceReady 同一套条件（数组非空 ⇔ 未就绪），单点维护免得两处口径漂移。 */
  get missingPrerequisites(): string[] {
    const s = this.snapshot;
    return [
      !s || s.currentProfile === 'No Profile' ? t.replace.needProfile : '',
      s && !s.currentGroup ? t.replace.needGroup : '',
      s && !(s.currentRtf || s.currentFm) ? t.replace.needRoster : '',
    ].filter(Boolean);
  },
  /** 第 15 列语义单点：extStaff（拓展职员）名单的「所在地」列实为「偏好工作」。
   *  按后端协议码 formatCode 判，不依赖中文展示文案（展示文案可变，各消费点各 $derived 一份会漂移）。 */
  get isStaffRoster(): boolean {
    return this.lastRtfScan?.formatCode === 'extStaff';
  },
  /** 第 15 列表头文案：职员名单「偏好工作」，球员名单「所在地」。 */
  get basedLabel(): string {
    return this.isStaffRoster ? t.table.colJob : t.table.colBased;
  },
});

/** 第 15 列取值：球员读 based，职员读 preferredJob（另一个在解析侧恒为空串）。 */
export function basedValue(detailed: PlayerRecord['detailed'] | null): string {
  return (appStore.isStaffRoster ? detailed?.preferredJob : detailed?.based) ?? '';
}

/** 人员分组（kind 的行级派生）：staffMale/staffFemale 归「职员」，其余归「球员」。
 *  判据是 types.ts 的 PersonKind 协议值，不从可空的拓展字段反推。 */
export function personKindGroup(r: PlayerRecord): 'player' | 'staff' {
  return r.kind === 'staffMale' || r.kind === 'staffFemale' ? 'staff' : 'player';
}

/** 判定列：该行的名单格式口径（不渲染成表格列），供混合名单下第 15 列逐行取值。 */
export function rowFormatOf(r: PlayerRecord): 'extPlayer' | 'extStaff' {
  return personKindGroup(r) === 'staff' ? 'extStaff' : 'extPlayer';
}

/** 名单视图复位为「全部」（名单预览 onReset 调用，与 newganFilter 复位同款口径）。 */
export function resetRosterView() {
  appStore.rosterView = 'all';
}

/** 名单搜索口径（名单预览表格 / 单人替换卡共用这一份实现）：
 *  UID、姓名、国籍、俱乐部任一包含搜索词即命中，大小写不敏感；
 *  俱乐部只有拓展名单有（基础名单为空串，不参与匹配）；搜索词为空视为全部命中。 */
export function matchPlayer(r: PlayerRecord, word: string): boolean {
  const w = word.trim().toLowerCase();
  if (!w) return true;
  return (
    r.uid.toLowerCase().includes(w) ||
    r.name.toLowerCase().includes(w) ||
    r.primaryNat.toLowerCase().includes(w) ||
    (r.detailed?.club ?? '').toLowerCase().includes(w)
  );
}

/** 过滤出命中的记录（保持入参顺序）；搜索词为空时原样返回。 */
export function filterPlayers(rows: readonly PlayerRecord[], word: string): PlayerRecord[] {
  return word.trim() ? rows.filter((r) => matchPlayer(r, word)) : [...rows];
}

/** 悬停提示：样式见 app.css 的 .tip（虚线描边 + 页面底色）。文本为空/缺省不挂载。
 *  用自绘气泡替代原生 title 的理由：原生气泡不跟随主题、不能换行排版，也跟不上滚动。
 *  挂载时删掉 node 上的原生 title——两套提示同时出现会读成两条互相矛盾的信息。 */
export function tip(node: HTMLElement, text?: string | null) {
  /** 视口边距（px）：气泡不许贴边，离屏幕边缘至少留这么宽。 */
  const EDGE = 8;
  /** 与元素的间距（px）。 */
  const GAP = 6;
  let bubble: HTMLDivElement | null = null;

  /** 落位：默认贴元素下方左对齐；下方空间不够翻到上方；左右夹取到视口内。 */
  function place(el: HTMLDivElement) {
    const host = node.getBoundingClientRect();
    const box = el.getBoundingClientRect();
    let top = host.bottom + GAP;
    if (top + box.height > window.innerHeight - EDGE) {
      const above = host.top - GAP - box.height;
      top = above >= EDGE ? above : Math.max(EDGE, window.innerHeight - EDGE - box.height);
    }
    const maxLeft = window.innerWidth - EDGE - box.width;
    const left = Math.min(Math.max(EDGE, host.left), Math.max(EDGE, maxLeft));
    el.style.top = `${Math.round(top)}px`;
    el.style.left = `${Math.round(left)}px`;
  }

  function hide() {
    if (!bubble) return;
    bubble.remove();
    bubble = null;
  }

  function show() {
    if (!text?.trim() || bubble) return;
    const el = document.createElement('div');
    el.className = 'tip';
    el.textContent = text;
    document.body.appendChild(el);
    place(el); // 先量后落位：插入后同一帧内量得到尺寸，不会先闪在默认位置
    bubble = el;
  }

  // 滚动（capture，能收到任意内层滚动容器的）与窗口失焦都要收起：气泡是 fixed 定位，
  // 不会跟着元素走，留着就脱锚指错了地方。
  function onScroll() {
    hide();
  }

  node.removeAttribute('title');
  node.addEventListener('mouseenter', show);
  node.addEventListener('mouseleave', hide);
  node.addEventListener('blur', hide);
  window.addEventListener('scroll', onScroll, true);
  window.addEventListener('blur', hide);

  return {
    /** 文本变了：先收起再等下次悬停按新文本重建（不做原地改字，省一套重排逻辑）。 */
    update(next?: string | null) {
      text = next;
      hide();
    },
    destroy() {
      hide();
      node.removeEventListener('mouseenter', show);
      node.removeEventListener('mouseleave', hide);
      node.removeEventListener('blur', hide);
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('blur', hide);
    },
  };
}

/** 主动拉一次最新快照（调后端 get_app_state）。 */
export async function refreshSnapshot() {
  await applySnapshot(await api.getAppState());
}

/** 直接用命令返回的快照替换本地状态（省一次 get_app_state 往返）。
 *  【作用域变化要复位派生标记】切档/切组/换名单时后端会清空名单缓存但只回传快照，
 *  缓存作废只能靠这几个量比对出来；不复位，名单预览会继续显示上一个档/组的名单。 */
export async function applySnapshot(s: AppSnapshot) {
  const prev = appStore.snapshot;
  const scopeChanged =
    !!prev &&
    (prev.currentProfile !== s.currentProfile ||
      prev.currentGroup !== s.currentGroup ||
      prev.currentRtf !== s.currentRtf);
  if (scopeChanged && prev) {
    appStore.rosterReady = false; // 后端名单缓存已作废 → 名单预览回引导空态
    appStore.lastRtfScan = null;  // 摘要「是这份名单解析出来的」，名单换了就作废
    appStore.rosterSource = null; // 来源随摘要一起作废，留着会让状态点认成「当前 .fm 已扫描」
    appStore.rosterVersion++;     // 触发名单预览按清空后的空态重画
    // 快照读态跟「档+组」走、不跟 RTF 走（快照文件是档+组的 config.xml 副本，换 RTF 不作废，
    // 清零会白白把芯片打回琥珀）；但换档/组后旧路径必不属于当前作用域，不清会跨组虚绿。
    if (
      prev.currentProfile !== s.currentProfile ||
      prev.currentGroup !== s.currentGroup
    ) {
      appStore.snapshotReadPath = null;
    }
  }
  appStore.snapshot = s;
}

/** 「后端名单缓存已填充」的单点标记（扫描 RTF/.fm 成功、替换成功三处调用）。
 *  source 只有扫描调用传：替换收尾沿用扫描时记下的来源，不覆盖。
 *  失败/取消**不调**：后端不把缓存写回空表（作用域没变），保留旧摘要与预览，省用户重扫一遍。 */
export function markRosterFilled(
  scan: RtfScanResult | null,
  source?: { kind: 'rtf' | 'fm'; path: string },
) {
  appStore.lastRtfScan = scan;
  if (source) appStore.rosterSource = source;
  appStore.rosterReady = true;
  appStore.rosterVersion++; // 触发名单预览重载（旧查询结果已过期）
}

/** 「快照已读入缓存」的单点标记（读取快照成功、替换成功两处调用）；失败/取消不改 config.xml，不调。 */
export function markSnapshotRead(xmlPath: string) {
  appStore.snapshotReadPath = xmlPath;
  appStore.snapshotVersion++; // 同路径重读也生效，触发快照预览重载
}

/** 跑完的进度条在状态栏保留多久（毫秒）：留出看得见 100% 的时间，到点自动清回「就绪」。 */
export const BAR_LINGER_MS = 20_000;

// 待清空的定时器（普通模块变量，非响应式）。同一时刻只留一个：后来的任务/替换会重排。
let barClearTimer: ReturnType<typeof setTimeout> | null = null;

/** 撤掉待清空的定时器（新任务开始、替换推进、已清空时调用）。 */
function cancelBarClear() {
  if (barClearTimer !== null) {
    clearTimeout(barClearTimer);
    barClearTimer = null;
  }
}

/** 安排 BAR_LINGER_MS 后清空状态栏两条进度；重复调用只保留最后一次。 */
function scheduleBarClear() {
  cancelBarClear();
  barClearTimer = setTimeout(() => {
    barClearTimer = null;
    // 这 20 秒里用户可能又点了扫描或开始了替换：状态栏归它，别把新进度抹掉
    if (appStore.taskRunning || appStore.snapshot?.replacing) return;
    appStore.task = { status: '', value: 0, current: 0, total: 0 };
    appStore.progress = { status: '', value: 0, current: 0, total: 0 };
  }, BAR_LINGER_MS);
}

/** 包一层「轻量任务」：命令执行期间状态栏显示其进度（进度值由 task:progress 事件写入 appStore.task），
 *  四个扫描按钮都经它调用。点击先清空 task：后端读完文件才发第一条进度，不清会闪出上一个任务名。 */
export async function runTask<T>(label: string, fn: () => Promise<T>): Promise<T> {
  cancelBarClear(); // 新任务接管状态栏
  appStore.task = { status: label, value: 0, current: 0, total: 0 };
  appStore.taskRunning = true;
  try {
    const out = await fn();
    // 小包/空包几毫秒就返回，后端来不及上报——按它的口径补一个 100%，否则进度条一闪而过。
    // 放 try 里：命令抛错时保持 0%，别谎报完成。
    appStore.task = { ...appStore.task, value: 100, current: appStore.task.total };
    return out;
  } finally {
    appStore.taskRunning = false; // 失败也要复位，否则状态栏一直卡在任务态
    scheduleBarClear(); // 收尾排一次清空（失败也排：顺手收掉上一次替换的遗留进度）
  }
}

// inited 保证 initListeners 只真正执行一次。
let inited = false;

/** 从 Rust 环形缓冲回填当前过滤条件下的日志历史。 */
export async function reloadLogs() {
  const page = await api.queryLogs(logStore.level, logStore.only, 0, 10_000);
  clearDisplay();               // 先清空旧的显示
  logStore.lines.push(...page.items); // 再填入后端返回的历史日志
}

/** 订阅后端事件（App 挂载时调用一次）。onFinished 是替换结束时的回调。 */
export async function initListeners(onFinished: (r: ReplaceResult) => Promise<void>) {
  if (inited) return; // 防止重复订阅
  inited = true;
  await refreshSnapshot(); // 先拉一次全局快照，界面立即有数据可渲染
  // 启动阶段的 log:batch 在 WebView 导航开始前就已 emit、前端收不到，故先回填历史再订阅增量（顺序反了会重复行）。
  await reloadLogs();
  // 替换进度：后端每进入一个阶段（及阶段内每推进一项）emit 一次，存进 appStore.progress。
  await listen<ProgressPayload>('replace:progress', (e) => {
    appStore.progress = e.payload;
    cancelBarClear(); // 替换推进接管状态栏，别让上次排的清空把替换进度抹掉
    // 替换一开始推进就清掉轻量任务留下的 100% 收尾态，否则取消后状态栏显示的是旧任务的 100% 而非替换真实百分比
    if (appStore.task.value > 0) {
      appStore.task = { status: '', value: 0, current: 0, total: 0 };
    }
  });
  // 轻量任务进度：与 replace:progress 分开存，两条通道同时存在互不覆盖。
  await listen<TaskProgressPayload>('task:progress', (e) => {
    appStore.task = e.payload;
  });
  // 替换完成：记结果、刷新快照（replacing 变 false）、按需打缓存标记、排进度清空、回调弹窗。
  await listen<{
    result: Exclude<ReplaceResult, ''>;
    roster: RtfScanResult | null;
    source?: { kind: 'rtf' | 'fm'; path: string } | null;
  }>('replace:finished', async (e) => {
    appStore.lastResult = e.payload.result;
    // 「缓存已填充」标记只在成功时打：失败/取消保留替换前的摘要与预览，用户不必重扫（详见 markRosterFilled）。
    // source 仅在本次线程内自动扫描时非空：补上来源标记；命中缓存时为 null，沿用扫描时记下的那份。
    if (e.payload.result === 'finished') {
      markRosterFilled(e.payload.roster, e.payload.source ?? undefined);
    }
    await refreshSnapshot();
    if (e.payload.result === 'finished') {
      const xmlPath = appStore.snapshot?.configXml?.path;
      if (xmlPath) markSnapshotRead(xmlPath);
    }
    scheduleBarClear(); // 跑完（成功/失败/取消都一样）把进度条留 20 秒再清回「就绪」
    await onFinished(e.payload.result);
  });
  // 日志批量事件：新日志追写到前端列表。
  await listen<import('../types').LogLine[]>('log:batch', (e) => {
    appendLogBatch(e.payload);
  });
}
