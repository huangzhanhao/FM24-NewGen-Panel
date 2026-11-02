<!-- RuleEditor · 规则配置工作台：把 data/rules/standard2|standard3.json 从纯文本编辑升级为结构化交互。
     左侧分区导航 + 右侧分节编辑器：目录清单 / 国籍映射 / 种族码修正（标准二）/ 规则块（标准三）/ 全局设置。
     顶部可切标准、切「源码」高级模式；保存仍走原命令，后端按各标准 schema 做权威校验。
     切换标准、关闭与进入源码前都会检查未保存修改。
     【拆分建议】本组件约 2500 行（导航 + 六个分区 + 源码模式 + 样式）。
     判定逻辑已收进 rules.ts，组件只剩交互与绑定；继续增长时把分区拆成子组件（FoldersPane / EthnicsPane / BlocksPane），
     共享状态用 props + 回调，别再往这个文件塞新分区。 -->
<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text";
  import { appStore, refreshSnapshot, tip } from "../../stores/app.svelte";
  import Icon from "../../ui/Icon.svelte";
  import {
    RACE_CODES,
    blockKind,
    directDir,
    folderUsageMap,
    formatDoc,
    healthIssues,
    mappingKind,
    parseRuleFile,
    randomDirs,
    ruleDirs,
    sortedKeys,
    weightedList,
    type HealthIssue,
    type MappingKind,
    type MappingRule,
    type RuleStandard,
    type S2Doc,
    type S3Doc,
  } from "../../rules";

  type SectionId = "overview" | "folders" | "ethnics" | "codeRules" | "blocks" | "settings";
  type NavIcon = "gear" | "folder" | "file" | "dice" | "play";
  type AnyDoc = S2Doc | S3Doc;

  const STANDARDS: RuleStandard[] = ["standard2", "standard3"];

  let open = $state(false);
  let active = $state<RuleStandard>("standard2");
  let section = $state<SectionId>("overview");

  let doc2 = $state<S2Doc | null>(null);
  let doc3 = $state<S3Doc | null>(null);
  let baseline2 = $state("");
  let baseline3 = $state("");
  let rawMode = $state(false);
  let rawText = $state("");
  let rawBaseline = $state("");
  let status = $state("");
  let notice = $state("");
  let saving = $state(false);
  let loadSeq = 0;

  // ---- 各分区的新增/搜索草稿 ----
  let newFolder = $state("");
  let folderQuery = $state("");
  let newNation = $state("");
  let newNationFolder = $state("");
  let nationQuery = $state("");
  // 国籍表可达 240 行，每行一个目录下拉；分页把「行 × 选项」的 DOM 规模压到可交互范围。
  const NATION_PAGE_SIZE = 40;
  let nationPage = $state(0);
  let preferAdd = $state<Record<string, string>>({});
  let blockQuery = $state("");
  let newBlockKey = $state("");
  let selectedBlock = $state("_DEFAULT_");
  let dirAdd = $state<Record<string, string>>({});
  let weightAdd = $state<Record<string, string>>({});

  // ===================== 文档 / 脏标记 =====================

  function currentDoc(): AnyDoc | null {
    return active === "standard2" ? doc2 : doc3;
  }

  const doc = $derived<AnyDoc | null>(active === "standard2" ? doc2 : doc3);
  const s2 = $derived<S2Doc | null>(active === "standard2" ? doc2 : null);
  const s3 = $derived<S3Doc | null>(active === "standard3" ? doc3 : null);

  const baseline = $derived(active === "standard2" ? baseline2 : baseline3);
  const dirty = $derived(
    rawMode ? rawText !== rawBaseline : !!doc && formatDoc(doc) !== baseline,
  );
  const canSave = $derived(rawMode ? rawText.trim().length > 0 : !!doc);

  const usage = $derived(doc ? folderUsageMap(active, doc) : new Map<string, number>());
  const issues = $derived(doc ? healthIssues(active, doc) : []);
  const folders = $derived(doc?.FolderList ?? []);
  const filePath = $derived(`data/rules/${active}.json`);

  const folderVisible = $derived.by(() => {
    const q = folderQuery.trim().toLowerCase();
    return q ? folders.filter((f) => f.toLowerCase().includes(q)) : folders;
  });

  const nationRows = $derived.by(() => {
    if (!doc) return [];
    const q = nationQuery.trim().toLowerCase();
    return sortedKeys(doc.Ethnics)
      .filter((code) => {
        if (!q) return true;
        return code.toLowerCase().includes(q) || doc.Ethnics[code].toLowerCase().includes(q);
      })
      .map((code) => ({ code, dir: doc.Ethnics[code] }));
  });
  const nationPageCount = $derived(Math.max(1, Math.ceil(nationRows.length / NATION_PAGE_SIZE)));
  const nationVisible = $derived(
    nationRows.slice(nationPage * NATION_PAGE_SIZE, (nationPage + 1) * NATION_PAGE_SIZE),
  );
  // 删除/筛选后当前页可能越界：夹回最后一页，避免停在空白页。
  $effect(() => {
    if (nationPage >= nationPageCount) nationPage = nationPageCount - 1;
  });

  const blockRows = $derived.by(() => {
    if (!s3) return [];
    const q = blockQuery.trim().toLowerCase();
    return Object.keys(s3.EthnicityMappingRules)
      .filter((key) => !q || key.toLowerCase().includes(q))
      .sort((a, b) => {
        if (a === "_DEFAULT_") return -1;
        if (b === "_DEFAULT_") return 1;
        return a.localeCompare(b, "en");
      })
      .map((key) => ({
        key,
        kind: blockKind(s3, key),
        count: raceCodeCount(s3.EthnicityMappingRules[key]),
      }));
  });

  const blockCandidates = $derived.by(() => {
    if (!s3) {
      return { nations: [] as string[], folders: [] as string[], canAddDefault: false };
    }
    const used = new Set(Object.keys(s3.EthnicityMappingRules));
    return {
      nations: sortedKeys(s3.Ethnics).filter((c) => !used.has(c)),
      folders: s3.FolderList.filter((f) => !used.has(f)),
      // 缺 _DEFAULT_ 时也要能从界面补回来，否则体检提示「缺默认块」却只能改源码。
      canAddDefault: !used.has("_DEFAULT_"),
    };
  });

  const currentBlock = $derived(
    s3 && selectedBlock ? s3.EthnicityMappingRules[selectedBlock] ?? null : null,
  );
  const currentBlockKind = $derived(
    s3 && selectedBlock ? blockKind(s3, selectedBlock) : "unknown",
  );

  const stats = $derived.by(() => {
    if (active === "standard2") {
      return [
        { label: t.ruleEditor.statFolders, value: s2?.FolderList.length ?? 0 },
        { label: t.ruleEditor.statNations, value: Object.keys(s2?.Ethnics ?? {}).length },
        { label: t.ruleEditor.statCodeRules, value: Object.keys(s2?.CodeRules ?? {}).length },
        { label: t.ruleEditor.statRaceCodes, value: RACE_CODES.length },
      ];
    }
    const rules = Object.values(s3?.EthnicityMappingRules ?? {}).reduce(
      (sum, block) => sum + Object.keys(block).length,
      0,
    );
    return [
      { label: t.ruleEditor.statFolders, value: s3?.FolderList.length ?? 0 },
      { label: t.ruleEditor.statNations, value: Object.keys(s3?.Ethnics ?? {}).length },
      { label: t.ruleEditor.statBlocks, value: Object.keys(s3?.EthnicityMappingRules ?? {}).length },
      { label: t.ruleEditor.statRuleEntries, value: rules },
    ];
  });

  const sectionMeta = $derived.by(() => {
    switch (section) {
      case "folders":
        return { title: t.ruleEditor.folders.title, desc: t.ruleEditor.folders.desc };
      case "ethnics":
        return { title: t.ruleEditor.ethnics.title, desc: t.ruleEditor.ethnics.desc };
      case "codeRules":
        return { title: t.ruleEditor.codeRules.title, desc: t.ruleEditor.codeRules.desc };
      case "blocks":
        return { title: t.ruleEditor.blocks.title, desc: t.ruleEditor.blocks.desc };
      case "settings":
        return { title: t.ruleEditor.settings.title, desc: t.ruleEditor.settings.desc };
      default:
        return { title: t.ruleEditor.overview.title, desc: t.ruleEditor.overview.desc };
    }
  });

  const navItems = $derived.by(() => {
    const items: { id: SectionId; icon: NavIcon; label: string; count?: number }[] = [
      { id: "overview", icon: "gear", label: t.ruleEditor.sections.overview },
      {
        id: "folders",
        icon: "folder",
        label: t.ruleEditor.sections.folders,
        count: doc?.FolderList.length ?? 0,
      },
      {
        id: "ethnics",
        icon: "file",
        label: t.ruleEditor.sections.ethnics,
        count: Object.keys(doc?.Ethnics ?? {}).length,
      },
    ];
    if (active === "standard2") {
      items.push({
        id: "codeRules",
        icon: "dice",
        label: t.ruleEditor.sections.codeRules,
        count: Object.keys(s2?.CodeRules ?? {}).length,
      });
    } else {
      items.push({
        id: "blocks",
        icon: "play",
        label: t.ruleEditor.sections.blocks,
        count: Object.keys(s3?.EthnicityMappingRules ?? {}).length,
      });
      items.push({ id: "settings", icon: "gear", label: t.ruleEditor.sections.settings });
    }
    return items;
  });

  // ===================== 加载 / 保存 / 模式切换 =====================

  function ensureSelectedBlock(table: S3Doc) {
    const keys = Object.keys(table.EthnicityMappingRules);
    selectedBlock = keys.includes(selectedBlock)
      ? selectedBlock
      : keys.includes("_DEFAULT_")
        ? "_DEFAULT_"
        : keys[0] ?? "";
  }

  async function load(name: RuleStandard) {
    const seq = ++loadSeq;
    status = "";
    notice = "";
    try {
      const text = await api.readRuleFile(name);
      if (seq !== loadSeq || name !== active) return;
      try {
        const parsed = parseRuleFile(name, text);
        newNationFolder = parsed.FolderList[0] ?? "";
        if (name === "standard2") {
          doc2 = parsed as S2Doc;
          baseline2 = formatDoc(parsed);
        } else {
          doc3 = parsed as S3Doc;
          baseline3 = formatDoc(parsed);
          ensureSelectedBlock(parsed as S3Doc);
        }
        rawText = formatDoc(parsed);
        rawBaseline = rawText;
        rawMode = false;
      } catch (e) {
        // 结构化解析失败：不静默丢内容，直接进源码模式让用户修复。
        if (name === "standard2") {
          doc2 = null;
          baseline2 = "";
        } else {
          doc3 = null;
          baseline3 = "";
        }
        rawText = text;
        rawBaseline = text;
        rawMode = true;
        notice = t.ruleEditor.parseFailed(e instanceof Error ? e.message : String(e));
      }
    } catch (e) {
      if (seq !== loadSeq) return;
      await showError(e);
      // 读取本身失败（权限/IO）时不要停在「正在载入」：进入空源码页，用户可关闭后重试打开。
      rawText = "";
      rawBaseline = "";
      rawMode = true;
      notice = e instanceof Error ? e.message : String(e);
    }
  }

  export function openEditor() {
    open = true;
    section = "overview";
    // 清掉上一次的搜索/新增草稿，保留最近使用的标准与规则块选择。
    folderQuery = "";
    nationQuery = "";
    nationPage = 0;
    blockQuery = "";
    newFolder = "";
    newNation = "";
    newBlockKey = "";
    void load(active);
  }

  async function switchTo(name: RuleStandard) {
    if (name === active || saving) return;
    // 切换标准 = 丢弃当前编辑：确认框只问一次，之后 load(name) 直接覆盖内存文档。
    // 【为什么不自动存草稿】规则文件只有一份权威内容（保存即生效），草稿与线上内容混放会让
    // 用户分不清「现在生效的是哪份」；宁可让用户明确回答「放弃修改」，也不暗存第二份。
    if (dirty && !(await ask(t.ruleEditor.unsavedSwitch(standardLabel(active)), {
      title: t.ruleEditor.unsavedTitle,
    }))) return;
    active = name;
    section = "overview";
    rawMode = false;
    status = "";
    notice = "";
    await load(name);
  }

  function standardLabel(name: RuleStandard): string {
    return name === "standard2" ? t.ruleEditor.std2Label : t.ruleEditor.std3Label;
  }

  async function save() {
    if (saving || !canSave) return;
    // 捕获本次保存对应的标准与正文：await 期间即使界面状态变化，也只回填这一份。
    const name = active;
    const payload = rawMode ? rawText : doc ? formatDoc(doc) : "";
    saving = true;
    try {
      await api.saveRuleFile(name, payload);
      const parsed = parseRuleFile(name, payload);
      const formatted = formatDoc(parsed);
      if (name === "standard2") {
        doc2 = parsed as S2Doc;
        baseline2 = formatted;
      } else {
        doc3 = parsed as S3Doc;
        baseline3 = formatted;
        ensureSelectedBlock(parsed as S3Doc);
      }
      if (active === name) {
        rawText = formatted;
        rawBaseline = formatted;
        status = t.ruleEditor.savedStatus;
        notice = "";
      }
      await refreshSnapshot();
    } catch (e) {
      await showError(e);
    } finally {
      saving = false;
    }
  }

  async function toggleSource(): Promise<boolean> {
    if (!rawMode) {
      if (!doc) return false;
      rawText = formatDoc(doc);
      rawBaseline = baseline;
      rawMode = true;
      notice = "";
      return true;
    }
    try {
      const parsed = parseRuleFile(active, rawText);
      if (active === "standard2") doc2 = parsed as S2Doc;
      else {
        doc3 = parsed as S3Doc;
        ensureSelectedBlock(parsed as S3Doc);
      }
      rawMode = false;
      notice = "";
      return true;
    } catch (e) {
      notice = t.ruleEditor.parseFailed(e instanceof Error ? e.message : String(e));
      return false;
    }
  }

  /** 左侧导航：源码模式下先尝试解析回结构化，解析失败则留在源码页。 */
  async function goSection(id: SectionId) {
    if (rawMode && !(await toggleSource())) return;
    section = id;
  }

  async function close() {
    if (dirty && !(await ask(t.ruleEditor.unsavedClose(standardLabel(active)), {
      title: t.ruleEditor.unsavedTitle,
    }))) return;
    open = false;
  }

  function onWindowKey(e: KeyboardEvent) {
    if (e.key === "Escape" && open) void close();
  }

  // ===================== 目录清单 =====================

  function addFolder() {
    const d = currentDoc();
    if (!d) return;
    const name = newFolder.trim();
    if (!name) {
      notice = t.ruleEditor.folders.blank;
      return;
    }
    if (d.FolderList.includes(name)) {
      notice = t.ruleEditor.folders.duplicate;
      return;
    }
    d.FolderList.push(name);
    newFolder = "";
    notice = "";
    if (!newNationFolder) newNationFolder = name;
  }

  async function removeFolder(name: string) {
    const d = currentDoc();
    if (!d) return;
    const used = usage.get(name) ?? 0;
    const ok = await ask(t.ruleEditor.folders.removeConfirm(name, used), {
      title: t.ruleEditor.unsavedTitle,
    });
    if (!ok) return;
    d.FolderList = d.FolderList.filter((f) => f !== name);
    if (newNationFolder === name) newNationFolder = d.FolderList[0] ?? "";
    notice = "";
  }

  // ===================== 国籍映射 =====================

  function addNation() {
    const d = currentDoc();
    if (!d) return;
    const code = newNation.trim().toUpperCase();
    if (!/^[A-Z]{2,3}$/.test(code)) {
      notice = t.ruleEditor.ethnics.invalidCode;
      return;
    }
    if (Object.hasOwn(d.Ethnics, code)) {
      notice = t.ruleEditor.ethnics.exists;
      return;
    }
    if (!d.FolderList.includes(newNationFolder)) {
      notice = t.ruleEditor.ethnics.noFolders;
      return;
    }
    d.Ethnics[code] = newNationFolder;
    newNation = "";
    notice = "";
  }

  async function removeNation(code: string) {
    const d = currentDoc();
    if (!d) return;
    const cascade =
      active === "standard3" &&
      (Object.hasOwn((d as S3Doc).DefaultCodes, code) ||
        Object.hasOwn((d as S3Doc).EthnicityMappingRules, code));
    if (
      cascade &&
      !(await ask(t.ruleEditor.ethnics.removeConfirm(code), {
        title: t.ruleEditor.unsavedTitle,
      }))
    ) {
      return;
    }
    delete d.Ethnics[code];
    if (active === "standard3") {
      const table = d as S3Doc;
      delete table.DefaultCodes[code];
      delete table.EthnicityMappingRules[code];
      if (selectedBlock === code) ensureSelectedBlock(table);
    }
    notice = "";
  }

  function setNationFolder(code: string, dir: string) {
    const d = currentDoc();
    if (d) d.Ethnics[code] = dir;
  }

  // ===================== 标准二：种族码修正 =====================

  function addCodeRule(code: string) {
    if (!s2 || s2.CodeRules[code]) return;
    const fallback = s2.FolderList[0] ?? "";
    if (!fallback) {
      notice = t.ruleEditor.ethnics.noFolders;
      return;
    }
    s2.CodeRules[code] = { prefer: [], fallback };
    notice = "";
  }

  function removeCodeRule(code: string) {
    if (s2) delete s2.CodeRules[code];
  }

  function availablePrefer(code: string): string[] {
    if (!s2) return [];
    const rule = s2.CodeRules[code];
    return rule ? s2.FolderList.filter((f) => !rule.prefer.includes(f)) : [];
  }

  function addPrefer(code: string) {
    const rule = s2?.CodeRules[code];
    if (!rule) return;
    const options = availablePrefer(code);
    const dir = preferAdd[code] && options.includes(preferAdd[code]) ? preferAdd[code] : options[0];
    if (!dir) return;
    rule.prefer.push(dir);
    preferAdd[code] = "";
  }

  function removePrefer(code: string, index: number) {
    s2?.CodeRules[code]?.prefer.splice(index, 1);
  }

  function movePrefer(code: string, index: number, delta: number) {
    const list = s2?.CodeRules[code]?.prefer;
    if (!list) return;
    const to = index + delta;
    if (to < 0 || to >= list.length) return;
    const [item] = list.splice(index, 1);
    list.splice(to, 0, item);
  }

  function setFallback(code: string, dir: string) {
    const rule = s2?.CodeRules[code];
    if (rule) rule.fallback = dir;
  }

  // ===================== 标准三：规则块 =====================

  function addBlock() {
    if (!s3 || !newBlockKey) return;
    if (Object.hasOwn(s3.EthnicityMappingRules, newBlockKey)) return;
    const allowed =
      newBlockKey === "_DEFAULT_"
        ? blockCandidates.canAddDefault
        : blockCandidates.nations.includes(newBlockKey) ||
          blockCandidates.folders.includes(newBlockKey);
    if (!allowed) return;
    s3.EthnicityMappingRules[newBlockKey] = {};
    selectedBlock = newBlockKey;
    newBlockKey = "";
    section = "blocks";
    notice = "";
  }

  async function deleteBlock(key: string) {
    if (!s3 || key === "_DEFAULT_") return;
    const ok = await ask(t.ruleEditor.blocks.deleteBlockConfirm(key), {
      title: t.ruleEditor.unsavedTitle,
    });
    if (!ok) return;
    delete s3.EthnicityMappingRules[key];
    if (selectedBlock === key) ensureSelectedBlock(s3);
    notice = "";
  }

  function firstFolder(): string {
    return currentDoc()?.FolderList[0] ?? "";
  }

  function ruleTypeHint(kind: MappingKind | null): string {
    switch (kind) {
      case "default":
        return t.ruleEditor.blocks.typeDefaultHint;
      case "directly":
        return t.ruleEditor.blocks.typeDirectlyHint;
      case "random":
        return t.ruleEditor.blocks.typeRandomHint;
      case "randomWithWeight":
        return t.ruleEditor.blocks.typeWeightedHint;
      default:
        return t.ruleEditor.blocks.unknownRule;
    }
  }

  function blockKindLabel(kind: "default" | "nation" | "folder" | "unknown"): string {
    switch (kind) {
      case "default":
        return t.ruleEditor.blocks.defaultKind;
      case "nation":
        return t.ruleEditor.blocks.nationKind;
      case "folder":
        return t.ruleEditor.blocks.folderKind;
      default:
        return t.ruleEditor.blocks.unknownKind;
    }
  }

  /** 切换某条规则的四形态；保留能复用的目录，切到 default 时丢弃目录。 */
  function setRuleKind(blockKey: string, code: string, kind: MappingKind | "") {
    if (!s3) return;
    const block = s3.EthnicityMappingRules[blockKey];
    if (!block) return;
    if (kind === "") {
      delete block[code];
      return;
    }
    const old = block[code];
    // 只保留仍在 FolderList 内的旧目录：切换形态不应把既有失效引用续进新规则。
    const dirs = ruleDirs(old).filter((dir) => s3.FolderList.includes(dir));
    const first = dirs[0] ?? firstFolder();
    if (!first) {
      notice = t.ruleEditor.ethnics.noFolders;
      return;
    }
    if (kind === "default") {
      block[code] = "default";
    } else if (kind === "directly") {
      block[code] = { typeOfMapping: "directly", directory: first };
    } else if (kind === "random") {
      block[code] = { typeOfMapping: "random", directory: dirs.length ? [...dirs] : [first] };
    } else {
      const list = dirs.length ? [...dirs] : [first];
      const base = Math.floor(100 / list.length);
      block[code] = {
        typeOfMapping: "randomWithWeight",
        directory: {
          ethnicities: list,
          weights: list.map((_, i) => (i === list.length - 1 ? 100 - base * (list.length - 1) : base)),
        },
      };
    }
    notice = "";
  }

  function ruleKeyFor(code: string): string {
    return `${selectedBlock}:${code}`;
  }

  function raceCodeCount(block: Record<string, unknown>): number {
    return RACE_CODES.filter((code) => Object.hasOwn(block, code)).length;
  }

  function extraCodeCount(block: Record<string, unknown>): number {
    const known = new Set<string>(RACE_CODES);
    return Object.keys(block).filter((code) => !known.has(code)).length;
  }

  function randomAddOptions(exclude: string[]): string[] {
    return (s3?.FolderList ?? []).filter((f) => !exclude.includes(f));
  }

  function setDirectDir(blockKey: string, code: string, dir: string) {
    const block = s3?.EthnicityMappingRules[blockKey];
    const rule = block?.[code];
    if (rule && mappingKind(rule) === "directly") {
      (rule as { directory: string }).directory = dir;
    }
  }

  function addRandomDir(blockKey: string, code: string) {
    const block = s3?.EthnicityMappingRules[blockKey];
    const rule = block?.[code];
    if (!rule || mappingKind(rule) !== "random") return;
    const list = randomDirs(rule);
    const options = (s3?.FolderList ?? []).filter((d) => !list.includes(d));
    const pick = dirAdd[ruleKeyFor(code)];
    const dir = pick && options.includes(pick) ? pick : options[0];
    if (!dir) return;
    list.push(dir);
    dirAdd[ruleKeyFor(code)] = "";
  }

  function removeRandomDir(blockKey: string, code: string, index: number) {
    const block = s3?.EthnicityMappingRules[blockKey];
    const rule = block?.[code];
    if (!rule || mappingKind(rule) !== "random") return;
    const list = randomDirs(rule);
    if (list.length <= 1) {
      notice = t.ruleEditor.blocks.atLeastOne;
      return;
    }
    list.splice(index, 1);
    notice = "";
  }

  function addWeightedRow(blockKey: string, code: string) {
    const block = s3?.EthnicityMappingRules[blockKey];
    const rule = block?.[code];
    if (!rule) return;
    const data = weightedList(rule);
    if (!data) return;
    const options = s3?.FolderList ?? [];
    const pick = weightAdd[ruleKeyFor(code)];
    const dir = pick && options.includes(pick) ? pick : options[0];
    if (!dir) return;
    data.ethnicities.push(dir);
    data.weights.push(100);
    weightAdd[ruleKeyFor(code)] = "";
  }

  function removeWeightedRow(blockKey: string, code: string, index: number) {
    const block = s3?.EthnicityMappingRules[blockKey];
    const rule = block?.[code];
    const data = rule ? weightedList(rule) : null;
    if (!data) return;
    if (data.ethnicities.length <= 1) {
      notice = t.ruleEditor.blocks.atLeastOne;
      return;
    }
    data.ethnicities.splice(index, 1);
    data.weights.splice(index, 1);
    notice = "";
  }

  function setWeightedDir(blockKey: string, code: string, index: number, dir: string) {
    const block = s3?.EthnicityMappingRules[blockKey];
    const rule = block?.[code];
    const data = rule ? weightedList(rule) : null;
    if (data) data.ethnicities[index] = dir;
  }

  function setWeightedWeight(blockKey: string, code: string, index: number, raw: string) {
    const block = s3?.EthnicityMappingRules[blockKey];
    const rule = block?.[code];
    const data = rule ? weightedList(rule) : null;
    if (!data) return;
    const n = Number(raw);
    data.weights[index] = Number.isFinite(n) ? n : 0;
  }

  function deleteRule(blockKey: string, code: string) {
    const block = s3?.EthnicityMappingRules[blockKey];
    if (block) delete block[code];
    notice = "";
  }

  function setFallbackMode(mode: "random" | "skip") {
    if (s3) s3.Fallback = mode;
  }

  // ===================== 概览 =====================

  function issueText(issue: HealthIssue): string {
    switch (issue.kind) {
      case "emptyFolders":
        return t.ruleEditor.overview.healthEmptyFolders;
      case "emptyEthnics":
        return t.ruleEditor.overview.healthEmptyEthnics;
      case "dupFolders":
        return t.ruleEditor.overview.healthDup(issue.count);
      case "badEthnics":
        return t.ruleEditor.overview.healthEthnics(issue.count);
      case "badRefs":
        return t.ruleEditor.overview.healthRefs(issue.count);
      case "missingDefault":
        return t.ruleEditor.overview.healthMissingDefault;
      case "badVersion":
        return t.ruleEditor.overview.healthVersion(issue.version);
      case "orphanDefaultCodes":
        return t.ruleEditor.overview.healthOrphans(issue.count);
    }
  }

  function issueSection(issue: HealthIssue): SectionId {
    switch (issue.kind) {
      case "emptyFolders":
      case "dupFolders":
        return "folders";
      case "emptyEthnics":
      case "badEthnics":
        return "ethnics";
      case "badRefs":
        return active === "standard2" ? "codeRules" : "blocks";
      case "missingDefault":
        return "blocks";
      case "badVersion":
        return "settings";
      case "orphanDefaultCodes":
        return "ethnics";
    }
  }

  const flowSteps = $derived(
    active === "standard2"
      ? t.ruleEditor.overview.flow2
      : t.ruleEditor.overview.flow3,
  );
</script>

<svelte:window onkeydown={onWindowKey} />

{#if open}
  <div
    class="modal-overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) void close();
    }}
  >
    <div class="modal rule-modal" role="dialog" aria-modal="true" aria-label={t.ruleEditor.editorAria}>
      <div class="rule-accent"></div>

      <header class="rule-head">
        <div class="rule-heading">
          <span class="rule-kicker">{t.ruleEditor.kicker(active === "standard2" ? 2 : 3, folders.length)}</span>
          <div class="rule-title-row">
            <span class="rule-title">{t.ruleEditor.title}</span>
            <div class="seg3">
              {#each STANDARDS as name (name)}
                <button
                  class:active={active === name}
                  onclick={() => switchTo(name)}
                  disabled={saving}
                >
                  {name === "standard2" ? t.ruleEditor.std2Label : t.ruleEditor.std3Label}
                </button>
              {/each}
            </div>
          </div>
          <span class="rule-path mono">{filePath}</span>
        </div>
        <div class="modal-actions rule-actions">
          {#if notice}<span class="notice small">{notice}</span>{/if}
          {#if status && !dirty}<span class="ok-text small">{status}</span>{/if}
          <button
            class="btn source-btn"
            class:source-on={rawMode}
            onclick={toggleSource}
            disabled={saving}
            use:tip={t.ruleEditor.sourceHint}
          >
            {rawMode ? t.ruleEditor.structuredBtn : t.ruleEditor.sourceBtn}
          </button>
          <button
            class="btn primary"
            onclick={save}
            disabled={!dirty || !canSave || saving}
            use:tip={t.ruleEditor.saveHint}
          >
            {t.ruleEditor.saveBtn}
          </button>
          <button class="btn" onclick={close} disabled={saving}>{t.ruleEditor.closeBtn}</button>
        </div>
      </header>

      <div class="rule-body">
        <nav class="rule-nav" aria-label={t.ruleEditor.editorAria}>
          {#each navItems as item (item.id)}
            <button
              class="nav-btn"
              class:active={section === item.id}
              onclick={() => goSection(item.id)}
            >
              <Icon name={item.icon} size={15} />
              <span class="nav-label">{item.label}</span>
              {#if item.count !== undefined}
                <span class="nav-count">{item.count}</span>
              {/if}
            </button>
          {/each}
        </nav>

        <div class="rule-pane">
          {#if rawMode}
            <div class="pane-head">
              <div>
                <h3 class="pane-title">{t.ruleEditor.sourceTitle}</h3>
                <p class="pane-desc">{t.ruleEditor.sourceDesc}</p>
              </div>
            </div>
            <div class="pane-scroll flush source-wrap">
              <textarea
                class="editor source-editor"
                bind:value={rawText}
                oninput={() => (status = "")}
                spellcheck="false"
                wrap="off"
                aria-label={t.ruleEditor.editorAria}
              ></textarea>
            </div>
          {:else if doc}
            <div class="pane-head">
              <div>
                <h3 class="pane-title">{sectionMeta.title}</h3>
                <p class="pane-desc">{sectionMeta.desc}</p>
              </div>
            </div>

            <div
              class="pane-scroll"
              class:flush={section === "blocks"}
              class:fill-table={section === "ethnics"}
            >
              {#if section === "overview"}
                <div class="stat-grid">
                  {#each stats as stat (stat.label)}
                    <div class="stat-card">
                      <div class="stat-value">{stat.value}</div>
                      <div class="stat-label">{stat.label}</div>
                    </div>
                  {/each}
                </div>

                <div class="health-card">
                  <div class="health-head">
                    <span class="health-title">{t.ruleEditor.overview.healthTitle}</span>
                    {#if issues.length === 0}
                      <span class="pill ok"><span class="dot"></span>{t.ruleEditor.overview.healthOk}</span>
                    {:else}
                      <span class="pill warn"><span class="dot"></span>{t.ruleEditor.overview.healthWarn(issues.length)}</span>
                    {/if}
                  </div>
                  {#if issues.length === 0}
                    <p class="health-ok-desc">{t.ruleEditor.overview.healthOkDesc}</p>
                  {:else}
                    {#each issues as issue, i (i)}
                      <button class="health-row" onclick={() => (section = issueSection(issue))}>
                        <span class="health-ic"><Icon name="alert" size={14} /></span>
                        <span>{issueText(issue)}</span>
                        <span class="health-go">{t.ruleEditor.overview.goFix}</span>
                      </button>
                    {/each}
                  {/if}
                </div>

                <div class="flow-card">
                  <span class="flow-title">{t.ruleEditor.overview.flowTitle}</span>
                  <div class="flow-steps">
                    {#each flowSteps as step, i (step)}
                      <span class="flow-step">
                        <span class="flow-num">{i + 1}</span>
                        <span>{step}</span>
                      </span>
                      {#if i < flowSteps.length - 1}
                        <Icon name="chevronRight" size={13} />
                      {/if}
                    {/each}
                  </div>
                  <p class="flow-hint">{t.ruleEditor.overview.flowHint}</p>
                </div>

                <p class="overview-file muted small">
                  {t.ruleEditor.overview.fileLabel}：<span class="mono">{filePath}</span>
                </p>
              {:else if section === "folders"}
                <div class="section-toolbar">
                  <div class="search inline-search">
                    <Icon name="search" />
                    <input
                      type="search"
                      bind:value={folderQuery}
                      placeholder={t.ruleEditor.folders.searchPlaceholder}
                      aria-label={t.ruleEditor.folders.searchPlaceholder}
                    />
                  </div>
                  <span class="muted small">{t.ruleEditor.folders.count(folders.length)}</span>
                  <input
                    type="text"
                    class="add-input"
                    bind:value={newFolder}
                    placeholder={t.ruleEditor.folders.addPlaceholder}
                    aria-label={t.ruleEditor.folders.addPlaceholder}
                    onkeydown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        addFolder();
                      }
                    }}
                  />
                  <button class="btn" onclick={addFolder}>
                    <Icon name="plus" size={13} /> {t.ruleEditor.common.add}
                  </button>
                </div>
                {#if folderVisible.length === 0}
                  <div class="empty-state">
                    <Icon name="folder" size={26} />
                    <span>{t.ruleEditor.folders.empty}</span>
                  </div>
                {:else}
                  <div class="tag-cloud">
                    {#each folderVisible as folder (folder)}
                      {@const used = usage.get(folder) ?? 0}
                      <span class="folder-tag" class:in-use={used > 0}>
                        <Icon name="folder" size={12} />
                        <span class="mono folder-name">{folder}</span>
                        <span class="folder-use" use:tip={t.ruleEditor.folders.used(used)}>{used}</span>
                        <button
                          class="tag-x"
                          onclick={() => removeFolder(folder)}
                          use:tip={t.ruleEditor.folders.removeHint}
                          aria-label={t.ruleEditor.folders.removeHint}
                        >
                          <Icon name="x" size={11} />
                        </button>
                      </span>
                    {/each}
                  </div>
                {/if}
              {:else if section === "ethnics"}
                <!-- 与「国籍对照表」同款交互：第一行搜索/计数/翻页，第二行独立的新增表单 -->
                <div class="section-toolbar">
                  <div class="search inline-search">
                    <Icon name="search" />
                    <input
                      type="search"
                      bind:value={nationQuery}
                      oninput={() => (nationPage = 0)}
                      placeholder={t.ruleEditor.ethnics.searchPlaceholder}
                      aria-label={t.ruleEditor.ethnics.searchPlaceholder}
                    />
                  </div>
                  <span class="muted small">{t.ruleEditor.ethnics.count(nationRows.length)}</span>
                  {#if nationPageCount > 1}
                    <div class="mini-pager">
                      <button
                        class="btn sm"
                        onclick={() => (nationPage = Math.max(0, nationPage - 1))}
                        disabled={nationPage === 0}
                        use:tip={t.table.prevPage}
                        aria-label={t.table.prevPage}
                      >
                        <Icon name="chevronLeft" size={12} />
                      </button>
                      <span class="muted small">{t.table.pageOf(nationPage + 1, nationPageCount)}</span>
                      <button
                        class="btn sm"
                        onclick={() => (nationPage = Math.min(nationPageCount - 1, nationPage + 1))}
                        disabled={nationPage + 1 >= nationPageCount}
                        use:tip={t.table.nextPage}
                        aria-label={t.table.nextPage}
                      >
                        <Icon name="chevronRight" size={12} />
                      </button>
                    </div>
                  {/if}
                </div>
                <div class="section-add">
                  <input
                    type="text"
                    class="add-input code-input"
                    bind:value={newNation}
                    maxlength="3"
                    placeholder={t.ruleEditor.ethnics.addCodePlaceholder}
                    aria-label={t.ruleEditor.ethnics.codeHeader}
                  />
                  <select
                    class="add-select"
                    bind:value={newNationFolder}
                    aria-label={t.ruleEditor.ethnics.folderHeader}
                  >
                    {#each folders as folder (folder)}
                      <option value={folder}>{folder}</option>
                    {/each}
                  </select>
                  <button class="btn" onclick={addNation}>
                    <Icon name="plus" size={13} /> {t.ruleEditor.common.add}
                  </button>
                </div>
                {#if nationRows.length === 0}
                  <div class="empty-state">
                    <Icon name="file" size={26} />
                    <span>{t.ruleEditor.ethnics.empty}</span>
                  </div>
                {:else}
                  <div class="data-table">
                    <table>
                      <thead>
                        <tr>
                          <th class="code-col">{t.ruleEditor.ethnics.codeHeader}</th>
                          <th>{t.ruleEditor.ethnics.folderHeader}</th>
                          <th class="act-col"></th>
                        </tr>
                      </thead>
                      <tbody>
                        {#each nationVisible as row (row.code)}
                          <tr>
                            <td class="code-cell mono">{row.code}</td>
                            <td>
                              <select
                                value={row.dir}
                                onchange={(e) => setNationFolder(row.code, e.currentTarget.value)}
                                aria-label={t.ruleEditor.ethnics.folderHeader}
                              >
                                {#if !folders.includes(row.dir)}
                                  <option value={row.dir}>{row.dir}（{t.ruleEditor.common.notInList}）</option>
                                {/if}
                                {#each folders as folder (folder)}
                                  <option value={folder}>{folder}</option>
                                {/each}
                              </select>
                            </td>
                            <td class="act-col">
                              <button
                                class="btn sm danger"
                                onclick={() => removeNation(row.code)}
                                use:tip={t.ruleEditor.ethnics.removeHint}
                              >
                                {t.ruleEditor.common.delete}
                              </button>
                            </td>
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  </div>
                {/if}
              {:else if section === "codeRules" && s2}
                <div class="code-grid">
                  {#each RACE_CODES as code (code)}
                    {#if s2.CodeRules[code]}
                      {@const rule = s2.CodeRules[code]}
                      <div class="code-card">
                        <div class="code-card-head">
                          <span class="code-badge">{code}</span>
                          <span class="code-card-title">{t.ruleEditor.codeRules.codeLabel(code)}</span>
                          <button
                            class="icon-btn danger"
                            onclick={() => removeCodeRule(code)}
                            use:tip={t.ruleEditor.codeRules.deleteHint}
                            aria-label={t.ruleEditor.codeRules.deleteHint}
                          >
                            <Icon name="x" size={13} />
                          </button>
                        </div>

                        <div class="field">
                          <span class="field-label">{t.ruleEditor.codeRules.preferLabel}</span>
                          {#if rule.prefer.length === 0}
                            <p class="field-hint">{t.ruleEditor.codeRules.emptyPrefer}</p>
                          {:else}
                            <div class="prefer-list">
                              {#each rule.prefer as dir, i (i)}
                                <div class="prefer-row" class:bad={!s2.FolderList.includes(dir)}>
                                  <span class="prefer-idx">{i + 1}</span>
                                  <span class="prefer-dir mono">{dir}</span>
                                  <button
                                    class="icon-btn"
                                    disabled={i === 0}
                                    onclick={() => movePrefer(code, i, -1)}
                                    use:tip={t.ruleEditor.codeRules.moveUp}
                                    aria-label={t.ruleEditor.codeRules.moveUp}
                                  >
                                    <Icon name="chevronUp" size={12} />
                                  </button>
                                  <button
                                    class="icon-btn"
                                    disabled={i === rule.prefer.length - 1}
                                    onclick={() => movePrefer(code, i, 1)}
                                    use:tip={t.ruleEditor.codeRules.moveDown}
                                    aria-label={t.ruleEditor.codeRules.moveDown}
                                  >
                                    <Icon name="chevronDown" size={12} />
                                  </button>
                                  <button
                                    class="icon-btn danger"
                                    onclick={() => removePrefer(code, i)}
                                    use:tip={t.ruleEditor.codeRules.removeDir}
                                    aria-label={t.ruleEditor.codeRules.removeDir}
                                  >
                                    <Icon name="x" size={12} />
                                  </button>
                                </div>
                              {/each}
                            </div>
                          {/if}
                          <div class="add-row">
                            <select
                              value={preferAdd[code] ?? availablePrefer(code)[0] ?? ""}
                              onchange={(e) => (preferAdd[code] = e.currentTarget.value)}
                              aria-label={t.ruleEditor.codeRules.addPlaceholder}
                            >
                              {#each availablePrefer(code) as dir (dir)}
                                <option value={dir}>{dir}</option>
                              {/each}
                            </select>
                            <button
                              class="btn sm"
                              onclick={() => addPrefer(code)}
                              disabled={availablePrefer(code).length === 0}
                            >
                              <Icon name="plus" size={12} /> {t.ruleEditor.codeRules.addBtnShort}
                            </button>
                          </div>
                        </div>

                        <div class="field inline-field">
                          <span class="field-label">{t.ruleEditor.codeRules.fallbackLabel}</span>
                          <select
                            value={rule.fallback}
                            onchange={(e) => setFallback(code, e.currentTarget.value)}
                            aria-label={t.ruleEditor.codeRules.fallbackLabel}
                          >
                            {#if rule.fallback && !s2.FolderList.includes(rule.fallback)}
                              <option value={rule.fallback}>{rule.fallback}（{t.ruleEditor.common.notInList}）</option>
                            {/if}
                            {#each s2.FolderList as dir (dir)}
                              <option value={dir}>{dir}</option>
                            {/each}
                          </select>
                        </div>
                      </div>
                    {:else}
                      <div class="code-card ghost">
                        <div class="code-card-head">
                          <span class="code-badge muted">{code}</span>
                          <span class="code-card-title muted">{t.ruleEditor.codeRules.unset}</span>
                          <button class="btn sm" onclick={() => addCodeRule(code)}>
                            <Icon name="plus" size={12} /> {t.ruleEditor.codeRules.addBtn}
                          </button>
                        </div>
                      </div>
                    {/if}
                  {/each}
                </div>
              {:else if section === "blocks" && s3}
                <div class="block-layout">
                  <aside class="block-list">
                    <div class="search inline-search">
                      <Icon name="search" />
                      <input
                        type="search"
                        bind:value={blockQuery}
                        placeholder={t.ruleEditor.blocks.searchPlaceholder}
                        aria-label={t.ruleEditor.blocks.searchPlaceholder}
                      />
                    </div>
                    <div class="add-row">
                      <select
                        bind:value={newBlockKey}
                        aria-label={t.ruleEditor.blocks.addPlaceholder}
                      >
                        <option value="">{t.ruleEditor.blocks.addPlaceholder}</option>
                        {#if blockCandidates.canAddDefault}
                          <option value="_DEFAULT_">_DEFAULT_ · {t.ruleEditor.blocks.defaultKind}</option>
                        {/if}
                        {#if blockCandidates.nations.length > 0}
                          <optgroup label={t.ruleEditor.blocks.nationKind}>
                            {#each blockCandidates.nations as code (code)}
                              <option value={code}>{code}</option>
                            {/each}
                          </optgroup>
                        {/if}
                        {#if blockCandidates.folders.length > 0}
                          <optgroup label={t.ruleEditor.blocks.folderKind}>
                            {#each blockCandidates.folders as folder (folder)}
                              <option value={folder}>{folder}</option>
                            {/each}
                          </optgroup>
                        {/if}
                      </select>
                      <button class="btn sm" onclick={addBlock} disabled={!newBlockKey}>
                        <Icon name="plus" size={12} /> {t.ruleEditor.common.add}
                      </button>
                    </div>
                    <div class="block-items">
                      {#each blockRows as row (row.key)}
                        <button
                          class="block-item"
                          class:active={selectedBlock === row.key}
                          onclick={() => (selectedBlock = row.key)}
                        >
                          <span class="block-kind kind-{row.kind}">{blockKindLabel(row.kind)}</span>
                          <span class="block-key mono">{row.key}</span>
                          <span class="block-count">{row.count}/{RACE_CODES.length}</span>
                        </button>
                      {:else}
                        <p class="muted small block-empty">{t.ruleEditor.blocks.empty}</p>
                      {/each}
                    </div>
                  </aside>

                  <div class="block-detail">
                    {#if currentBlock && selectedBlock}
                      <div class="block-detail-head">
                        <div class="block-detail-title">
                          <span class="block-kind kind-{currentBlockKind}">{blockKindLabel(currentBlockKind)}</span>
                          <span class="mono block-detail-key">{selectedBlock}</span>
                        </div>
                        <span class="muted small">{t.ruleEditor.blocks.count(raceCodeCount(currentBlock), RACE_CODES.length)}</span>
                        {#if selectedBlock !== "_DEFAULT_"}
                          <button
                            class="btn sm danger block-delete"
                            onclick={() => deleteBlock(selectedBlock)}
                            use:tip={t.ruleEditor.blocks.deleteBlockHint}
                          >
                            <Icon name="x" size={12} /> {t.ruleEditor.common.delete}
                          </button>
                        {/if}
                      </div>
                      <p class="block-desc">{t.ruleEditor.blocks.detailDesc}</p>
                      {#if extraCodeCount(currentBlock) > 0}
                        <p class="rule-hint block-extra">
                          {t.ruleEditor.blocks.extraCodes(extraCodeCount(currentBlock))}
                        </p>
                      {/if}

                      <div class="rule-code-list">
                        {#each RACE_CODES as code (code)}
                          {@const rule = currentBlock[code]}
                          {@const kind = rule === undefined ? null : mappingKind(rule)}
                          {@const key = ruleKeyFor(code)}
                          <div class="rule-code-card" class:missing={rule === undefined}>
                            <div class="rule-code-head">
                              <span class="code-badge">{code}</span>
                              {#if rule === undefined}
                                <span class="muted small rule-missing-text">{t.ruleEditor.blocks.codeMissing}</span>
                                <button class="btn sm rule-add-btn" onclick={() => setRuleKind(selectedBlock, code, "default")}>
                                  <Icon name="plus" size={12} /> {t.ruleEditor.blocks.addRuleBtn}
                                </button>
                              {:else}
                                <select
                                  class="rule-type-select"
                                  value={kind ?? ""}
                                  onchange={(e) => setRuleKind(selectedBlock, code, e.currentTarget.value as MappingKind)}
                                  aria-label={t.ruleEditor.blocks.typeLabel}
                                >
                                  {#if kind === null}
                                    <option value="" disabled>{t.ruleEditor.blocks.typeUnknown}</option>
                                  {/if}
                                  <option value="default">{t.ruleEditor.blocks.typeDefault}</option>
                                  <option value="directly">{t.ruleEditor.blocks.typeDirectly}</option>
                                  <option value="random">{t.ruleEditor.blocks.typeRandom}</option>
                                  <option value="randomWithWeight">{t.ruleEditor.blocks.typeWeighted}</option>
                                </select>
                                <button
                                  class="icon-btn danger rule-delete-btn"
                                  onclick={() => deleteRule(selectedBlock, code)}
                                  use:tip={t.ruleEditor.blocks.deleteRuleHint}
                                  aria-label={t.ruleEditor.blocks.deleteRuleHint}
                                >
                                  <Icon name="x" size={13} />
                                </button>
                              {/if}
                            </div>

                            {#if rule !== undefined}
                              {#if kind}
                                <p class="rule-hint">{ruleTypeHint(kind)}</p>
                              {/if}
                              {#if kind === "directly"}
                                {@const dir = directDir(rule)}
                                <div class="rule-line">
                                  <span class="rule-line-label">{t.ruleEditor.blocks.directoryLabel}</span>
                                  <select
                                    value={dir}
                                    onchange={(e) => setDirectDir(selectedBlock, code, e.currentTarget.value)}
                                    aria-label={t.ruleEditor.blocks.directoryLabel}
                                  >
                                    {#if dir && !s3.FolderList.includes(dir)}
                                      <option value={dir}>{dir}（{t.ruleEditor.common.notInList}）</option>
                                    {/if}
                                    {#each s3.FolderList as folder (folder)}
                                      <option value={folder}>{folder}</option>
                                    {/each}
                                  </select>
                                </div>
                              {:else if kind === "random"}
                                {@const list = randomDirs(rule)}
                                {@const options = randomAddOptions(list)}
                                <div class="dir-chips">
                                  {#each list as dir, i (i)}
                                    <span class="dir-chip" class:bad={!s3.FolderList.includes(dir)}>
                                      <span class="mono">{dir}</span>
                                      <button
                                        class="icon-btn"
                                        disabled={list.length <= 1}
                                        onclick={() => removeRandomDir(selectedBlock, code, i)}
                                        use:tip={t.ruleEditor.blocks.removeItem}
                                        aria-label={t.ruleEditor.blocks.removeItem}
                                      >
                                        <Icon name="x" size={11} />
                                      </button>
                                    </span>
                                  {/each}
                                  <div class="add-row compact-add">
                                    <select
                                      value={dirAdd[key] ?? options[0] ?? ""}
                                      onchange={(e) => (dirAdd[key] = e.currentTarget.value)}
                                      aria-label={t.ruleEditor.blocks.addDirPlaceholder}
                                    >
                                      {#each options as folder (folder)}
                                        <option value={folder}>{folder}</option>
                                      {/each}
                                    </select>
                                    <button
                                      class="btn sm"
                                      onclick={() => addRandomDir(selectedBlock, code)}
                                      disabled={options.length === 0}
                                    >
                                      <Icon name="plus" size={12} /> {t.ruleEditor.blocks.addDirBtn}
                                    </button>
                                  </div>
                                </div>
                              {:else if kind === "randomWithWeight"}
                                {@const data = weightedList(rule)}
                                {#if data}
                                  <div class="weight-list">
                                    {#each data.ethnicities as dir, i (i)}
                                      <div class="weight-row">
                                        <select
                                          value={dir}
                                          onchange={(e) => setWeightedDir(selectedBlock, code, i, e.currentTarget.value)}
                                          aria-label={t.ruleEditor.blocks.directoryLabel}
                                        >
                                          {#if !s3.FolderList.includes(dir)}
                                            <option value={dir}>{dir}（{t.ruleEditor.common.notInList}）</option>
                                          {/if}
                                          {#each s3.FolderList as folder (folder)}
                                            <option value={folder}>{folder}</option>
                                          {/each}
                                        </select>
                                        <input
                                          type="number"
                                          min="0.1"
                                          step="1"
                                          value={data.weights[i]}
                                          oninput={(e) => setWeightedWeight(selectedBlock, code, i, e.currentTarget.value)}
                                          aria-label={t.ruleEditor.blocks.weightLabel}
                                        />
                                        <button
                                          class="icon-btn danger"
                                          disabled={data.ethnicities.length <= 1}
                                          onclick={() => removeWeightedRow(selectedBlock, code, i)}
                                          use:tip={t.ruleEditor.blocks.removeItem}
                                          aria-label={t.ruleEditor.blocks.removeItem}
                                        >
                                          <Icon name="x" size={13} />
                                        </button>
                                      </div>
                                    {/each}
                                    <div class="add-row compact-add">
                                      <select
                                        value={weightAdd[key] ?? s3.FolderList[0] ?? ""}
                                        onchange={(e) => (weightAdd[key] = e.currentTarget.value)}
                                        aria-label={t.ruleEditor.blocks.addDirPlaceholder}
                                      >
                                        {#each s3.FolderList as folder (folder)}
                                          <option value={folder}>{folder}</option>
                                        {/each}
                                      </select>
                                      <button class="btn sm" onclick={() => addWeightedRow(selectedBlock, code)}>
                                        <Icon name="plus" size={12} /> {t.ruleEditor.blocks.addWeightBtn}
                                      </button>
                                    </div>
                                  </div>
                                {/if}
                              {:else if kind === null}
                                <div class="rule-warning">
                                  <Icon name="alert" size={14} />
                                  <span>{t.ruleEditor.blocks.unknownRule}</span>
                                  <button class="btn sm" onclick={() => setRuleKind(selectedBlock, code, "default")}>
                                    {t.ruleEditor.blocks.resetRuleBtn}
                                  </button>
                                </div>
                              {/if}
                            {/if}
                          </div>
                        {/each}
                      </div>
                    {:else}
                      <div class="empty-state">
                        <Icon name="play" size={26} />
                        <span>{t.ruleEditor.blocks.noSelection}</span>
                      </div>
                    {/if}
                  </div>
                </div>
              {:else if section === "settings" && s3}
                <div class="settings-grid">
                  <div class="setting-card">
                    <div class="setting-row">
                      <div>
                        <div class="setting-title">{t.ruleEditor.settings.versionLabel}</div>
                        <p class="setting-desc">{t.ruleEditor.settings.versionHint}</p>
                      </div>
                      <span class="pill mono">Version {s3.Version}</span>
                    </div>
                  </div>

                  <div class="setting-card">
                    <div class="setting-title">{t.ruleEditor.settings.fallbackLabel}</div>
                    <div class="choice-row">
                      <button
                        class="choice"
                        class:active={(s3.Fallback ?? "random") === "random"}
                        onclick={() => setFallbackMode("random")}
                      >
                        <span class="choice-title">{t.ruleEditor.settings.fallbackRandom}</span>
                        <span class="choice-desc">{t.ruleEditor.settings.fallbackRandomHint}</span>
                      </button>
                      <button
                        class="choice"
                        class:active={s3.Fallback === "skip"}
                        onclick={() => setFallbackMode("skip")}
                      >
                        <span class="choice-title">{t.ruleEditor.settings.fallbackSkip}</span>
                        <span class="choice-desc">{t.ruleEditor.settings.fallbackSkipHint}</span>
                      </button>
                    </div>
                  </div>

                  <div class="setting-card">
                    <div class="setting-row">
                      <div>
                        <div class="setting-title">{t.ruleEditor.settings.passthroughTitle}</div>
                        <p class="setting-desc">{t.ruleEditor.settings.passthroughDesc}</p>
                      </div>
                      <button class="btn sm" onclick={toggleSource} disabled={saving}>
                        <Icon name="file" size={12} /> {t.ruleEditor.settings.passthroughEditBtn}
                      </button>
                    </div>
                    <div class="passthrough-stats">
                      <span>DefaultCodes · {t.ruleEditor.settings.count(Object.keys(s3.DefaultCodes).length)}</span>
                      <span>FolderMappingFemale · {t.ruleEditor.settings.count(Object.keys(s3.FolderMappingFemale ?? {}).length)}</span>
                      <span>FolderMappingStaff · {t.ruleEditor.settings.count(Object.keys(s3.FolderMappingStaff ?? {}).length)}</span>
                      <span>FolderMappingStaffFemale · {t.ruleEditor.settings.count(Object.keys(s3.FolderMappingStaffFemale ?? {}).length)}</span>
                    </div>
                  </div>
                </div>
              {/if}
            </div>
          {:else}
            <div class="empty-state">
              <Icon name="file" size={26} />
              <span>{t.ruleEditor.loading}</span>
            </div>
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ===================== 弹窗骨架 ===================== */
  :global(.modal.rule-modal) {
    width: min(1120px, 96vw);
    height: min(800px, 92vh);
    padding: 0;
    overflow: hidden;
    position: relative;
  }
  .rule-accent {
    position: absolute;
    inset: 0 0 auto 0;
    height: 3px;
    background: linear-gradient(90deg, var(--accent), var(--accent-line) 45%, transparent 90%);
    z-index: 2;
  }
  .rule-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--sp-5);
    padding: var(--sp-5) var(--sp-6) var(--sp-4);
    border-bottom: 1px solid var(--hairline);
    background: linear-gradient(180deg, var(--bg-surface), var(--bg-inset));
    flex-shrink: 0;
  }
  .rule-heading {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 0;
  }
  .rule-kicker {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    letter-spacing: 0.09em;
    color: var(--accent);
    text-transform: uppercase;
  }
  .rule-title-row {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    flex-wrap: wrap;
  }
  .rule-title {
    font-size: var(--fs-xl);
    font-weight: var(--fw-semibold);
    color: var(--text-1);
  }
  .rule-path {
    font-size: var(--fs-xs);
    color: var(--text-3);
    letter-spacing: 0.02em;
  }
  .rule-actions {
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .notice {
    color: var(--warn);
    max-width: 36ch;
  }
  .source-btn.source-on {
    border-color: var(--accent-line);
    color: var(--accent);
    background: var(--accent-soft);
  }

  /* ===================== 左侧导航 + 右侧工作区 ===================== */
  .rule-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .rule-nav {
    width: 214px;
    flex-shrink: 0;
    padding: var(--sp-4) var(--sp-3);
    border-right: 1px solid var(--hairline);
    background: var(--bg-inset);
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .nav-btn {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    width: 100%;
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: none;
    color: var(--text-2);
    font-size: var(--fs-md);
    text-align: left;
    cursor: pointer;
    transition: color var(--dur-fast) var(--ease-out), background-color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out);
  }
  .nav-btn:hover:not(.active) {
    color: var(--text-1);
    background: var(--hairline);
  }
  .nav-btn.active {
    color: var(--accent);
    background: var(--accent-soft);
    border-color: var(--accent-line);
    font-weight: var(--fw-semibold);
  }
  .nav-label {
    flex: 1;
    min-width: 0;
  }
  .nav-count {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .nav-btn.active .nav-count {
    color: var(--accent);
  }

  .rule-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
  }
  .pane-head {
    padding: var(--sp-4) var(--sp-6) var(--sp-3);
    border-bottom: 1px solid var(--hairline);
    flex-shrink: 0;
  }
  .pane-title {
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: var(--fw-semibold);
    color: var(--text-1);
  }
  .pane-desc {
    margin: var(--sp-1) 0 0;
    color: var(--text-3);
    font-size: var(--fs-sm);
    max-width: 76ch;
    line-height: 1.6;
  }
  .pane-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--sp-4) var(--sp-6) var(--sp-6);
  }
  .pane-scroll.flush {
    padding: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  /* 国籍映射：表格自己滚（表头吸顶），而不是让整页滚、表头浮在行上面 */
  .pane-scroll.fill-table {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .pane-scroll.fill-table .section-toolbar,
  .pane-scroll.fill-table .section-add {
    flex-shrink: 0;
  }
  .pane-scroll.fill-table .data-table {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  /* ===================== 通用工具 ===================== */
  .section-toolbar {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
    margin-bottom: var(--sp-2);
  }
  .section-toolbar .add-input {
    width: 190px;
  }
  /* 新增行：与国籍对照表的「三字码 | 语言 | 全称 | 新增」同构，独立成一行 */
  .section-add {
    display: grid;
    grid-template-columns: 120px 220px auto;
    gap: var(--sp-2);
    align-items: center;
    margin-bottom: var(--sp-3);
  }
  .section-add .code-input {
    width: 100%;
    text-transform: uppercase;
  }
  .section-add .add-select {
    width: 100%;
  }
  .section-add .btn {
    justify-self: start;
  }
  .inline-search {
    flex: 1;
    min-width: 180px;
  }
  .add-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .add-row select {
    flex: 1;
    min-width: 0;
  }
  .compact-add {
    margin-top: var(--sp-2);
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-6);
    color: var(--text-3);
    font-size: var(--fs-sm);
  }

  /* ===================== 概览 ===================== */
  .stat-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--sp-3);
  }
  .stat-card {
    position: relative;
    overflow: hidden;
    padding: var(--sp-4);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
  }
  .stat-card::after {
    content: "";
    position: absolute;
    right: -18px;
    bottom: -26px;
    width: 76px;
    height: 76px;
    border-radius: 50%;
    background: var(--accent-soft);
  }
  .stat-value {
    position: relative;
    z-index: 1;
    font-family: var(--mono);
    font-size: 28px;
    line-height: 1;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }
  .stat-label {
    position: relative;
    z-index: 1;
    margin-top: var(--sp-2);
    font-size: var(--fs-sm);
    color: var(--text-3);
  }
  .health-card {
    margin-top: var(--sp-4);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    overflow: hidden;
  }
  .health-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    border-bottom: 1px solid var(--hairline);
  }
  .health-title {
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
  }
  .health-head .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }
  .health-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    width: 100%;
    padding: var(--sp-2) var(--sp-4);
    border: none;
    border-bottom: 1px solid var(--hairline);
    background: none;
    color: var(--text-2);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .health-row:last-child {
    border-bottom: none;
  }
  .health-row:hover {
    background: var(--bg-surface);
  }
  .health-ic {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--warn);
  }
  .health-go {
    margin-left: auto;
    color: var(--accent);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .health-ok-desc {
    margin: 0;
    padding: var(--sp-3) var(--sp-4);
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  .flow-card {
    margin-top: var(--sp-4);
    padding: var(--sp-4);
    border: 1px dashed var(--hairline-2);
    border-radius: var(--radius-md);
  }
  .flow-title {
    font-size: var(--fs-sm);
    color: var(--text-3);
  }
  .flow-steps {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
    margin-top: var(--sp-2);
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .flow-step {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .flow-num {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
    font-family: var(--mono);
    font-size: var(--fs-xs);
  }
  .flow-hint {
    margin: var(--sp-2) 0 0;
    color: var(--text-3);
    font-size: var(--fs-sm);
    line-height: 1.6;
  }
  .overview-file {
    margin: var(--sp-4) 0 0;
  }

  /* ===================== 目录标签云 ===================== */
  .tag-cloud {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    align-content: flex-start;
  }
  .folder-tag {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    max-width: 100%;
    padding: 3px 4px 3px var(--sp-3);
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-pill);
    background: var(--bg-inset);
    font-size: var(--fs-sm);
    color: var(--text-2);
  }
  .folder-tag.in-use {
    border-color: var(--accent-line);
  }
  .folder-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .folder-use {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
    padding: 0 2px;
  }
  .folder-tag.in-use .folder-use {
    color: var(--accent);
  }
  .tag-x {
    display: grid;
    place-items: center;
    padding: 2px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-3);
    cursor: pointer;
  }
  .tag-x:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }

  /* ===================== 数据表（国籍映射） ===================== */
  .data-table {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
  }
  .data-table table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }
  .data-table th {
    position: sticky;
    top: 0;
    z-index: 1;
    height: calc(var(--control-h) + 4px);
    padding: 0 calc(var(--sp-3) + var(--sp-4) + 1px);
    text-align: left;
    vertical-align: middle;
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
    color: var(--text-2);
    background: var(--bg-surface);
    border-bottom: 1px solid var(--hairline-2);
    user-select: none;
    white-space: nowrap;
  }
  .data-table td {
    padding: 2px var(--sp-3);
    border-bottom: 1px solid var(--hairline);
  }
  .data-table tbody tr:last-child td {
    border-bottom: none;
  }
  .data-table .code-col {
    width: 110px;
  }
  .data-table .act-col {
    width: 88px;
    text-align: right;
  }
  .code-cell {
    padding-left: calc(var(--sp-3) + var(--sp-4) + 1px) !important;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .data-table td select {
    width: 100%;
    min-width: 0;
  }
  .mini-pager {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
  }
  .mini-pager .btn {
    min-height: var(--control-h-sm);
    padding: 3px var(--sp-2);
  }

  /* ===================== 标准二：种族码卡片 ===================== */
  .code-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(330px, 1fr));
    gap: var(--sp-3);
  }
  .code-card {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    padding: var(--sp-3) var(--sp-4) var(--sp-4);
  }
  .code-card.ghost {
    border-style: dashed;
    background: none;
    padding-bottom: var(--sp-3);
  }
  .code-card-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .code-badge {
    display: inline-grid;
    place-items: center;
    min-width: 26px;
    height: 22px;
    padding: 0 6px;
    border-radius: var(--radius-sm);
    background: var(--accent-soft);
    color: var(--accent);
    font-family: var(--mono);
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
  }
  .code-badge.muted {
    background: var(--hairline);
    color: var(--text-3);
  }
  .code-card-title {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
    color: var(--text-2);
  }
  .field {
    margin-top: var(--sp-3);
  }
  .field > .field-label {
    display: block;
    margin-bottom: var(--sp-1);
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  .field-hint {
    margin: var(--sp-1) 0;
    font-size: var(--fs-sm);
    color: var(--text-3);
  }
  .inline-field {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }
  .inline-field > .field-label {
    margin: 0;
    white-space: nowrap;
  }
  .inline-field > select {
    flex: 1;
    min-width: 0;
  }
  .prefer-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: var(--sp-2) 0;
  }
  .prefer-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 2px var(--sp-2);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    font-size: var(--fs-sm);
  }
  .prefer-row.bad {
    border-color: var(--danger);
  }
  .prefer-idx {
    width: 16px;
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text-3);
    text-align: center;
  }
  .prefer-dir {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .prefer-row.bad .prefer-dir {
    color: var(--danger);
  }
  .icon-btn {
    display: grid;
    place-items: center;
    padding: 2px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-3);
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    color: var(--text-1);
    background: var(--hairline);
  }
  .icon-btn.danger:hover:not(:disabled) {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .icon-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }

  /* ===================== 标准三：规则块主从布局 ===================== */
  .block-layout {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 262px minmax(0, 1fr);
  }
  .block-list {
    min-height: 0;
    overflow: auto;
    padding: var(--sp-4);
    border-right: 1px solid var(--hairline);
    background: var(--bg-inset);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .block-items {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0 calc(-1 * var(--sp-2));
  }
  .block-item {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    padding: var(--sp-2) var(--sp-2);
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: none;
    color: var(--text-2);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .block-item:hover:not(.active) {
    background: var(--hairline);
    color: var(--text-1);
  }
  .block-item.active {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--text-1);
  }
  .block-kind {
    flex-shrink: 0;
    padding: 1px 5px;
    border-radius: var(--radius-pill);
    background: var(--bg-surface);
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .kind-default {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .kind-nation {
    color: var(--text-2);
  }
  .kind-unknown {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .block-key {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .block-count {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .block-empty {
    padding: var(--sp-3) var(--sp-2);
  }
  .block-detail {
    min-height: 0;
    overflow: auto;
    padding: var(--sp-4) var(--sp-5) var(--sp-6);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .block-detail-head {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    flex-wrap: wrap;
  }
  .block-detail-title {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }
  .block-detail-key {
    font-size: var(--fs-lg);
    font-weight: var(--fw-semibold);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .block-delete {
    margin-left: auto;
  }
  .block-desc {
    margin: 0 0 var(--sp-2);
    color: var(--text-3);
    font-size: var(--fs-sm);
    line-height: 1.6;
  }
  .block-extra {
    color: var(--warn);
  }
  .rule-code-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .rule-code-card {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    padding: var(--sp-2) var(--sp-3) var(--sp-3);
  }
  .rule-code-card.missing {
    border-style: dashed;
    background: none;
    padding-bottom: var(--sp-2);
  }
  .rule-code-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .rule-missing-text {
    flex: 1;
    min-width: 0;
  }
  .rule-add-btn {
    margin-left: auto;
  }
  .rule-type-select {
    width: 144px;
    flex-shrink: 0;
  }
  .rule-delete-btn {
    margin-left: auto;
  }
  .rule-hint {
    margin: var(--sp-2) 0 0;
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  .rule-line {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    margin-top: var(--sp-2);
  }
  .rule-line-label {
    flex-shrink: 0;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .rule-line select {
    flex: 1;
    min-width: 0;
  }
  .dir-chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
    margin-top: var(--sp-2);
  }
  .dir-chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px 4px 2px var(--sp-2);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    font-size: var(--fs-sm);
  }
  .dir-chip.bad {
    border-color: var(--danger);
    color: var(--danger);
  }
  .dir-chips .compact-add {
    flex-basis: 100%;
    margin-top: 0;
  }
  .weight-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin-top: var(--sp-2);
  }
  .weight-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 92px auto;
    gap: var(--sp-2);
    align-items: center;
  }
  .weight-row input {
    width: 100%;
  }
  .rule-warning {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-top: var(--sp-2);
    color: var(--warn);
    font-size: var(--fs-sm);
  }
  .rule-warning .btn {
    margin-left: auto;
  }

  /* ===================== 标准三：全局设置 ===================== */
  .settings-grid {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .setting-card {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    padding: var(--sp-4);
  }
  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
  }
  .setting-title {
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
    color: var(--text-1);
  }
  .setting-desc {
    margin: var(--sp-1) 0 0;
    color: var(--text-3);
    font-size: var(--fs-sm);
    line-height: 1.6;
  }
  .choice-row {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-3);
    margin-top: var(--sp-3);
  }
  .choice {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-3);
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    text-align: left;
    cursor: pointer;
    color: var(--text-2);
  }
  .choice:hover:not(.active) {
    border-color: var(--hairline-3);
    color: var(--text-1);
  }
  .choice.active {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--text-1);
  }
  .choice-title {
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
  }
  .choice-desc {
    font-size: var(--fs-sm);
    color: var(--text-3);
    line-height: 1.55;
  }
  .passthrough-stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2) var(--sp-4);
    margin-top: var(--sp-3);
    color: var(--text-3);
    font-size: var(--fs-sm);
  }

  /* ===================== 源码模式 ===================== */
  .source-wrap {
    padding: var(--sp-4) var(--sp-5) var(--sp-5);
    display: flex;
    flex-direction: column;
  }
  .source-editor {
    flex: 1;
    min-height: 0;
    width: 100%;
    resize: none;
    padding: var(--sp-4) var(--sp-5);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    color: var(--text-2);
    font-family: var(--mono);
    font-size: var(--fs-sm);
    line-height: 1.65;
    white-space: pre;
    overflow: auto;
    outline: none;
  }
  .source-editor:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--focus-ring);
  }

  @media (max-width: 900px) {
    .rule-nav {
      width: 178px;
    }
    .stat-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .block-layout {
      grid-template-columns: 210px minmax(0, 1fr);
    }
  }
</style>
