use std::path::PathBuf;

pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
        }
        table[i as usize] = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn dos_datetime(secs: u64) -> (u16, u16) {
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    if y < 1980 {
        return (0, 0x21);
    }
    let h = (secs / 3600) % 24;
    let mi = (secs / 60) % 60;
    let s = secs % 60;
    let time = ((h as u16) << 11) | ((mi as u16) << 5) | ((s as u16) / 2);
    let date = (((y - 1980) as u16) << 9) | ((m as u16) << 5) | (d as u16);
    (time, date)
}

/// 将 (相对路径, 磁盘文件) 列表打包为 zip 字节流。
pub fn build_zip(files: &[(String, PathBuf)]) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut central: Vec<(String, u32, u32, u16, u16, u32)> = Vec::new();
    for (rel, path) in files {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let crc = crc32(&data);
        let mtime = std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (dos_time, dos_date) = dos_datetime(mtime);
        let local_off = buf.len() as u32;
        let name = rel.as_bytes();
        buf.extend(&0x04034B50u32.to_le_bytes());
        buf.extend(&20u16.to_le_bytes());
        buf.extend(&0x0800u16.to_le_bytes());
        buf.extend(&0u16.to_le_bytes());
        buf.extend(&dos_time.to_le_bytes());
        buf.extend(&dos_date.to_le_bytes());
        buf.extend(&crc.to_le_bytes());
        buf.extend(&(data.len() as u32).to_le_bytes());
        buf.extend(&(data.len() as u32).to_le_bytes());
        buf.extend(&(name.len() as u16).to_le_bytes());
        buf.extend(&0u16.to_le_bytes());
        buf.extend(name);
        buf.extend(&data);
        central.push((rel.clone(), crc, local_off, dos_time, dos_date, data.len() as u32));
    }
    let cd_start = buf.len() as u32;
    for (name, crc, off, dos_time, dos_date, size) in &central {
        let nb = name.as_bytes();
        buf.extend(&0x02014B50u32.to_le_bytes());
        buf.extend(&0x0314u16.to_le_bytes());
        buf.extend(&20u16.to_le_bytes());
        buf.extend(&0x0800u16.to_le_bytes());
        buf.extend(&0u16.to_le_bytes());
        buf.extend(&dos_time.to_le_bytes());
        buf.extend(&dos_date.to_le_bytes());
        buf.extend(&crc.to_le_bytes());
        buf.extend(&size.to_le_bytes());
        buf.extend(&size.to_le_bytes());
        buf.extend(&(nb.len() as u16).to_le_bytes());
        buf.extend(&0u16.to_le_bytes());
        buf.extend(&0u16.to_le_bytes());
        buf.extend(&0u16.to_le_bytes());
        buf.extend(&0u16.to_le_bytes());
        buf.extend(&0u32.to_le_bytes());
        buf.extend(&off.to_le_bytes());
        buf.extend(nb);
    }
    let cd_size = (buf.len() - cd_start as usize) as u32;
    buf.extend(&0x06054B50u32.to_le_bytes());
    buf.extend(&0u16.to_le_bytes());
    buf.extend(&0u16.to_le_bytes());
    buf.extend(&(central.len() as u16).to_le_bytes());
    buf.extend(&(central.len() as u16).to_le_bytes());
    buf.extend(&cd_size.to_le_bytes());
    buf.extend(&cd_start.to_le_bytes());
    buf.extend(&0u16.to_le_bytes());
    Ok(buf)
}