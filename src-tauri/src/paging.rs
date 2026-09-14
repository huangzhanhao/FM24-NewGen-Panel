//! 通用分页契约：前端所有「按需拉一页」的查询（日志 / 名单预览 / 快照预览）共用同一返回结构。
//!
//! 放在中立的叶子模块，供 logger 与 commands 共同引用，避免两处各定义一份 total+items。

use serde::Serialize;

/// 分页结果：过滤后总条数 + 当前页条目。camelCase 序列化，与前端 `PageResult<T>` 类型一致。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PageResult<T> {
    pub total: usize,
    pub items: Vec<T>,
}
