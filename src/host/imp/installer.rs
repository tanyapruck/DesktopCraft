use super::*;

// ---- per-user registry (HKEY_CURRENT_USER only) ----
fn reg_set(key: &str, name: Option<&str>, value: &str) -> bool {
    unsafe {
        let mut h: HKEY = ptr::null_mut();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            0,
            ptr::null_mut(),
            0,
            KEY_SET_VALUE,
            ptr::null_mut(),
            &mut h,
            ptr::null_mut(),
        ) != 0
        {
            return false;
        }
        let data = wide(value);
        let name = name.map(wide);
        let ok = RegSetValueExW(
            h,
            name.as_ref().map_or(ptr::null(), |n| n.as_ptr()),
            0,
            REG_SZ,
            data.as_ptr() as _,
            (data.len() * 2) as u32,
        ) == 0;
        RegCloseKey(h);
        ok
    }
}
fn reg_set_dword(key: &str, name: &str, value: u32) -> bool {
    unsafe {
        let mut h: HKEY = ptr::null_mut();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            0,
            ptr::null_mut(),
            0,
            KEY_SET_VALUE,
            ptr::null_mut(),
            &mut h,
            ptr::null_mut(),
        ) != 0 {
            return false;
        }
        let ok=RegSetValueExW(
            h,wide(name).as_ptr(),0,REG_DWORD,
            &value as *const u32 as _,4,
        )==0;
        RegCloseKey(h);
        ok
    }
}
fn reg_get(key: &str, name: &str) -> Option<String> {
    unsafe {
        let mut buf = vec![0u16; 2048];
        let mut size = (buf.len() * 2) as u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(name).as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            buf.as_mut_ptr() as _,
            &mut size,
        );
        (r == 0)
            .then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }
}
fn reg_key_exists(key: &str) -> bool {
    unsafe {
        let mut h: HKEY = ptr::null_mut();
        let ok = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            0,
            winapi::um::winnt::KEY_READ,
            &mut h,
        ) == 0;
        if ok {
            RegCloseKey(h);
        }
        ok
    }
}
fn reg_delete_value(key: &str, name: &str) {
    unsafe {
        RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr());
    }
}
fn reg_delete_tree(key: &str) {
    unsafe {
        RegDeleteTreeW(HKEY_CURRENT_USER, wide(key).as_ptr());
    }
}

/// Recognize the previous startup entry, but never start both brands.
/// A user-initiated toggle replaces old startup registration with new.
pub fn autostart(set: Option<bool>) -> bool {
    if let Some(on) = set {
        if on {
            if let Ok(exe) = std::env::current_exe() {
                reg_set(
                    RUN_KEY,
                    Some(APP_NAME),
                    &format!("\"{}\" --at-sign-in", exe.display()),
                );
                reg_delete_value(RUN_KEY, LEGACY_APP_NAME);
            }
        } else {
            reg_delete_value(RUN_KEY, APP_NAME);
            reg_delete_value(RUN_KEY, LEGACY_APP_NAME);
        }
    }
    reg_get(RUN_KEY, APP_NAME).is_some() || reg_get(RUN_KEY, LEGACY_APP_NAME).is_some()
}

fn shortcut(link: &Path, target: &Path, arguments: &str) -> Result<(), String> {
    use winapi::shared::guiddef::GUID;
    type Ptr = *mut std::ffi::c_void;
    const CLSID_SHELL_LINK: GUID = GUID {
        Data1: 0x00021401,
        Data2: 0,
        Data3: 0,
        Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
    };
    const IID_SHELL_LINK_W: GUID = GUID {
        Data1: 0x000214F9,
        Data2: 0,
        Data3: 0,
        Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
    };
    const IID_PERSIST_FILE: GUID = GUID {
        Data1: 0x0000010B,
        Data2: 0,
        Data3: 0,
        Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
    };
    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(r: Ptr, flags: u32) -> i32;
        fn CoCreateInstance(
            clsid: *const GUID,
            outer: Ptr,
            ctx: u32,
            iid: *const GUID,
            ppv: *mut Ptr,
        ) -> i32;
    }
    unsafe fn vfn(obj: Ptr, i: usize) -> *const std::ffi::c_void {
        *(*(obj as *mut *const *const std::ffi::c_void)).add(i)
    }
    unsafe {
        CoInitializeEx(ptr::null_mut(), 2);
        let mut link_obj = ptr::null_mut();
        if CoCreateInstance(
            &CLSID_SHELL_LINK,
            ptr::null_mut(),
            1,
            &IID_SHELL_LINK_W,
            &mut link_obj,
        ) < 0
        {
            return Err("Could not create the shortcut".into());
        }
        let set_path: extern "system" fn(Ptr, *const u16) -> i32 =
            mem::transmute(vfn(link_obj, 20));
        let set_args: extern "system" fn(Ptr, *const u16) -> i32 =
            mem::transmute(vfn(link_obj, 11));
        let set_dir: extern "system" fn(Ptr, *const u16) -> i32 =
            mem::transmute(vfn(link_obj, 9));
        let set_desc: extern "system" fn(Ptr, *const u16) -> i32 =
            mem::transmute(vfn(link_obj, 7));
        set_path(link_obj, wide(&target.to_string_lossy()).as_ptr());
        set_args(link_obj, wide(arguments).as_ptr());
        if let Some(dir) = target.parent() {
            set_dir(link_obj, wide(&dir.to_string_lossy()).as_ptr());
        }
        set_desc(
            link_obj,
            wide("Organize your Windows desktop into groups").as_ptr(),
        );
        let qi: extern "system" fn(Ptr, *const GUID, *mut Ptr) -> i32 =
            mem::transmute(vfn(link_obj, 0));
        let mut file = ptr::null_mut();
        qi(link_obj, &IID_PERSIST_FILE, &mut file);
        let mut result = Err("Could not save the shortcut".to_string());
        if !file.is_null() {
            let save: extern "system" fn(Ptr, *const u16, i32) -> i32 =
                mem::transmute(vfn(file, 6));
            if save(file, wide(&link.to_string_lossy()).as_ptr(), 1) >= 0 {
                result = Ok(());
            }
            let release: extern "system" fn(Ptr) -> u32 = mem::transmute(vfn(file, 2));
            release(file);
        }
        let release: extern "system" fn(Ptr) -> u32 = mem::transmute(vfn(link_obj, 2));
        release(link_obj);
        result
    }
}
fn start_menu_link_for(name: &str) -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(format!("{name}.lnk"))
}
fn start_menu_link() -> PathBuf {
    start_menu_link_for(APP_NAME)
}

/// After a successful new per-user install, retire only registrations that
/// belong to our prior name. Never remove the old data or installed EXE:
/// keeping it allows manual recovery if a new release misbehaves.
fn retire_legacy_install() {
    if reg_get(UNINSTALL_KEY, "DisplayName").as_deref() != Some(APP_NAME) {
        return; // Avoid orphaning a legacy install if new registration failed.
    }
    reg_delete_value(RUN_KEY, LEGACY_APP_NAME);
    // Previous builds (and the standalone menu-repair script) may have
    // registered the desktop verbs without an Apps & features entry.
    reg_delete_tree(LEGACY_VERB_NEW);
    reg_delete_tree(LEGACY_VERB_SETTINGS);
    if reg_get(LEGACY_UNINSTALL_KEY, "DisplayName").as_deref() == Some(LEGACY_APP_NAME) {
        reg_delete_tree(LEGACY_UNINSTALL_KEY);
    }
    let _ = std::fs::remove_file(start_menu_link_for(LEGACY_APP_NAME));
}

/// Acquire an exclusive per-user installer lock. Two concurrent setup
/// instances must never race in replacing the installed executable.
fn acquire_installer_lock(dir: &Path) -> Result<std::fs::File,String> {
    use std::os::windows::fs::OpenOptionsExt;
    use winapi::shared::winerror::ERROR_SHARING_VIOLATION;
    let path = dir.join(".desktopcraft-installer.lock");
    // The open handle is the lock. Existing sentinel files need no writes.
    std::fs::OpenOptions::new().read(true).share_mode(0).open(&path)
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                std::fs::OpenOptions::new()
                    .read(true).write(true).create(true).share_mode(0).open(&path)
            } else {
                Err(error)
            }
        })
        .map_err(|error| {
            if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) {
                format!("The DesktopCraft installer lock is in use. Close any other DesktopCraft Setup window and try again. ({error})")
            } else {
                format!("Could not open the DesktopCraft installer lock at {}. ({error})", path.display())
            }
        })
}

#[cfg(test)]
#[test]
fn installer_lock_reopens_a_read_only_file_and_excludes_a_live_holder() {
    let dir = std::env::temp_dir().join(format!(
        "desktopcraft-lock-test-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    ));
    std::fs::create_dir(&dir).unwrap();
    let lock_path = dir.join(".desktopcraft-installer.lock");
    let first = acquire_installer_lock(&dir).unwrap();
    let error = acquire_installer_lock(&dir).unwrap_err();
    assert!(error.contains("(os error 32)"));
    drop(first);

    let original_permissions = std::fs::metadata(&lock_path).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&lock_path, readonly).unwrap();
    let reopened = acquire_installer_lock(&dir).map(drop);
    std::fs::set_permissions(&lock_path, original_permissions).unwrap();
    std::fs::remove_file(lock_path).unwrap();
    std::fs::remove_dir(dir).unwrap();
    reopened.unwrap();
}

// Windows can temporarily deny metadata access while the previous process
// closes a journal, a filesystem filter scans it or deletion is pending.
// An unknown state is never equivalent to a missing recovery journal.
fn retryable_journal_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5 | 32 | 33 | 303))
}

fn probe_recovery_journal(
    journal: &Path,
    mut inspect: impl FnMut() -> std::io::Result<bool>,
    mut retry: impl FnMut() -> bool,
) -> Result<bool, String> {
    loop {
        match inspect() {
            Ok(pending) => return Ok(pending),
            Err(error) if retryable_journal_error(&error) && retry() => continue,
            Err(error) => return Err(format!(
                "Cannot inspect the native recovery journal at {}: {}. Close DesktopCraft normally and retry with the same Windows account; recovery data was not removed.",
                journal.display(), error)),
        }
    }
}

fn inspect_recovery_journal_until(journal: &Path, deadline: std::time::Instant) -> Result<bool, String> {
    probe_recovery_journal(journal, || journal.try_exists(), || {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() { return false; }
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(200)));
        true
    })
}

#[cfg(test)]
#[test]
fn installer_journal_retry_never_treats_access_denied_as_absent() {
    use std::collections::VecDeque;
    use std::io::Error;
    let journal = Path::new("native-inbox-recovery.json");
    let mut probes = VecDeque::from([Err(Error::from_raw_os_error(5)),
        Err(Error::from_raw_os_error(32)), Err(Error::from_raw_os_error(303)), Ok(false)]);
    let mut retries = 0;
    assert!(!probe_recovery_journal(journal, || probes.pop_front().unwrap(),
        || { retries += 1; true }).unwrap());
    assert_eq!(retries, 3);
    let mut probes = VecDeque::from([Err(Error::from_raw_os_error(5)), Ok(true)]);
    assert!(probe_recovery_journal(journal, || probes.pop_front().unwrap(), || true).unwrap());
    let mut retries = 0;
    let result = probe_recovery_journal(journal, || Err(Error::from_raw_os_error(5)), || {
        retries += 1; retries < 3
    });
    assert!(result.is_err());
    assert_eq!(retries, 3);
    assert!(probe_recovery_journal(journal, || Err(Error::from_raw_os_error(123)),
        || panic!("invalid path must not be retried")).is_err());
}

/// All installer entry points (manual first upgrade included) are subject
/// to the same live-Explorer safety barrier as the GitHub updater. No bytes
/// or registry entries are touched until the previous UI exits, its
/// recovery journal is gone and the Explorer hook mapping is detached.
fn wait_for_installer_quiescence() -> Result<(),String> {
    let journal=data_dir().join("native-inbox-recovery.json");
    if instance_running() {
        send_command("exit").map_err(|e|
            format!("Cannot request orderly DesktopCraft shutdown: {e}"))?;
    }
    let started=std::time::Instant::now();
    let deadline=started+std::time::Duration::from_secs(55);
    loop {
        let pending=inspect_recovery_journal_until(&journal, deadline)?;
        // The metadata probe may have waited while the former GUI exited.
        let running=instance_running();
        let policy=crate::native_filter_host::native_policy_status();
        if !running && !pending && policy["mapping_absence_confirmed"]==true {return Ok(());}
        if !running && pending {
            return Err(format!("Native Desktop recovery pending at {}; install refused",
                journal.display()));
        }
        if started.elapsed()>=std::time::Duration::from_secs(55) {
            return Err("DesktopCraft/Explorer did not detach safely before installer deadline; existing version remains unchanged".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

fn install_string_registry_keys() -> Vec<(String,String)> {
    vec![
        (UNINSTALL_KEY.into(),"DisplayName".into()),
        (UNINSTALL_KEY.into(),"DisplayVersion".into()),
        (UNINSTALL_KEY.into(),"Publisher".into()),
        (UNINSTALL_KEY.into(),"InstallLocation".into()),
        (UNINSTALL_KEY.into(),"DisplayIcon".into()),
        (UNINSTALL_KEY.into(),"UninstallString".into()),
        (VERB_NEW.into(),"MUIVerb".into()),
        (VERB_NEW.into(),"Icon".into()),
        (format!(r"{VERB_NEW}\command"),"".into()),
        (VERB_SETTINGS.into(),"MUIVerb".into()),
        (VERB_SETTINGS.into(),"Icon".into()),
        (format!(r"{VERB_SETTINGS}\command"),"".into()),
        (VERB_PIN_FILE.into(),"MUIVerb".into()),
        (VERB_PIN_FILE.into(),"Icon".into()),
        (format!(r"{VERB_PIN_FILE}\command"),"".into()),
        (RUN_KEY.into(),APP_NAME.into()),
        (RUN_KEY.into(),LEGACY_APP_NAME.into()),
    ]
}
fn get_reg_dword(key:&str,name:&str)->Option<u32>{
    unsafe {
        let mut result=0u32;
        let mut bytes=4u32;
        let status=RegGetValueW(HKEY_CURRENT_USER,
            wide(key).as_ptr(),wide(name).as_ptr(),
            0x10,ptr::null_mut(),&mut result as *mut u32 as _,
            &mut bytes);
        (status==0 && bytes==4).then_some(result)
    }
}

fn required_registry_string(key:&str,name:Option<&str>,value:&str)
    ->Result<(),String> {
    if !reg_set(key,name,value) ||
        reg_get(key,name.unwrap_or(""))!=Some(value.to_owned()) {
        return Err(format!("Cannot verify required per-user registration at {key}"));
    }
    Ok(())
}

fn restore_registry_snapshot(
    strings:&[(String,String,Option<String>)],
    dwords:&[(String,Option<u32>)],
    existing_keys:&[(&str,bool)],
)->Result<(),String> {
    let mut failures=0usize;
    for (key,name,value) in strings {
        match value {
            Some(prior)=>{
                if !reg_set(key,if name.is_empty(){None}else{Some(name)},prior) {
                    failures+=1;
                }
            }
            None=>reg_delete_value(key,name),
        }
    }
    for (name,value) in dwords {
        match value {
            Some(prior)=>{
                if !reg_set_dword(UNINSTALL_KEY,name,*prior) {
                    failures+=1;
                }
            }
            None=>reg_delete_value(UNINSTALL_KEY,name),
        }
    }
    for &(key,existed) in existing_keys {
        if !existed {
            reg_delete_tree(key);
            if reg_key_exists(key) {
                failures+=1;
            }
        }
    }
    if failures>0 {
        Err(format!("{failures} registry rollback operations failed"))
    }else{Ok(())}
}

struct LiveInstallTx {
    source: PathBuf,
    dir: PathBuf,
    exe: PathBuf,
    previous: PathBuf,
    notices_path: PathBuf,
    link: PathBuf,
    tx_marker: PathBuf,
    old_installed: bool,
    old_notices: Option<Vec<u8>>,
    old_link: Option<Vec<u8>>,
    original_values: Vec<(String, String, Option<String>)>,
    original_flags: Vec<(String, Option<u32>)>,
    existing_keys: Vec<(&'static str, bool)>,
}

impl InstallOps for LiveInstallTx {
    fn swap_executable(&mut self) -> Result<(), String> {
        if self.source != self.exe {
            if self.old_installed {
                install_executable_staged(&self.exe, &self.previous)
                    .map_err(|e| format!("Cannot back up the installed version: {e}"))?;
            }
            install_executable_staged(&self.source, &self.exe)?;
        }
        Ok(())
    }
    fn rollback_executable(&mut self) -> Result<(), String> {
        if self.source == self.exe {
            return Ok(());
        }
        if self.old_installed && self.previous.is_file() {
            install_executable_staged(&self.previous, &self.exe)
        } else if !self.old_installed && self.exe.is_file() {
            std::fs::remove_file(&self.exe).map_err(|e| format!("new EXE cleanup: {e}"))
        } else {
            Ok(())
        }
    }
    fn write_notices(&mut self) -> Result<(), String> {
        std::fs::write(&self.notices_path, crate::THIRD_PARTY_NOTICES.as_bytes())
            .map_err(|e| format!("Could not write third-party notices: {e}"))
    }
    fn rollback_notices(&mut self) -> Result<(), String> {
        match &self.old_notices {
            Some(bytes) => std::fs::write(&self.notices_path, bytes).map_err(|e| e.to_string()),
            None if self.notices_path.exists() => {
                std::fs::remove_file(&self.notices_path).map_err(|e| e.to_string())
            }
            None => Ok(()),
        }
    }
    fn write_shortcut(&mut self) -> Result<(), String> {
        shortcut(&self.link, &self.exe, "")
    }
    fn rollback_shortcut(&mut self) -> Result<(), String> {
        match &self.old_link {
            Some(bytes) => std::fs::write(&self.link, bytes).map_err(|e| e.to_string()),
            None if self.link.exists() => {
                std::fs::remove_file(&self.link).map_err(|e| e.to_string())
            }
            None => Ok(()),
        }
    }
    fn write_registry(&mut self, _start_at_login: bool) -> Result<(), String> {
        let quoted = format!("\"{}\"", self.exe.display());
        for (key, name, value) in [
            (UNINSTALL_KEY, Some("DisplayName"), APP_NAME.to_owned()),
            (UNINSTALL_KEY, Some("DisplayVersion"), env!("CARGO_PKG_VERSION").to_owned()),
            (UNINSTALL_KEY, Some("Publisher"), APP_NAME.to_owned()),
            (UNINSTALL_KEY, Some("InstallLocation"), self.dir.to_string_lossy().to_string()),
            (UNINSTALL_KEY, Some("DisplayIcon"), self.exe.to_string_lossy().to_string()),
            (UNINSTALL_KEY, Some("UninstallString"), format!("{quoted} --uninstall")),
            (VERB_NEW, Some("MUIVerb"), "New group here".into()),
            (VERB_NEW, Some("Icon"), self.exe.to_string_lossy().to_string()),
            (VERB_SETTINGS, Some("MUIVerb"), "DesktopCraft settings".into()),
            (VERB_SETTINGS, Some("Icon"), self.exe.to_string_lossy().to_string()),
            (VERB_PIN_FILE, Some("MUIVerb"), "Pin to DesktopCraft".into()),
            (VERB_PIN_FILE, Some("Icon"), self.exe.to_string_lossy().to_string()),
        ] {
            required_registry_string(key, name, &value)?;
        }
        for (key, command) in [
            (format!(r"{VERB_NEW}\command"), format!("{quoted} --command new-group")),
            (format!(r"{VERB_SETTINGS}\command"), format!("{quoted} --command settings")),
            (format!(r"{VERB_PIN_FILE}\command"), format!("{quoted} --pin-file \"%1\"")),
        ] {
            required_registry_string(&key, None, &command)?;
        }
        if !reg_set_dword(UNINSTALL_KEY, "NoModify", 1)
            || !reg_set_dword(UNINSTALL_KEY, "NoRepair", 1)
        {
            return Err("Uninstall registry flags failed".into());
        }
        Ok(())
    }
    fn rollback_registry(&mut self) -> Result<(), String> {
        restore_registry_snapshot(&self.original_values, &self.original_flags, &self.existing_keys)
    }
    fn apply_autostart(&mut self, start_at_login: bool) -> Result<(), String> {
        if start_at_login {
            let quoted = format!("\"{}\"", self.exe.display());
            required_registry_string(RUN_KEY, Some(APP_NAME), &format!("{quoted} --at-sign-in"))?;
        } else {
            reg_delete_value(RUN_KEY, APP_NAME);
            if reg_get(RUN_KEY, APP_NAME).is_some() {
                return Err("Autostart was not disabled as requested".into());
            }
        }
        Ok(())
    }
    fn rollback_autostart(&mut self) -> Result<(), String> {
        let mut failures = 0usize;
        for (key, name, prior) in self.original_values.iter().filter(|(k, _, _)| k == RUN_KEY) {
            match prior {
                Some(v) => {
                    if !reg_set(key, Some(name), v) { failures += 1; }
                }
                None => reg_delete_value(key, name),
            }
        }
        if failures > 0 { Err("autostart restore failed".into()) } else { Ok(()) }
    }
}

/// Pending installation transaction handle: allows rolling back EXE,
/// notices, shortcut, and registry if post-install launch fails.
pub struct InstallCommit {
    tx: LiveInstallTx,
    _exclusive_install_lock: std::fs::File,
    desktop_instance: Option<DesktopInstance>,
}

impl InstallCommit {
    pub fn exe(&self) -> &Path {
        &self.tx.exe
    }
    pub fn release_desktop_instance(&mut self) {
        self.desktop_instance.take();
    }
    pub fn commit(self) -> Result<(), String> {
        retire_legacy_install();
        std::fs::remove_file(&self.tx.tx_marker)
            .map_err(|e| format!("Install succeeded but could not clear transaction marker: {e}"))
    }
    pub fn rollback(mut self) -> Result<(), String> {
        self.desktop_instance.take();
        let result = rollback_install_stages(&mut self.tx, 4);
        if result.is_ok() {
            let _ = std::fs::remove_file(&self.tx.tx_marker);
        }
        result
    }
}

/// Per-user install (no administrator rights): copy the program, add
/// third-party notices, a Start menu shortcut, an Apps & features entry,
/// and desktop/file context-menu registrations atomically with rollback.
pub fn install(start_at_login: bool) -> Result<InstallCommit, String> {
    use std::io::Write;
    let source = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = dir.join(format!("{APP_NAME}.exe"));
    let _exclusive_install_lock = acquire_installer_lock(&dir)?;
    let tx_marker = dir.join(".desktopcraft-install-tx.json");
    if tx_marker.try_exists().map_err(|e| e.to_string())? {
        return Err(format!(
            "An incomplete DesktopCraft installation transaction exists at {}; inspect or restore the previous installation before retrying.",
            tx_marker.display()
        ));
    }
    wait_for_installer_quiescence()?;
    // A launch that wins the desktop-instance mutex cancels this upgrade.
    let desktop_instance = claim_desktop_instance()
        .ok_or("DesktopCraft started during installer preflight; close it and retry")?;
    let journal = data_dir().join("native-inbox-recovery.json");
    if inspect_recovery_journal_until(&journal,
        std::time::Instant::now()+std::time::Duration::from_secs(5))?
        || crate::native_filter_host::native_policy_status()["mapping_absence_confirmed"] != true {
        return Err("Explorer detach and recovery could not be confirmed; installation refused".into());
    }

    // Cancel a detached cleanup from an earlier uninstall under the same lock.
    match std::fs::remove_file(dir.join(".desktopcraft-uninstall-token")) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(format!("Cannot cancel previous uninstall cleanup: {e}")),
    }

    // Snapshot registration and prior files before activating anything.
    let notices_path = dir.join("THIRD-PARTY-NOTICES.txt");
    let old_notices = match std::fs::read(&notices_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Notices backup failed: {error}")),
    };
    let link = start_menu_link();
    let old_link = match std::fs::read(&link) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Start Menu backup failed: {error}")),
    };
    let original_values = install_string_registry_keys().iter().map(|(key, name)|
        (key.clone(), name.clone(), reg_get(key, name))).collect::<Vec<_>>();
    let original_flags = ["NoModify", "NoRepair"].iter().map(|name|
        (name.to_string(), get_reg_dword(UNINSTALL_KEY, name))).collect::<Vec<_>>();
    let existing_keys = vec![
        (UNINSTALL_KEY, reg_key_exists(UNINSTALL_KEY)),
        (VERB_NEW, reg_key_exists(VERB_NEW)),
        (VERB_SETTINGS, reg_key_exists(VERB_SETTINGS)),
        (VERB_PIN_FILE, reg_key_exists(VERB_PIN_FILE)),
    ];
    let old_installed = exe.is_file();
    let previous = dir.join(format!("{APP_NAME}.previous.exe"));

    // Durable transaction marker written before any binary/registry write.
    let marker_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "old_installed": old_installed,
    })).map_err(|e| e.to_string())?;
    let mut marker_file = std::fs::OpenOptions::new()
        .create_new(true).write(true).open(&tx_marker)
        .map_err(|e| format!("Cannot create install transaction marker: {e}"))?;
    if let Err(e) = marker_file.write_all(&marker_bytes).and_then(|()| marker_file.sync_all()) {
        drop(marker_file);
        let _ = std::fs::remove_file(&tx_marker);
        return Err(format!("Cannot flush install transaction marker: {e}"));
    }
    drop(marker_file);

    let mut tx = LiveInstallTx {
        source,
        dir,
        exe,
        previous,
        notices_path,
        link,
        tx_marker: tx_marker.clone(),
        old_installed,
        old_notices,
        old_link,
        original_values,
        original_flags,
        existing_keys,
    };
    if let Err(error) = run_install_transaction(&mut tx, start_at_login) {
        if !error.contains("Rollback:") || error.ends_with("Rollback: attempt completed") {
            let _ = std::fs::remove_file(&tx_marker);
        }
        return Err(error);
    }
    Ok(InstallCommit {
        tx,
        _exclusive_install_lock,
        desktop_instance: Some(desktop_instance),
    })
}
/// The version installed for this account (from its Settings > Apps
/// entry), when its program file is really there; None when not installed.
pub fn installed_version() -> Option<String> {
    if !install_dir().join(format!("{APP_NAME}.exe")).exists() {
        return None;
    }
    Some(reg_get(UNINSTALL_KEY, "DisplayVersion").unwrap_or_default())
}
/// Wait for the uninstaller to exit, then remove only known files and registrations.
/// The generation token, written under the installer lock, prevents a stale cleanup
/// from deleting a later installation. Keep the lock sentinel to avoid a remove/open race.
fn spawn_uninstall_cleanup(
    dir: &Path, parent_pid: u32, uninstall_key: Option<&str>, token: &str, log: &Path,
) -> Result<std::process::Child, String> {
    use std::os::windows::process::CommandExt;
    if !dir.is_absolute() || dir.file_name().and_then(|v| v.to_str()) != Some(APP_NAME) {
        return Err("Unexpected uninstall directory; cleanup refused".into());
    }
    let system_root = std::env::var_os("SystemRoot").ok_or("Windows system root unavailable")?;
    let powershell = PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let script = r#"
$ErrorActionPreference = 'Stop'
$dir = $env:DESKTOPCRAFT_UNINSTALL_DIR
$log = $env:DESKTOPCRAFT_UNINSTALL_LOG
$handle = $null
$mutex = $null
$ok = $false
$reason = ''
function Record([string]$message) {
    Add-Content -LiteralPath $log -Value ((Get-Date -Format o) + ' ' + $message) -Encoding UTF8
}
function Remove-KnownFile([string]$file) {
    for ($attempt = 0; $attempt -lt 12; $attempt++) {
        try {
            if (Test-Path -LiteralPath $file) {
                $item = Get-Item -LiteralPath $file -Force
                if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                    throw 'Unexpected directory or reparse point at known file path'
                }
                Remove-Item -LiteralPath $file -Force
            }
            if (!(Test-Path -LiteralPath $file)) { return }
            throw 'File still exists after removal'
        } catch {
            if ($attempt -eq 11) { throw }
            Start-Sleep -Milliseconds 500
        }
    }
}
try {
    Record 'Uninstall cleanup started'
    $parentId = [int]$env:DESKTOPCRAFT_UNINSTALL_PID
    if ($parentId -gt 0) {
        $parent = Get-Process -Id $parentId -ErrorAction SilentlyContinue
        if ($parent) { Wait-Process -InputObject $parent -Timeout 30 }
    }
    if (!(Test-Path -LiteralPath $dir -PathType Container)) { throw 'Installation directory missing' }
    if ((Get-Item -LiteralPath $dir -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Reparse-point installation directory refused' }
    $lock = Join-Path $dir '.desktopcraft-installer.lock'
    if (Test-Path -LiteralPath $lock) {
        $handle = [IO.File]::Open($lock,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::None)
    } else {
        $handle = [IO.File]::Open($lock,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    }
    $marker = Join-Path $dir '.desktopcraft-uninstall-token'
    if (!(Test-Path -LiteralPath $marker -PathType Leaf) -or
        [IO.File]::ReadAllText($marker) -cne $env:DESKTOPCRAFT_UNINSTALL_TOKEN) {
        throw 'Uninstall superseded by another operation'
    }
    if (Test-Path -LiteralPath (Join-Path $dir '.desktopcraft-install-tx.json')) { throw 'Install transaction pending' }
    if ($parentId -gt 0) {
        $created = $false
        $mutex = New-Object System.Threading.Mutex($false,'Local\DesktopSpacesHost',[ref]$created)
        if (!$created) { throw 'DesktopCraft started again; close it and retry uninstall' }
    }
    # Retain the main executable and its registration until auxiliary files are removed.
    foreach ($name in @('DesktopCraft.previous.exe','THIRD-PARTY-NOTICES.txt','DesktopCraft.exe')) {
        Remove-KnownFile (Join-Path $dir $name)
    }
    # Only after successful file removal, remove exact per-user registration.
    if ($env:DESKTOPCRAFT_UNINSTALL_RUN_KEY) {
        $run = 'HKCU:\' + $env:DESKTOPCRAFT_UNINSTALL_RUN_KEY
        if (Test-Path -LiteralPath $run) {
            if ((Get-Item -LiteralPath $run).GetValueNames() -contains 'DesktopCraft') {
                Remove-ItemProperty -LiteralPath $run -Name 'DesktopCraft'
            }
        }
        foreach ($key in @($env:DESKTOPCRAFT_UNINSTALL_VERB_NEW,$env:DESKTOPCRAFT_UNINSTALL_VERB_SETTINGS,$env:DESKTOPCRAFT_UNINSTALL_VERB_PIN)) {
            if ($key -and (Test-Path -LiteralPath ('HKCU:\' + $key))) {
                Remove-Item -LiteralPath ('HKCU:\' + $key) -Recurse -Force
            }
        }
        Remove-KnownFile $env:DESKTOPCRAFT_UNINSTALL_SHORTCUT
    }
    if ($env:DESKTOPCRAFT_UNINSTALL_KEY) {
        $key = 'HKCU:\' + $env:DESKTOPCRAFT_UNINSTALL_KEY
        if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force }
    }
    Remove-Item -LiteralPath $marker -Force
    # Deliberately retain the lock sentinel and any unrelated files.
    Record 'Uninstall cleanup completed; workspace retained'
    $ok = $true
} catch {
    $reason = $_.Exception.Message
    try { Record ('Uninstall cleanup FAILED: ' + $reason) } catch {}
} finally {
    if ($mutex) { $mutex.Dispose() }
    if ($handle) { $handle.Dispose() }
}
if ($env:DESKTOPCRAFT_UNINSTALL_RUN_KEY) {
    try {
        $popup = New-Object -ComObject WScript.Shell
        if ($ok) {
            $null = $popup.Popup('DesktopCraft was removed. Your workspace was retained.', 0, 'DesktopCraft', 64)
        } else {
            $null = $popup.Popup(('Uninstall could not finish: ' + $reason + '. See ' + $log + '. Close DesktopCraft and retry, or reinstall to repair registration.'), 0, 'DesktopCraft', 16)
        }
    } catch {}
}
if ($ok) { exit 0 } else { exit 1 }
"#;
    let mut command = std::process::Command::new(powershell);
    command.args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("DESKTOPCRAFT_UNINSTALL_DIR", dir)
        .env("DESKTOPCRAFT_UNINSTALL_PID", parent_pid.to_string())
        .env("DESKTOPCRAFT_UNINSTALL_KEY", uninstall_key.unwrap_or(""))
        .env("DESKTOPCRAFT_UNINSTALL_TOKEN", token)
        .env("DESKTOPCRAFT_UNINSTALL_LOG", log)
        .env("DESKTOPCRAFT_UNINSTALL_RUN_KEY", "")
        .env("DESKTOPCRAFT_UNINSTALL_VERB_NEW", "")
        .env("DESKTOPCRAFT_UNINSTALL_VERB_SETTINGS", "")
        .env("DESKTOPCRAFT_UNINSTALL_VERB_PIN", "")
        .env("DESKTOPCRAFT_UNINSTALL_SHORTCUT", "")
        .creation_flags(0x08000000);
    if uninstall_key == Some(UNINSTALL_KEY) {
        command.env("DESKTOPCRAFT_UNINSTALL_RUN_KEY", RUN_KEY)
            .env("DESKTOPCRAFT_UNINSTALL_VERB_NEW", VERB_NEW)
            .env("DESKTOPCRAFT_UNINSTALL_VERB_SETTINGS", VERB_SETTINGS)
            .env("DESKTOPCRAFT_UNINSTALL_VERB_PIN", VERB_PIN_FILE)
            .env("DESKTOPCRAFT_UNINSTALL_SHORTCUT", start_menu_link());
    }
    command.spawn().map_err(|e| format!("Could not start uninstall cleanup: {e}"))
}

#[cfg(test)]
#[test]
fn uninstall_cleanup_handles_literal_spaces_brackets_and_unicode() {
    let root = std::env::temp_dir().join(format!("desktopcraft uninstall [test] ภาษาไทย {}-{}",
        std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let dir = root.join(APP_NAME);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["DesktopCraft.exe","DesktopCraft.previous.exe","THIRD-PARTY-NOTICES.txt"] {
        std::fs::write(dir.join(name), b"test").unwrap();
    }
    let sibling = root.join("preserve.txt");
    std::fs::write(&sibling, b"safe").unwrap();
    let log = root.join("uninstall.log");
    std::fs::write(dir.join(".desktopcraft-uninstall-token"), "test-token").unwrap();
    let status = spawn_uninstall_cleanup(&dir, 0, None, "test-token", &log).unwrap().wait().unwrap();
    assert!(status.success());
    assert!(!dir.join("DesktopCraft.exe").exists());
    assert!(!dir.join("DesktopCraft.previous.exe").exists());
    assert!(!dir.join("THIRD-PARTY-NOTICES.txt").exists());
    assert!(dir.join(".desktopcraft-installer.lock").exists());
    assert!(std::fs::read_to_string(&log).unwrap().contains("cleanup completed"));
    assert_eq!(std::fs::read(&sibling).unwrap(), b"safe");
    std::fs::remove_dir_all(&root).unwrap();
}

#[cfg(test)]
#[test]
fn uninstall_cleanup_refuses_a_live_installer_lock_and_preserves_unknown_files() {
    let root = std::env::temp_dir().join(format!("desktopcraft-cleanup-lock-{}-{}",
        std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let dir = root.join(APP_NAME);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("DesktopCraft.exe");
    let unknown = dir.join("user-file.txt");
    std::fs::write(&exe, b"test").unwrap();
    std::fs::write(&unknown, b"keep").unwrap();
    let log = root.join("uninstall.log");
    std::fs::write(dir.join(".desktopcraft-uninstall-token"), "current-token").unwrap();
    let lock = acquire_installer_lock(&dir).unwrap();
    assert!(!spawn_uninstall_cleanup(&dir, 0, None, "current-token", &log).unwrap().wait().unwrap().success());
    assert!(exe.exists());
    drop(lock);
    assert!(!spawn_uninstall_cleanup(&dir, 0, None, "stale-token", &log).unwrap().wait().unwrap().success());
    assert!(exe.exists());
    assert!(spawn_uninstall_cleanup(&dir, 0, None, "current-token", &log).unwrap().wait().unwrap().success());
    assert!(!exe.exists());
    assert_eq!(std::fs::read(&unknown).unwrap(), b"keep");
    std::fs::remove_dir_all(&root).unwrap();
}

#[cfg(test)]
#[test]
fn uninstall_cleanup_retains_registration_on_locked_executable_then_retries() {
    use std::os::windows::fs::OpenOptionsExt;
    let id = format!("{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let root = std::env::temp_dir().join(format!("desktopcraft-uninstall-retry-{id}"));
    let dir = root.join(APP_NAME);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("DesktopCraft.exe");
    std::fs::write(&exe, b"test").unwrap();
    let log = root.join("uninstall.log");
    let key = format!(r"Software\DesktopCraft-Test\Uninstall-{id}");
    assert!(reg_set(&key, Some("DisplayName"), "Isolated uninstall test"));
    std::fs::write(dir.join(".desktopcraft-uninstall-token"), "test-token").unwrap();
    let locked = std::fs::OpenOptions::new().read(true).share_mode(1).open(&exe).unwrap();
    let result = spawn_uninstall_cleanup(&dir, 0, Some(&key), "test-token", &log).unwrap().wait().unwrap();
    assert!(!result.success());
    assert!(exe.exists());
    assert!(reg_key_exists(&key));
    assert!(std::fs::read_to_string(&log).unwrap().contains("cleanup FAILED"));
    drop(locked);
    let result = spawn_uninstall_cleanup(&dir, 0, Some(&key), "test-token", &log).unwrap().wait().unwrap();
    assert!(result.success());
    assert!(!exe.exists());
    assert!(!reg_key_exists(&key));
    std::fs::remove_dir_all(&root).unwrap();
}

/// Remove everything install() added, guarded by the same quiescence barrier
/// as install(). Layout data in %APPDATA% is kept.
pub fn uninstall() -> Result<(), String> {
    let dir = install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _exclusive_lock = acquire_installer_lock(&dir)?;
    if dir.join(".desktopcraft-install-tx.json").try_exists().map_err(|e| e.to_string())? {
        return Err("Install transaction pending; resolve it before uninstall".into());
    }
    wait_for_installer_quiescence()?;
    let desktop_instance = claim_desktop_instance()
        .ok_or("DesktopCraft started during uninstall preflight; close it and retry")?;
    let journal = data_dir().join("native-inbox-recovery.json");
    if inspect_recovery_journal_until(&journal,
        std::time::Instant::now()+std::time::Duration::from_secs(5))?
        || crate::native_filter_host::native_policy_status()["mapping_absence_confirmed"] != true {
        return Err("Explorer detach and recovery could not be confirmed; uninstall refused".into());
    }
    // Do not remove registration in this process: its own EXE is still mapped.
    let token = format!("{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos());
    let marker = dir.join(".desktopcraft-uninstall-token");
    std::fs::write(&marker, &token).map_err(|e| format!("Cannot write uninstall token: {e}"))?;
    let log_dir = data_dir();
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let log = log_dir.join("uninstall.log");
    // Spawn while both barriers are held; the child waits for this process to exit.
    if let Err(e) = spawn_uninstall_cleanup(&dir, std::process::id(), Some(UNINSTALL_KEY), &token, &log) {
        let _ = std::fs::remove_file(&marker);
        return Err(e);
    }
    drop(desktop_instance);
    Ok(())
}
