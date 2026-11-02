/** 规则文件纯数据层单测：解析收窄、规则形态、目录引用统计与配置体检。 */
import { describe, expect, it } from "vitest";
import s2Raw from "../../src-tauri/assets/standard2.json?raw";
import s3Raw from "../../src-tauri/assets/standard3.json?raw";
import {
  blockKind,
  folderUsageMap,
  formatDoc,
  healthIssues,
  mappingKind,
  parseRuleFile,
  ruleDirs,
  sortedKeys,
  type S2Doc,
  type S3Doc,
} from "./rules";

const s2Text = JSON.stringify({
  VersionIgnored: 1,
  FolderList: ["SpanMed", "Caucasian"],
  Ethnics: { ESP: "SpanMed", USA: "Caucasian", BAD: "Missing" },
  CodeRules: { "1": { prefer: ["Caucasian"], fallback: "SpanMed" } },
});

const s3Text = JSON.stringify({
  Version: 4,
  Fallback: "skip",
  FolderList: ["Spain", "Mestizo", "NorthernSA", "China"],
  Ethnics: { ESP: "Spain", CHN: "China", BAD: "Missing" },
  DefaultCodes: { ESP: [1, 2], ORPHAN: [3] },
  EthnicityMappingRules: {
    _DEFAULT_: {
      "0": "default",
      "1": { typeOfMapping: "directly", directory: "Mestizo" },
      "2": { typeOfMapping: "random", directory: ["Spain", "Mestizo"] },
      "3": {
        typeOfMapping: "randomWithWeight",
        directory: { ethnicities: ["Mestizo", "NorthernSA"], weights: [66, 33] },
      },
    },
    Spain: { "0": { typeOfMapping: "directly", directory: "Spain" } },
    Mestizo: {},
  },
  FolderMappingFemale: { Mestizo: ["WF/Mestizo"] },
});

describe("规则文件解析", () => {
  it("标准二收窄常见字段并保留未知字段", () => {
    const doc = parseRuleFile("standard2", s2Text) as S2Doc;
    expect(doc.FolderList).toEqual(["SpanMed", "Caucasian"]);
    expect(doc.Ethnics.ESP).toBe("SpanMed");
    expect(doc.CodeRules["1"]).toEqual({ prefer: ["Caucasian"], fallback: "SpanMed" });
    expect(doc.VersionIgnored).toBe(1);
  });

  it("标准三收窄块/默认码并保留类别映射", () => {
    const doc = parseRuleFile("standard3", s3Text) as S3Doc;
    expect(doc.Version).toBe(4);
    expect(doc.Fallback).toBe("skip");
    expect(doc.DefaultCodes.ESP).toEqual([1, 2]);
    expect(Object.keys(doc.EthnicityMappingRules)).toEqual(["_DEFAULT_", "Spain", "Mestizo"]);
    expect(doc.FolderMappingFemale?.Mestizo).toEqual(["WF/Mestizo"]);
  });

  it("非法 JSON / 字段类型错误都抛中文错误", () => {
    expect(() => parseRuleFile("standard2", "{")).toThrow(/JSON 格式错误/);
    expect(() => parseRuleFile("standard2", JSON.stringify({ FolderList: [1] }))).toThrow(
      /FolderList 应为字符串数组/,
    );
    expect(() =>
      parseRuleFile("standard3", JSON.stringify({ DefaultCodes: { ESP: ["1"] } })),
    ).toThrow(/DefaultCodes\[ESP\] 应为数字数组/);
  });

  it("formatDoc 可被再次解析，语义不丢", () => {
    const doc = parseRuleFile("standard3", s3Text);
    const again = parseRuleFile("standard3", formatDoc(doc));
    expect(again).toEqual(doc);
  });

  it("能打开真实的出厂规则文件，且体检无结构问题", () => {
    for (const [name, text] of [
      ["standard2", s2Raw],
      ["standard3", s3Raw],
    ] as const) {
      const doc = parseRuleFile(name, text);
      expect(parseRuleFile(name, formatDoc(doc))).toEqual(doc);
      expect(healthIssues(name, doc)).toEqual([]);
    }
  });
});

describe("规则形态", () => {
  it("识别四种形态与引用目录", () => {
    expect(mappingKind("default")).toBe("default");
    expect(ruleDirs("default")).toEqual([]);

    const direct = { typeOfMapping: "directly", directory: "Spain" };
    expect(mappingKind(direct)).toBe("directly");
    expect(ruleDirs(direct)).toEqual(["Spain"]);

    const random = { typeOfMapping: "random", directory: ["Spain", "China"] };
    expect(mappingKind(random)).toBe("random");
    expect(ruleDirs(random)).toEqual(["Spain", "China"]);

    const weighted = {
      typeOfMapping: "randomWithWeight",
      directory: { ethnicities: ["Mestizo", "NorthernSA"], weights: [66, 33] },
    };
    expect(mappingKind(weighted)).toBe("randomWithWeight");
    expect(ruleDirs(weighted)).toEqual(["Mestizo", "NorthernSA"]);
  });

  it("结构不完整的规则判为未知", () => {
    expect(mappingKind({ typeOfMapping: "randomWithWeight", directory: { ethnicities: ["A"], weights: [] } }))
      .toBeNull();
    expect(mappingKind({ typeOfMapping: "directly" })).toBeNull();
    expect(mappingKind({ typeOfMapping: "nope", directory: "A" })).toBeNull();
  });
});

describe("目录引用与体检", () => {
  it("标准二引用统计覆盖 Ethnics 与 CodeRules", () => {
    const doc = parseRuleFile("standard2", s2Text) as S2Doc;
    const uses = folderUsageMap("standard2", doc);
    expect(uses.get("SpanMed")).toBe(2); // Ethnics.ESP + CodeRules fallback
    expect(uses.get("Caucasian")).toBe(2); // Ethnics.USA + CodeRules prefer
    expect(uses.get("Missing")).toBe(1); // 失效引用也按字面统计，供删除/修复提示
  });

  it("标准三引用统计覆盖块键、规则目录与类别映射", () => {
    const doc = parseRuleFile("standard3", s3Text) as S3Doc;
    const uses = folderUsageMap("standard3", doc);
    expect(uses.get("Spain")).toBe(4); // Ethnics.ESP + 族群块键 + random 项 + Spain 块规则
    expect(uses.get("Mestizo")).toBe(4); // 直接规则 + random 项 + 加权项 + 族群块键
    expect(uses.get("WF/Mestizo")).toBe(1); // FolderMappingFemale 的值
  });

  it("体检能发现重复目录、失效引用、缺默认块与孤儿 DefaultCodes", () => {
    const s2 = parseRuleFile("standard2", s2Text) as S2Doc;
    const issues2 = healthIssues("standard2", s2);
    expect(issues2.map((i) => i.kind)).toContain("badEthnics");

    const s3 = parseRuleFile("standard3", s3Text) as S3Doc;
    s3.FolderList.push("Spain"); // 制造重复
    delete s3.EthnicityMappingRules._DEFAULT_;
    s3.EthnicityMappingRules.UnknownBlock = {}; // 制造未知块键 → 失效引用
    const issues3 = healthIssues("standard3", s3);
    expect(issues3.map((i) => i.kind)).toEqual(
      expect.arrayContaining(["dupFolders", "badEthnics", "badRefs", "missingDefault", "orphanDefaultCodes"]),
    );
  });

  it("块键分类与排序键", () => {
    const doc = parseRuleFile("standard3", s3Text) as S3Doc;
    expect(blockKind(doc, "_DEFAULT_")).toBe("default");
    expect(blockKind(doc, "ESP")).toBe("nation");
    expect(blockKind(doc, "Mestizo")).toBe("folder");
    expect(blockKind(doc, "Unknown")).toBe("unknown");
    expect(sortedKeys({ b: 1, a: 2 })).toEqual(["a", "b"]);
  });
});
