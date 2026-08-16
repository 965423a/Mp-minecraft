//! 注册表:加载嵌入的 registry_pack.bin,JSON 转 NBT,供 Configuration 阶段下发。

use flate2::read::ZlibDecoder;
use std::io::Read;

pub struct RegistryEntry {
    pub registry: String,
    pub key: String,
    pub nbt: Vec<u8>,
}

pub struct Registry {
    pub entries: Vec<RegistryEntry>,
}

pub fn load_registry() -> Registry {
    let raw = include_bytes!("../registry_pack.bin");
    Registry::parse(raw).expect("corrupt registry_pack.bin")
}

impl Registry {
    pub fn parse(raw: &[u8]) -> Option<Registry> {
        if raw.len() < 12 || &raw[0..4] != b"MREG" {
            return None;
        }
        let version = u32::from_le_bytes(raw[4..8].try_into().ok()?);
        if version != 1 {
            return None;
        }
        let count = u32::from_le_bytes(raw[8..12].try_into().ok()?) as usize;
        let mut pos = 12usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let rn = read_u16_string(raw, &mut pos)?;
            let kn = read_u16_string(raw, &mut pos)?;
            let zlen = u32::from_le_bytes(raw[pos..pos + 4].try_into().ok()?) as usize;
            pos += 4;
            let zdata = raw.get(pos..pos + zlen)?;
            pos += zlen;
            let mut dec = ZlibDecoder::new(zdata);
            let mut json = Vec::new();
            dec.read_to_end(&mut json).ok()?;
            let Some(nbt) = json_to_nbt_root(&json) else {
                eprintln!("[registry] NBT convert failed: {rn} {kn}");
                return None;
            };
            entries.push(RegistryEntry {
                registry: rn,
                key: kn,
                nbt,
            });
        }
        Some(Registry { entries })
    }

    pub fn groups(&self) -> Vec<(String, Vec<(String, Vec<u8>)>)> {
        let mut m: std::collections::BTreeMap<String, Vec<(String, Vec<u8>)>> =
            std::collections::BTreeMap::new();
        for e in &self.entries {
            m.entry(e.registry.clone()).or_default().push((e.key.clone(), e.nbt.clone()));
        }
        m.into_iter().collect()
    }
}

fn read_u16_string(raw: &[u8], pos: &mut usize) -> Option<String> {
    let len = u16::from_le_bytes(raw[*pos..*pos + 2].try_into().ok()?) as usize;
    *pos += 2;
    let bytes = raw.get(*pos..*pos + len)?;
    *pos += len;
    String::from_utf8(bytes.to_vec()).ok()
}

/// datapack JSON 转 NBT(root compound,无名字)。规则与 Mojang JsonOps 一致:
/// 对象→compound,数组→list,整数→int,小数→float,布尔→byte,字符串→string。
/// 输出完整 unnamed NBT:类型字节(10) + 键值 + 结束(0)。
fn json_to_nbt_root(json: &[u8]) -> Option<Vec<u8>> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let mut out = Vec::new();
    out.push(10);
    write_compound(&v, &mut out)?;
    Some(out)
}

fn write_compound(v: &serde_json::Value, out: &mut Vec<u8>) -> Option<()> {
    write_compound_at(v, out, "")
}

fn write_compound_at(v: &serde_json::Value, out: &mut Vec<u8>, path: &str) -> Option<()> {
    let obj = v.as_object()?;
    for (k, val) in obj {
        if k.starts_with('#') {
            continue;
        }
        let key_bytes = k.as_bytes();
        if key_bytes.len() > 65535 {
            return None;
        }
        out.push(tag_of(val));
        out.extend_from_slice(&(key_bytes.len() as u16).to_be_bytes());
        out.extend_from_slice(key_bytes);
        if write_value_at(val, out, &format!("{path}/{k}")).is_none() {
            eprintln!("[registry] convert fail at {path}/{k}");
            return None;
        }
    }
    out.push(0);
    Some(())
}

fn tag_of(v: &serde_json::Value) -> u8 {
    match v {
        serde_json::Value::Null => 1,
        serde_json::Value::Bool(_) => 1,
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                3
            } else {
                5
            }
        }
        serde_json::Value::String(_) => 8,
        serde_json::Value::Array(_) => 9,
        serde_json::Value::Object(_) => 10,
    }
}

fn write_value_at(v: &serde_json::Value, out: &mut Vec<u8>, path: &str) -> Option<()> {
    match v {
        serde_json::Value::Null => {}
        serde_json::Value::Bool(b) => out.push(*b as u8),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.extend_from_slice(&(i as i32).to_be_bytes());
            } else {
                out.extend_from_slice(&n.as_f64()?.to_be_bytes());
            }
        }
        serde_json::Value::String(s) => {
            let b = s.as_bytes();
            if b.len() > 65535 {
                return None;
            }
            out.extend_from_slice(&(b.len() as u16).to_be_bytes());
            out.extend_from_slice(b);
        }
        serde_json::Value::Array(a) => {
            let elem = if a.is_empty() { 1 } else { tag_of(&a[0]) };

            out.push(elem);
            out.extend_from_slice(&(a.len() as i32).to_be_bytes());
            for x in a {
                if elem == 8 {
                    if let serde_json::Value::String(s) = x {
                        let b = s.as_bytes();
                        out.extend_from_slice(&(b.len() as u16).to_be_bytes());
                        out.extend_from_slice(b);
                    } else {
                        return None;
                    }
                } else if elem == 3 {
                    out.extend_from_slice(&(x.as_i64()? as i32).to_be_bytes());
                } else if elem == 5 {
                    out.extend_from_slice(&x.as_f64()?.to_be_bytes());
                } else if elem == 1 {
                    out.push(x.as_bool()? as u8);
                } else if elem == 10 {
                    write_compound(x, out)?;
                } else if elem == 9 {
                    let sub: Vec<serde_json::Value> = x
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    if write_value_at(&serde_json::Value::Array(sub), out, path).is_none() {
                        eprintln!("[registry] nested list fail at {path}: {:?}", x);
                        return None;
                    }
                } else {
                    return None;
                }
            }
        }
        serde_json::Value::Object(_) => write_compound(v, out)?,
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_list() {
        let v: serde_json::Value =
            serde_json::from_str(r#"[["minecraft:end_island_decorated"]]"#).unwrap();
        let mut out = Vec::new();
        let ok = write_value_at(&v, &mut out, "/features");
        assert!(ok.is_some(), "nested list failed, out={out:?}");
    }

    #[test]
    fn parses_embedded_pack() {
        let reg = load_registry();
        assert!(!reg.entries.is_empty());
        let groups = reg.groups();
        assert!(!groups.is_empty());
        let total: usize = groups.iter().map(|(_, es)| es.len()).sum();
        assert_eq!(total, reg.entries.len());
        let has_biome = groups.iter().any(|(g, _)| g == "minecraft:worldgen/biome");
        assert!(has_biome, "missing biome registry");
        for (g, es) in &groups {
            assert!(es.iter().all(|(k, _)| k.contains(':')), "bad key in {g}");
            assert!(es.iter().all(|(_, n)| n.len() > 1 && n[0] == 10), "bad NBT root in {g}");
        }
    }
}
