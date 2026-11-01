//! 服务商 API Key 的密文存储：机器指纹派生主密钥 + AES-256-GCM 封装。
//!
//! 【威胁模型】只防“配置文件被转发 / 备份 / 上传后 Key 以明文泄露”。主密钥由固定盐与
//! 机器指纹派生、不落盘，因此文件拷到另一台机器或另一个系统账户就解不开——代价是
//! 换机 / 重装系统后要重填（用户已接受）。同一账户下的恶意程序、逆向二进制取出盐与算法
//! 不在防护范围内，不为它做任何额外努力。
//!
//! 【算法】ring 的 AES-256-GCM：一次调用完成加密与完整性校验，不必自己管 IV 长度、
//! 填充与 tag 校验。主密钥 = SHA-256(固定盐 ‖ 机器指纹)。
//!
//! 【容错口径】解密失败不是错误，是“未配置”：换机、重装、换账户、手改文件都会走到这里。
//! 调用方丢掉该条并记 warn，**绝不阻断启动**。

use base64::Engine as _;
use ring::aead;
use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};
use std::collections::HashMap;
use std::sync::OnceLock;

use super::types::{ProviderConfig, SealedKey};

/// 派生主密钥的固定盐。改动等于作废所有已存 Key，非必要不动。
const KEY_SALT: &[u8] = b"NewGAN-Manager/image-ai/key-store/v1";

/// GCM 随机数长度；AES-256-GCM 的 nonce 固定 12 字节。
const KEY_NONCE_LEN: usize = 12;

/// 机器指纹：启动时算一次并缓存，之后不再起子进程。
fn key_fingerprint() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(read_key_fingerprint)
}

/// 三平台取机器标识；任一环节失败都回落到空串（退化为“同目录混淆密文”），不阻断启动。
fn read_key_fingerprint() -> String {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // GUI 应用起子进程时不要闪控制台窗口。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Ok(out) = std::process::Command::new("reg")
            .args([
                "query",
                r"HKLM\SOFTWARE\Microsoft\Cryptography",
                "/v",
                "MachineGuid",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = text.lines().find(|l| l.contains("MachineGuid")) {
                if let Some(value) = line.split_whitespace().last() {
                    let value = value.trim();
                    if !value.is_empty() {
                        return value.to_string();
                    }
                }
            }
        }
        // reg 被组策略/安全软件禁用时退到「计算机名 + 用户名」：仍能区分机器与账户，
        // 代价是改计算机名后 Key 需重填（与换机同一类后果）。
        return format!(
            "{}|{}",
            std::env::var("COMPUTERNAME").unwrap_or_default(),
            std::env::var("USERNAME").unwrap_or_default()
        );
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = text.lines().find(|l| l.contains("IOPlatformUUID")) {
                if let Some(start) = line.rfind('"') {
                    let head = &line[..start];
                    if let Some(open) = head.rfind('"') {
                        return head[open + 1..].trim().to_string();
                    }
                }
            }
        }
        return String::new();
    }

    #[cfg(target_os = "linux")]
    {
        return std::fs::read_to_string("/etc/machine-id")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    String::new()
}

/// 主密钥：每次现算，不落盘。
fn key_master_key() -> [u8; 32] {
    let mut ctx = digest::Context::new(&digest::SHA256);
    ctx.update(KEY_SALT);
    ctx.update(key_fingerprint().as_bytes());
    let digest = ctx.finish();
    let mut key = [0u8; 32];
    key.copy_from_slice(digest.as_ref());
    key
}

/// 加密一把 Key。空串不该走到这里（清空是“字段置 None”）。
pub fn seal_key(key: &str) -> Result<SealedKey, String> {
    let mut nonce_bytes = [0u8; KEY_NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| "生成随机数失败，无法保存 Key".to_string())?;
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_master_key())
        .map_err(|_| "初始化加密算法失败".to_string())?;
    let sealing = aead::LessSafeKey::new(unbound);
    let mut buf = key.as_bytes().to_vec();
    sealing
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce_bytes),
            aead::Aad::empty(),
            &mut buf,
        )
        .map_err(|_| "加密 Key 失败".to_string())?;
    Ok(SealedKey {
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce_bytes),
        data: base64::engine::general_purpose::STANDARD.encode(&buf),
    })
}

/// 解密；任何一步不成立都返回 None（调用方按“未配置”处理）。
pub fn open_key(sealed: &SealedKey) -> Option<String> {
    let nonce_vec = base64::engine::general_purpose::STANDARD
        .decode(sealed.nonce.as_bytes())
        .ok()?;
    let nonce_bytes: [u8; KEY_NONCE_LEN] = nonce_vec.try_into().ok()?;
    let mut buf = base64::engine::general_purpose::STANDARD
        .decode(sealed.data.as_bytes())
        .ok()?;
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_master_key()).ok()?;
    let opening = aead::LessSafeKey::new(unbound);
    let plain = opening
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce_bytes),
            aead::Aad::empty(),
            &mut buf,
        )
        .ok()?;
    String::from_utf8(plain.to_vec()).ok()
}

/// 启动时把配置里的密文逐条解密成会话内存的 Key 表。
/// 解不开的条目**跳过并记一条 warn**（不打印密文，更不打印明文）。
pub fn load_stored_keys(providers: &[ProviderConfig]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for p in providers {
        let Some(sealed) = p.api_key.as_ref() else {
            continue;
        };
        match open_key(sealed) {
            Some(key) if !key.trim().is_empty() => {
                out.insert(p.id.clone(), key);
            }
            _ => log::warn!(
                target: "image_ai",
                "服务商 {} 的 Key 无法解密（换机/重装或文件被修改），按未配置处理",
                p.id
            ),
        }
    }
    out
}

// ===================== 单元测试：seal/open 往返与篡改拒收 ======================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_then_open_roundtrips_and_rejects_tampered_blob() {
        let sealed = seal_key("sk-test-1234").unwrap();
        assert_eq!(open_key(&sealed).as_deref(), Some("sk-test-1234"));
        // 同一把 Key 两次封装的 nonce 不同（随机），密文也不同。
        let again = seal_key("sk-test-1234").unwrap();
        assert_ne!(sealed.data, again.data);

        // nonce 长度不对 → None
        let bad_nonce = SealedKey {
            nonce: "AAAA".into(),
            data: sealed.data.clone(),
        };
        assert_eq!(open_key(&bad_nonce), None);

        // 密文被改一位 → 校验失败 → None（不返回乱码）
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(sealed.data.as_bytes())
            .unwrap();
        bytes[0] ^= 0x01;
        let tampered = SealedKey {
            nonce: sealed.nonce.clone(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        };
        assert_eq!(open_key(&tampered), None);
    }
}
