/**
 * 全局应用状态与后端事件订阅：前端与后端交互的中枢。
 * listen 订阅后端 emit 的事件（与命令 invoke 相反：事件由后端主动推送）。
 *
 * 状态用 $state 声明；命令返回的快照整体替换 appStore.snapshot。
 * 事件订阅是进程级副作用，initListeners 由 App 挂载时调用一次（内部用 inited 防重复）。
 */
import { listen } from '@tauri-apps/api/event';
import { api } from '../api';
import type { AppSnapshot, ProgressPayload, RtfScanResult, TaskProgressPayload } from '../types';
import { appendLogBatch, clearDisplay, logStore } from './log.svelte';

/** 替换流水线的最终结果（'' = 尚未发生过替换）。事件载荷不含 ''，lastResult 初始值用它。 */
export type ReplaceResult = 'finished' | 'cancelled' | 'failed' | '';

/** 全局应用状态（runes，响应式）。snapshot 由命令返回值整体替换。 */
export const appStore = $state({
  snapshot: null as AppSnapshot | null, // 后端回传的全局快照（真实激活态；初始为 null，启动后填充）
  /** 浏览态预览快照：下拉选择存档但未点「激活」时的展示数据（null = 无浏览态）。
   *  展示类组件（侧栏/当前任务卡）优先用它；操作类组件（替换执行等）始终用 snapshot。 */
  profilePreview: null as AppSnapshot | null,
  /** 下拉选中的存档名（浏览态）。与 snapshot.currentProfile 不同 = 正在浏览未激活的存档。 */
  selectedProfile: 'No Profile',
  /** 当前替换进度（替换流水线通道）：状态文字 + 百分比 + 阶段内的「第 n 个 / 共 total 个」。
   *  形状与下面的 task 一致，StatusBar 才能对两条通道用同一段计数渲染逻辑。 */
  progress: { status: '', value: 0, current: 0, total: 0 } as ProgressPayload,
  /** 「检查完整性 / 扫描头像 / 扫描RTF / 读取快照」四个轻量任务的实时进度：状态文字 + 百分比 +
   *  「第 n 个 / 共 total 个」。与上面的 progress（替换流水线）分两条通道，互不干扰。
   *  生命周期由 runTask 管：点击时清成 `{status: 任务名, value: 0}`，命令成功后补 value=100
   *  ——所以「taskRunning 为 false 但 value>0」表示「刚跑完」，状态栏会保留这条 100%
   *  直到 scheduleBarClear 排的 20 秒到点；替换一开始（replace:progress）也会把它清掉。 */
  task: { status: '', value: 0, current: 0, total: 0 } as TaskProgressPayload,
  /** 是否有轻量任务在跑：由 runTask 包住命令调用置位。状态栏据此优先显示 task 进度。 */
  taskRunning: false,
  lastResult: '' as ReplaceResult, // 最近一次替换结果
  /** 「保存备份」开关：批量替换与单人微调共用（两处写 config.xml 前是否先备份）。
   *  只在本会话内存活（mode 等其余替换选项也都是组件内会话态），不做持久化。 */
  saveBackup: true,
  rosterVersion: 0,                     // 名单预览版本号：名单缓存被填充/清空时 +1（「扫描RTF」成功、替换结束），触发 PlayersTable 重载
  /** 最近一次解析名单的摘要（null=手上没有名单）。两条来源：「扫描RTF」成功、替换成功
   *  （摘要随 replace:finished 回传，无需用户再点一次扫描）。名单预览/单人微调据此判断名单格式
   *  ——拓展格式才有「随机人」筛选，且 formatCode 为 extStaff 时第 15 列是偏好工作。 */
  lastRtfScan: null as RtfScanResult | null,
  /** 名单缓存可读标记：后端名单缓存里有没有数据。对应快照预览的 snapshotReadPath。
   *  两处会填充后端缓存：「扫描RTF」成功 → true；替换结束 → 成功为 true、失败/取消为 false
   *  （后端失败/取消时会把缓存写回空表）。名单预览据此决定是否渲染工具条。 */
  rosterReady: false,
  /** 结果区当前 Tab：'table'=名单预览 / 'viewer'=单人微调 / 'snapshot'=快照预览。
   *  放全局态：「读取快照」成功后要跨组件（当前任务卡 → 结果区）切换到快照预览。 */
  resultsView: 'table' as 'table' | 'viewer' | 'snapshot',
  /** 最近一次「读取快照」的快照文件路径（null=尚未读取过）。
   *  快照预览只认这份：当前展示的快照路径与它一致才拉数据，切档/切组后自动回到引导空态。
   *  两处会置它：「读取快照」成功；替换成功（后端把新 config.xml 同步到该档快照位置并刷新缓存）。 */
  snapshotReadPath: null as string | null,
  /** 快照预览版本号：「读取快照」成功、或替换成功（后端同步快照）后 +1，
   *  触发 SnapshotPreview 重载（同路径重读也生效）。 */
  snapshotVersion: 0,

  // ---- 展示态派生 getter（核心业务规则单点维护，各组件只消费，不再各自 $derived 一份）----
  /** 展示用快照：浏览态优先（下拉预览未激活时用预览快照），否则用后端真实激活快照。
   *  展示类组件（侧栏/当前任务卡/快照预览）用它取 currentGroup/currentRtf/configXml/groups。 */
  get displaySnapshot(): AppSnapshot | null {
    return this.profilePreview ?? this.snapshot;
  },
  /** 是否有替换正在进行：replacing 是后端全局运行标志（snapshot 与 preview 回传的都是同一个
   *  state.running），故直接读激活快照即可，展示/操作类组件统一用它禁用按钮。 */
  get busy(): boolean {
    return !!this.snapshot?.replacing;
  },
  /** 浏览态：下拉选中的存档 ≠ 后端当前激活档（选中项尚未激活）。展示只读、写激活档的操作应禁用。 */
  get browseMode(): boolean {
    const active = this.snapshot;
    return !!active && this.selectedProfile !== active.currentProfile;
  },
  /** 替换就绪：作用于后端激活档——「已选 Profile + 已选头像包 + 已设 RTF」三者齐备才允许开始替换。 */
  get replaceReady(): boolean {
    const s = this.snapshot;
    return !!s && s.currentProfile !== 'No Profile' && !!s.currentGroup && !!s.currentRtf;
  },
});

/** 主动拉一次最新快照（调后端 get_app_state）。 */
export async function refreshSnapshot() {
  appStore.snapshot = await api.getAppState();
}

/** 直接用命令返回的快照替换本地状态（避免多一次 get_app_state 往返）。 */
export async function applySnapshot(s: AppSnapshot) {
  appStore.snapshot = s;
}

/** 跑完的进度条在状态栏上保留多久（毫秒）：留一会儿让「100%」看得见，到点自动清空回纯「就绪」。
 *  【为什么要清】不清的话，四个扫描任务和替换留下的 100% 会一直挂在状态栏上，看着像还在忙。 */
export const BAR_LINGER_MS = 20_000;

// 待清空的定时器（普通模块变量，非响应式）。同一时刻只留一个有效的：后来的任务/替换会重排。
let barClearTimer: ReturnType<typeof setTimeout> | null = null;

/** 撤掉待清空的定时器。新任务开始、替换推进、或已经清空时调用。 */
function cancelBarClear() {
  if (barClearTimer !== null) {
    clearTimeout(barClearTimer);
    barClearTimer = null;
  }
}

/** 安排「BAR_LINGER_MS 后清空状态栏的两条进度」。重复调用只保留最后一次（先撤掉上一个）。 */
function scheduleBarClear() {
  cancelBarClear();
  barClearTimer = setTimeout(() => {
    barClearTimer = null;
    // 这 20 秒里用户可能又点了一次扫描、或开始了替换：那时状态栏归它们，别把新进度抹掉。
    if (appStore.taskRunning || appStore.snapshot?.replacing) return;
    appStore.task = { status: '', value: 0, current: 0, total: 0 };
    appStore.progress = { status: '', value: 0, current: 0, total: 0 };
  }, BAR_LINGER_MS);
}

/** 包一层「轻量任务」：命令执行期间状态栏显示它的进度（进度值由 task:progress 事件写入 appStore.task）。
 *  「检查完整性/扫描头像/扫描RTF/读取快照」四个按钮都经它调用，label 是状态栏上要显示的任务名。
 *  【为何点击时先清空 task】后端要读完文件、数出分母才发第一条 task:progress（扫描RTF/读取快照可达上百
 *  毫秒），不清空的话状态栏会先闪出上一个任务的名称与计数。 */
export async function runTask<T>(label: string, fn: () => Promise<T>): Promise<T> {
  cancelBarClear(); // 新任务接管状态栏：撤掉上一次排的「20 秒后清空」
  appStore.task = { status: label, value: 0, current: 0, total: 0 };
  appStore.taskRunning = true;
  try {
    const out = await fn();
    // 小包/空包整个命令只有几毫秒，后端来不及上报（或只报到中途）——这里按它的口径补一个 100%
    // （current 补齐到 total），否则进度条一闪而过、用户等于没看到反馈。后端已 finish() 上报过 100 的，
    // 这里写的是同样的值。放在 try 里：命令抛错时保持 0%，别谎报完成。
    appStore.task = { ...appStore.task, value: 100, current: appStore.task.total };
    return out;
  } finally {
    appStore.taskRunning = false; // 失败也要复位，否则状态栏一直卡在任务态
    scheduleBarClear(); // 收尾排一次清空（失败也排：顺手把上一次替换遗留的进度一起收掉）
  }
}

// inited 是一个普通模块变量（非响应式），用于保证 initListeners 只真正执行一次。
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
  // 启动阶段的 log:batch（包含 “Starting Application v2”）在 WebView 导航开始前就已 emit，
  // 前端无法收到，因此先回填历史再订阅增量（顺序反了会出现重复行）。
  await reloadLogs();
  // 订阅进度事件：后端每进入一个阶段（以及阶段内每推进一项）就 emit，这里把载荷存进 appStore.progress。
  await listen<ProgressPayload>('replace:progress', (e) => {
    appStore.progress = e.payload;
    cancelBarClear(); // 替换开始推进：撤掉上一次任务排的「20 秒后清空」，别把替换进度清掉
    // 替换流水线一开始推进，就把轻量任务留在状态栏上的收尾态（100%）清掉：否则替换被取消后，
    // 状态栏会退回显示那个旧任务的 100%，而不是替换被中断时的真实百分比。
    // 加 value>0 的判断只是为了别在每个进度事件上都新建对象（状态栏此刻正在读 progress，与 task 无关）。
    if (appStore.task.value > 0) {
      appStore.task = { status: '', value: 0, current: 0, total: 0 };
    }
  });
  // 订阅轻量任务进度：「检查完整性/扫描头像/扫描RTF/读取快照」执行期间实时刷新状态栏进度条。
  // 与 replace:progress 分开存（appStore.task），两条通道同时存在也互不覆盖。
  await listen<TaskProgressPayload>('task:progress', (e) => {
    appStore.task = e.payload;
  });
  // 订阅完成事件：记下结果、刷新快照（replacing 会变 false）、调用回调弹窗提示。
  await listen<{ result: Exclude<ReplaceResult, ''>; roster: RtfScanResult | null }>(
    'replace:finished',
    async (e) => {
      appStore.lastResult = e.payload.result;
      // 名单摘要随事件回传（后端替换时已经解析过名单）：前端不必再点一次「扫描RTF」就知道
      // 名单格式；失败/取消时为 null——后端已把名单缓存写回空表，摘要一并清掉。
      appStore.lastRtfScan = e.payload.roster;
      // 替换流程结束也会写后端名单缓存（见 commands.rs 的 start_replace）：
      // 成功 = 填上这份名单 → 名单预览可读；失败/取消 = 写回空表 → 回到引导空态。
      // 这两个标记必须在 refreshSnapshot 之前置好：名单预览要立刻按新缓存重载一次。
      appStore.rosterReady = e.payload.result === 'finished';
      appStore.rosterVersion++; // 触发名单预览重载（替换前的查询结果已过期）
      await refreshSnapshot();
      // 快照预览同样有两条写缓存路径（「读取快照」按钮 / 替换收尾的自动同步）：
      // 替换成功时后端已把新 config.xml 同步到该档快照位置并刷新快照缓存，
      // 这里补上与 SetupCard::readSnapshot 相同的两个标记——记下已读路径 + 触发预览重载。
      // 失败/取消不改 config.xml（快照保持原样），因此不动这两个标记。
      if (e.payload.result === 'finished') {
        const xmlPath = appStore.snapshot?.configXml?.path;
        if (xmlPath) {
          appStore.snapshotReadPath = xmlPath;
          appStore.snapshotVersion++;
        }
      }
      // 替换跑完（成功/失败/取消都一样）：把进度条在状态栏上留 20 秒再清空回「就绪」。
      scheduleBarClear();
      await onFinished(e.payload.result);
    },
  );
  // 订阅日志批量事件：把新日志追写到前端日志列表。
  await listen<import('../types').LogLine[]>('log:batch', (e) => {
    appendLogBatch(e.payload);
  });
}
