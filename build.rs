//! Embed the user-provided DesktopCraft icon in the Windows executable.
//! Produces a standard Windows .res file without external RC tools/crates.
//! On other targets the window icon still comes from miniquad's RGBA arrays.
use std::{env, fs, path::PathBuf};

#[derive(Debug)]
struct IconEntry { width: u8, height: u8, colors: u8, planes: u16, bits: u16, bytes: Vec<u8> }
fn le16(input: &[u8]) -> u16 { u16::from_le_bytes([input[0], input[1]]) }
fn le32(input: &[u8]) -> u32 { u32::from_le_bytes([input[0], input[1], input[2], input[3]]) }
fn entries(ico: &[u8]) -> Result<Vec<IconEntry>, String> {
    if ico.len() < 6 || le16(&ico[0..2]) != 0 || le16(&ico[2..4]) != 1 {
        return Err("invalid ICO header".into());
    }
    let count = le16(&ico[4..6]) as usize;
    if count == 0 || ico.len() < 6 + count * 16 { return Err("empty or truncated ICO".into()); }
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let e = &ico[6 + i * 16..6 + (i + 1) * 16];
        let len = le32(&e[8..12]) as usize;
        let offset = le32(&e[12..16]) as usize;
        if offset.checked_add(len).is_none_or(|end| end > ico.len()) { return Err("invalid ICO entry bounds".into()); }
        result.push(IconEntry {
            width: e[0], height: e[1], colors: e[2],
            planes: le16(&e[4..6]), bits: le16(&e[6..8]),
            bytes: ico[offset..offset + len].to_vec(),
        });
    }
    Ok(result)
}
fn push_res(output: &mut Vec<u8>, kind: u16, name: u16, data: &[u8]) {
    // Win32 .res: DataSize, HeaderSize, type and name as numeric IDs,
    // DataVersion, MemoryFlags, LanguageId, Version, Characteristics.
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(&32u32.to_le_bytes());
    output.extend_from_slice(&0xffffu16.to_le_bytes());
    output.extend_from_slice(&kind.to_le_bytes());
    output.extend_from_slice(&0xffffu16.to_le_bytes());
    output.extend_from_slice(&name.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0x1030u16.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(data);
    while output.len() % 4 != 0 { output.push(0); }
}
/// Win32 VERSIONINFO resource, so Explorer Properties, task manager and
/// installed-app metadata all agree with Cargo's 0.1.0 package version.
fn pad4(buffer: &mut Vec<u8>) {
    while buffer.len() % 4 != 0 { buffer.push(0); }
}
fn version_node(key: &str, value: &[u8], value_length: u16,
    value_type: u16, children: &[Vec<u8>]) -> Vec<u8> {
    let mut output = vec![0; 2]; // wLength is patched after children are written
    output.extend_from_slice(&value_length.to_le_bytes());
    output.extend_from_slice(&value_type.to_le_bytes());
    for unit in key.encode_utf16().chain(std::iter::once(0)) {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    pad4(&mut output);
    output.extend_from_slice(value);
    pad4(&mut output);
    for child in children { output.extend_from_slice(child); }
    assert!(output.len() <= u16::MAX as usize, "VERSIONINFO resource too large");
    let length = output.len() as u16;
    output[..2].copy_from_slice(&length.to_le_bytes());
    output
}
fn version_string(key: &str, value: &str) -> Vec<u8> {
    let units: Vec<_> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<_> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    version_node(key, &bytes, units.len() as u16, 1, &[])
}
fn version_resource(version: &str) -> Vec<u8> {
    let mut nums = [0u16; 4];
    for (i, part) in version.split('.').take(4).enumerate() {
        nums[i] = part.parse().unwrap_or(0);
    }
    let ver_ms = ((nums[0] as u32) << 16) | nums[1] as u32;
    let ver_ls = ((nums[2] as u32) << 16) | nums[3] as u32;
    let values = [
        0xFEEF04BDu32, 0x00010000, ver_ms, ver_ls, ver_ms, ver_ls,
        0x0000003f, 0x00000000, 0x00040004, 1, 0, 0, 0,
    ];
    let fixed: Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let readable = format!("{version}.0");
    let strings = [
        version_string("CompanyName", "DesktopCraft"),
        version_string("FileDescription", "DesktopCraft desktop organizer"),
        version_string("FileVersion", &readable),
        version_string("InternalName", "DesktopCraft"),
        version_string("OriginalFilename", "DesktopCraft.exe"),
        version_string("ProductName", "DesktopCraft"),
        version_string("ProductVersion", &readable),
    ];
    let table = version_node("040904B0", &[], 0, 1, &strings);
    let strings_info = version_node("StringFileInfo", &[], 0, 1, &[table]);
    let translation = version_node("Translation", &[0x09, 0x04, 0xB0, 0x04], 4, 0, &[]);
    let vars = version_node("VarFileInfo", &[], 0, 1, &[translation]);
    version_node("VS_VERSION_INFO", &fixed, fixed.len() as u16, 0, &[strings_info, vars])
}
fn write_resource(ico: &[u8], output: &PathBuf) -> Result<(), String> {
    let icons = entries(ico)?;
    let mut result = Vec::new();
    push_res(&mut result, 0, 0, &[]); // Standard initial null resource.
    let mut group = Vec::with_capacity(6 + icons.len() * 14);
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(icons.len() as u16).to_le_bytes());
    for (index, icon) in icons.iter().enumerate() {
        push_res(&mut result, 3, (index + 1) as u16, &icon.bytes); // RT_ICON
        group.extend_from_slice(&[icon.width, icon.height, icon.colors, 0]);
        group.extend_from_slice(&icon.planes.to_le_bytes());
        group.extend_from_slice(&icon.bits.to_le_bytes());
        group.extend_from_slice(&(icon.bytes.len() as u32).to_le_bytes());
        group.extend_from_slice(&((index + 1) as u16).to_le_bytes());
    }
    push_res(&mut result, 14, 1, &group); // RT_GROUP_ICON
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());
    push_res(&mut result, 16, 1, &version_resource(&version)); // RT_VERSION
    fs::write(output, result).map_err(|e| e.to_string())
}
/// A DesktopCraft started from the build folder (for example with Play) keeps
/// its program file locked, so the next build could not replace it ("Access is
/// denied"). Windows does let a running program's file be renamed, so move a
/// locked copy aside; the running copy keeps working and the build writes a
/// fresh file. Leftovers from earlier builds are removed once they're free.
fn set_aside_locked_program() {
    if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") || !cfg!(windows) { return; }
    let Some(out) = env::var_os("OUT_DIR").map(PathBuf::from) else { return };
    // OUT_DIR is <target>/<profile>/build/<package>-<hash>/out.
    let Some(profile) = out.ancestors().nth(3).map(PathBuf::from) else { return };
    let name = format!("{}.exe", env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "desktopcraft".into()));
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos()).unwrap_or(0);
    for dir in [profile.clone(), profile.join("deps")] {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.file_name().and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&name) && n.ends_with(".in-use")) {
                    let _ = fs::remove_file(&path); // fails quietly while still running
                }
            }
        }
        let program = dir.join(&name);
        if !program.exists() { continue; }
        let locked = fs::OpenOptions::new().write(true).open(&program).is_err();
        if locked {
            let aside = dir.join(format!("{name}.{stamp}.in-use"));
            match fs::rename(&program, &aside) {
                Ok(()) => println!("cargo:warning=DesktopCraft is running from the build folder; its program file was set aside so this build can replace it"),
                Err(e) => println!("cargo:warning=Could not set aside the running program {}: {e}", program.display()),
            }
        }
    }
}
fn main() {
    // Rerun (and so check for a running copy) whenever the program is rebuilt.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=vendor/miniquad/src");
    println!("cargo:rerun-if-changed=vendor/macroquad/src");
    set_aside_locked_program();
    println!("cargo:rerun-if-changed=assets/desktopcraft.ico");
    println!("cargo:rerun-if-changed=assets/desktopcraft-16.rgba");
    println!("cargo:rerun-if-changed=assets/desktopcraft-32.rgba");
    println!("cargo:rerun-if-changed=assets/desktopcraft-64.rgba");
    if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") { return; }
    if env::var("CARGO_CFG_TARGET_ENV").ok().as_deref() != Some("msvc") {
        println!("cargo:warning=Executable icon embedding currently supports Windows MSVC; window icon remains available");
        return;
    }
    let bytes = fs::read("assets/desktopcraft.ico").expect("DesktopCraft ICO asset missing");
    let resource = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("desktopcraft.res");
    write_resource(&bytes, &resource).expect("Unable to embed DesktopCraft icon resources");
    println!("cargo:rustc-link-arg={}", resource.display());
}
