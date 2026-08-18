use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserBackend {
    ChromiumCdp,
    WebDriverSafari,
    WebDriverFirefox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserKind {
    CustomChromium,
    Chrome,
    Edge,
    Brave,
    Arc,
    Vivaldi,
    Chromium,
    Opera,
    Firefox,
    Safari,
    ManagedChromium,
}

impl BrowserKind {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::CustomChromium => "自定义 Chromium 浏览器",
            Self::Chrome => "Google Chrome",
            Self::Edge => "Microsoft Edge",
            Self::Brave => "Brave",
            Self::Arc => "Arc",
            Self::Vivaldi => "Vivaldi",
            Self::Chromium => "Chromium",
            Self::Opera => "Opera",
            Self::Firefox => "Mozilla Firefox",
            Self::Safari => "Safari",
            Self::ManagedChromium => "Punctual 托管浏览器",
        }
    }

    pub const fn slug(self) -> &'static str {
        match self {
            Self::CustomChromium => "custom-chromium",
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Brave => "brave",
            Self::Arc => "arc",
            Self::Vivaldi => "vivaldi",
            Self::Chromium => "chromium",
            Self::Opera => "opera",
            Self::Firefox => "firefox",
            Self::Safari => "safari",
            Self::ManagedChromium => "managed-chromium",
        }
    }

    pub const fn backend(self) -> BrowserBackend {
        match self {
            Self::Firefox => BrowserBackend::WebDriverFirefox,
            Self::Safari => BrowserBackend::WebDriverSafari,
            _ => BrowserBackend::ChromiumCdp,
        }
    }

    fn from_preference(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "chrome" | "google-chrome" => Some(Self::Chrome),
            "edge" | "microsoft-edge" => Some(Self::Edge),
            "brave" => Some(Self::Brave),
            "arc" => Some(Self::Arc),
            "vivaldi" => Some(Self::Vivaldi),
            "chromium" => Some(Self::Chromium),
            "opera" => Some(Self::Opera),
            "firefox" => Some(Self::Firefox),
            "safari" => Some(Self::Safari),
            "managed" | "managed-chromium" | "punctual" => Some(Self::ManagedChromium),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserInstallation {
    pub kind: BrowserKind,
    pub executable: Option<PathBuf>,
    pub driver: Option<PathBuf>,
    pub is_default: bool,
    pub is_managed: bool,
    pub source: String,
}

impl BrowserInstallation {
    pub fn display_name(&self) -> &'static str {
        self.kind.display_name()
    }

    pub fn backend(&self) -> BrowserBackend {
        self.kind.backend()
    }

    pub fn profile_dir(&self, profile_root: &Path) -> PathBuf {
        // Preserve the profile location used by alpha.2-alpha.4 for Chrome so
        // existing users keep their Punctual browser login state.
        if self.kind == BrowserKind::Chrome {
            profile_root.to_path_buf()
        } else {
            profile_root.join(self.kind.slug())
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrowserDiscoveryOptions {
    pub explicit_executable: Option<PathBuf>,
    pub preference: Option<String>,
    pub resources_dir: Option<PathBuf>,
}

impl Default for BrowserDiscoveryOptions {
    fn default() -> Self {
        Self {
            explicit_executable: env::var_os("PUNCTUAL_CHROMIUM").map(PathBuf::from),
            preference: env::var("PUNCTUAL_BROWSER").ok(),
            resources_dir: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrowserInventory {
    pub installations: Vec<BrowserInstallation>,
    pub default_kind: Option<BrowserKind>,
}

impl BrowserInventory {
    pub fn preferred(&self) -> Option<&BrowserInstallation> {
        self.installations.first()
    }

    pub fn available_names(&self) -> Vec<&'static str> {
        self.installations
            .iter()
            .map(BrowserInstallation::display_name)
            .collect()
    }

    pub fn summary_zh(&self) -> String {
        let available = self.available_names();
        if available.is_empty() {
            return "没有检测到可自动控制的浏览器".into();
        }
        let selected = self
            .preferred()
            .map(BrowserInstallation::display_name)
            .unwrap_or("未知浏览器");
        let default = self
            .default_kind
            .map(BrowserKind::display_name)
            .unwrap_or("未识别");
        format!(
            "已检测到 {}；系统默认：{}；将优先使用 {}",
            available.join("、"),
            default,
            selected
        )
    }
}

pub fn discover_browsers(options: &BrowserDiscoveryOptions) -> BrowserInventory {
    let default_kind = detect_default_browser_kind();
    let preferred_kind = options
        .preference
        .as_deref()
        .and_then(BrowserKind::from_preference);

    let mut installations = Vec::new();

    if let Some(path) = options
        .explicit_executable
        .as_ref()
        .filter(|path| path.is_file())
    {
        installations.push(BrowserInstallation {
            kind: infer_chromium_kind(path).unwrap_or(BrowserKind::CustomChromium),
            executable: Some(path.clone()),
            driver: None,
            is_default: false,
            is_managed: false,
            source: "环境变量 PUNCTUAL_CHROMIUM".into(),
        });
    }

    installations.extend(platform_installations(default_kind, options.resources_dir.as_deref()));

    if let Some(managed) = managed_browser_installation(options.resources_dir.as_deref()) {
        installations.push(managed);
    }

    deduplicate(&mut installations);
    installations.sort_by_key(|installation| {
        priority(installation, preferred_kind, default_kind)
    });

    BrowserInventory {
        installations,
        default_kind,
    }
}

fn priority(
    installation: &BrowserInstallation,
    preferred_kind: Option<BrowserKind>,
    default_kind: Option<BrowserKind>,
) -> (u16, &'static str) {
    let rank = if installation.source.contains("PUNCTUAL_CHROMIUM") {
        0
    } else if preferred_kind == Some(installation.kind) {
        1
    } else if installation.kind == BrowserKind::Chrome {
        // Product rule: Chrome wins over the system default when both exist.
        10
    } else if default_kind == Some(installation.kind) {
        20
    } else {
        match installation.kind {
            BrowserKind::Edge => 30,
            BrowserKind::Brave => 31,
            BrowserKind::Arc => 32,
            BrowserKind::Vivaldi => 33,
            BrowserKind::Chromium => 34,
            BrowserKind::Opera => 35,
            BrowserKind::Firefox => 40,
            BrowserKind::Safari => 50,
            BrowserKind::ManagedChromium => 100,
            BrowserKind::CustomChromium => 5,
            BrowserKind::Chrome => 10,
        }
    };
    (rank, installation.display_name())
}

fn deduplicate(installations: &mut Vec<BrowserInstallation>) {
    let mut seen = HashSet::new();
    installations.retain(|installation| {
        let key = (
            installation.kind,
            installation
                .executable
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        );
        seen.insert(key)
    });
}

fn add_if_exists(
    output: &mut Vec<BrowserInstallation>,
    kind: BrowserKind,
    path: impl Into<PathBuf>,
    default_kind: Option<BrowserKind>,
    source: &str,
    driver: Option<PathBuf>,
) {
    let path = path.into();
    if path.is_file() {
        output.push(BrowserInstallation {
            kind,
            executable: Some(path),
            driver,
            is_default: default_kind == Some(kind),
            is_managed: false,
            source: source.into(),
        });
    }
}

#[cfg(target_os = "macos")]
fn platform_installations(
    default_kind: Option<BrowserKind>,
    resources_dir: Option<&Path>,
) -> Vec<BrowserInstallation> {
    let mut output = Vec::new();
    let home = env::var_os("HOME").map(PathBuf::from);
    let geckodriver = find_geckodriver(resources_dir);

    let candidates = [
        (BrowserKind::Chrome, "Google Chrome.app/Contents/MacOS/Google Chrome"),
        (BrowserKind::Edge, "Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        (BrowserKind::Brave, "Brave Browser.app/Contents/MacOS/Brave Browser"),
        (BrowserKind::Arc, "Arc.app/Contents/MacOS/Arc"),
        (BrowserKind::Vivaldi, "Vivaldi.app/Contents/MacOS/Vivaldi"),
        (BrowserKind::Chromium, "Chromium.app/Contents/MacOS/Chromium"),
        (BrowserKind::Opera, "Opera.app/Contents/MacOS/Opera"),
        (BrowserKind::Firefox, "Firefox.app/Contents/MacOS/firefox"),
    ];

    for (kind, relative) in candidates {
        let driver = (kind == BrowserKind::Firefox)
            .then(|| geckodriver.clone())
            .flatten();
        add_if_exists(
            &mut output,
            kind,
            Path::new("/Applications").join(relative),
            default_kind,
            "/Applications",
            driver.clone(),
        );
        if let Some(home) = &home {
            add_if_exists(
                &mut output,
                kind,
                home.join("Applications").join(relative),
                default_kind,
                "~/Applications",
                driver,
            );
        }
    }

    let safari = PathBuf::from("/Applications/Safari.app/Contents/MacOS/Safari");
    let safaridriver = PathBuf::from("/usr/bin/safaridriver");
    if safari.is_file() && safaridriver.is_file() {
        output.push(BrowserInstallation {
            kind: BrowserKind::Safari,
            executable: Some(safari),
            driver: Some(safaridriver),
            is_default: default_kind == Some(BrowserKind::Safari),
            is_managed: false,
            source: "macOS 系统浏览器".into(),
        });
    }

    output
}

#[cfg(target_os = "windows")]
fn platform_installations(
    default_kind: Option<BrowserKind>,
    resources_dir: Option<&Path>,
) -> Vec<BrowserInstallation> {
    let mut output = Vec::new();
    let local = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let program_files = env::var_os("ProgramFiles").map(PathBuf::from);
    let program_files_x86 = env::var_os("ProgramFiles(x86)").map(PathBuf::from);
    let app_data = env::var_os("APPDATA").map(PathBuf::from);
    let geckodriver = find_geckodriver(resources_dir);

    let mut roots = Vec::new();
    if let Some(path) = local {
        roots.push(path);
    }
    if let Some(path) = program_files {
        roots.push(path);
    }
    if let Some(path) = program_files_x86 {
        roots.push(path);
    }

    for root in roots {
        for (kind, relative) in [
            (BrowserKind::Chrome, "Google/Chrome/Application/chrome.exe"),
            (BrowserKind::Edge, "Microsoft/Edge/Application/msedge.exe"),
            (BrowserKind::Brave, "BraveSoftware/Brave-Browser/Application/brave.exe"),
            (BrowserKind::Vivaldi, "Vivaldi/Application/vivaldi.exe"),
            (BrowserKind::Chromium, "Chromium/Application/chrome.exe"),
            (BrowserKind::Firefox, "Mozilla Firefox/firefox.exe"),
        ] {
            let driver = (kind == BrowserKind::Firefox)
                .then(|| geckodriver.clone())
                .flatten();
            add_if_exists(
                &mut output,
                kind,
                root.join(relative),
                default_kind,
                "Windows 安装目录",
                driver,
            );
        }
    }

    if let Some(app_data) = app_data {
        add_if_exists(
            &mut output,
            BrowserKind::Opera,
            app_data.join("Opera Software/Opera Stable/opera.exe"),
            default_kind,
            "Windows 用户安装目录",
            None,
        );
    }
    output
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_installations(
    default_kind: Option<BrowserKind>,
    resources_dir: Option<&Path>,
) -> Vec<BrowserInstallation> {
    let mut output = Vec::new();
    let geckodriver = find_geckodriver(resources_dir);
    for (kind, names) in [
        (BrowserKind::Chrome, &["google-chrome", "google-chrome-stable"][..]),
        (BrowserKind::Edge, &["microsoft-edge", "microsoft-edge-stable"][..]),
        (BrowserKind::Brave, &["brave-browser"][..]),
        (BrowserKind::Vivaldi, &["vivaldi", "vivaldi-stable"][..]),
        (BrowserKind::Chromium, &["chromium", "chromium-browser"][..]),
        (BrowserKind::Opera, &["opera"][..]),
        (BrowserKind::Firefox, &["firefox"][..]),
    ] {
        if let Some(path) = names.iter().find_map(|name| find_in_path(name)) {
            output.push(BrowserInstallation {
                kind,
                executable: Some(path),
                driver: (kind == BrowserKind::Firefox)
                    .then(|| geckodriver.clone())
                    .flatten(),
                is_default: default_kind == Some(kind),
                is_managed: false,
                source: "PATH".into(),
            });
        }
    }
    output
}

fn managed_browser_installation(resources_dir: Option<&Path>) -> Option<BrowserInstallation> {
    let explicit = env::var_os("PUNCTUAL_MANAGED_BROWSER").map(PathBuf::from);
    let path = explicit.or_else(|| {
        let resources = resources_dir?;
        managed_browser_candidates(resources)
            .into_iter()
            .find(|path| path.is_file())
    })?;
    path.is_file().then_some(BrowserInstallation {
        kind: BrowserKind::ManagedChromium,
        executable: Some(path),
        driver: None,
        is_default: false,
        is_managed: true,
        source: "Punctual 应用内置资源".into(),
    })
}

fn managed_browser_candidates(resources: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return vec![resources.join(
            "managed-browser/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        )];
    }
    #[cfg(target_os = "windows")]
    {
        return vec![
            resources.join("managed-browser/chrome-win64/chrome.exe"),
            resources.join("managed-browser/chrome.exe"),
        ];
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![
            resources.join("managed-browser/chrome-linux64/chrome"),
            resources.join("managed-browser/chrome"),
        ]
    }
}

fn find_geckodriver(resources_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = env::var_os("PUNCTUAL_GECKODRIVER").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(resources) = resources_dir {
        #[cfg(target_os = "windows")]
        let bundled = resources.join("bin/geckodriver.exe");
        #[cfg(not(target_os = "windows"))]
        let bundled = resources.join("bin/geckodriver");
        if bundled.is_file() {
            return Some(bundled);
        }
    }
    #[cfg(target_os = "windows")]
    let name = "geckodriver.exe";
    #[cfg(not(target_os = "windows"))]
    let name = "geckodriver";
    find_in_path(name)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|path| path.join(name))
        .find(|path| path.is_file())
}

#[cfg(target_os = "macos")]
fn detect_default_browser_kind() -> Option<BrowserKind> {
    let home = env::var_os("HOME").map(PathBuf::from)?;
    let plist = home.join(
        "Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist",
    );
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "LSHandlers", "json", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    if !output.status.success() {
        return Some(BrowserKind::Safari);
    }
    let handlers: Vec<Value> = serde_json::from_slice(&output.stdout).ok()?;
    for scheme in ["https", "http"] {
        if let Some(kind) = handlers.iter().find_map(|handler| {
            (handler.get("LSHandlerURLScheme")?.as_str()? == scheme)
                .then(|| {
                    handler
                        .get("LSHandlerRoleAll")
                        .or_else(|| handler.get("LSHandlerRoleViewer"))
                        .and_then(Value::as_str)
                        .and_then(kind_from_macos_bundle_id)
                })
                .flatten()
        }) {
            return Some(kind);
        }
    }
    Some(BrowserKind::Safari)
}

#[cfg(target_os = "windows")]
fn detect_default_browser_kind() -> Option<BrowserKind> {
    let output = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
            "/v",
            "ProgId",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    if text.contains("chrome") {
        Some(BrowserKind::Chrome)
    } else if text.contains("microsoftedge") || text.contains("mse") {
        Some(BrowserKind::Edge)
    } else if text.contains("brave") {
        Some(BrowserKind::Brave)
    } else if text.contains("vivaldi") {
        Some(BrowserKind::Vivaldi)
    } else if text.contains("opera") {
        Some(BrowserKind::Opera)
    } else if text.contains("firefox") {
        Some(BrowserKind::Firefox)
    } else {
        None
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn detect_default_browser_kind() -> Option<BrowserKind> {
    let output = Command::new("xdg-settings")
        .args(["get", "default-web-browser"])
        .output()
        .ok()?;
    infer_kind_from_text(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "macos")]
fn kind_from_macos_bundle_id(bundle_id: &str) -> Option<BrowserKind> {
    let id = bundle_id.to_ascii_lowercase();
    if id.contains("google.chrome") {
        Some(BrowserKind::Chrome)
    } else if id.contains("microsoft.edgemac") {
        Some(BrowserKind::Edge)
    } else if id.contains("brave.browser") {
        Some(BrowserKind::Brave)
    } else if id.contains("thebrowser.browser") {
        Some(BrowserKind::Arc)
    } else if id.contains("vivaldi") {
        Some(BrowserKind::Vivaldi)
    } else if id.contains("chromium") {
        Some(BrowserKind::Chromium)
    } else if id.contains("opera") {
        Some(BrowserKind::Opera)
    } else if id.contains("firefox") {
        Some(BrowserKind::Firefox)
    } else if id.contains("safari") {
        Some(BrowserKind::Safari)
    } else {
        None
    }
}

fn infer_chromium_kind(path: &Path) -> Option<BrowserKind> {
    infer_kind_from_text(&path.to_string_lossy()).filter(|kind| {
        !matches!(kind, BrowserKind::Firefox | BrowserKind::Safari)
    })
}

fn infer_kind_from_text(value: &str) -> Option<BrowserKind> {
    let value = value.to_ascii_lowercase();
    if value.contains("chrome") && !value.contains("chromium") {
        Some(BrowserKind::Chrome)
    } else if value.contains("edge") || value.contains("msedge") {
        Some(BrowserKind::Edge)
    } else if value.contains("brave") {
        Some(BrowserKind::Brave)
    } else if value.contains("arc") {
        Some(BrowserKind::Arc)
    } else if value.contains("vivaldi") {
        Some(BrowserKind::Vivaldi)
    } else if value.contains("chromium") {
        Some(BrowserKind::Chromium)
    } else if value.contains("opera") {
        Some(BrowserKind::Opera)
    } else if value.contains("firefox") {
        Some(BrowserKind::Firefox)
    } else if value.contains("safari") {
        Some(BrowserKind::Safari)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installation(kind: BrowserKind, is_default: bool) -> BrowserInstallation {
        BrowserInstallation {
            kind,
            executable: Some(PathBuf::from(format!("/tmp/{}", kind.slug()))),
            driver: None,
            is_default,
            is_managed: kind == BrowserKind::ManagedChromium,
            source: "test".into(),
        }
    }

    #[test]
    fn chrome_wins_over_a_different_system_default() {
        let mut items = vec![
            installation(BrowserKind::Safari, true),
            installation(BrowserKind::Chrome, false),
            installation(BrowserKind::ManagedChromium, false),
        ];
        items.sort_by_key(|item| priority(item, None, Some(BrowserKind::Safari)));
        assert_eq!(items[0].kind, BrowserKind::Chrome);
        assert_eq!(items[1].kind, BrowserKind::Safari);
        assert_eq!(items[2].kind, BrowserKind::ManagedChromium);
    }

    #[test]
    fn supported_default_wins_when_chrome_is_absent() {
        let mut items = vec![
            installation(BrowserKind::Edge, false),
            installation(BrowserKind::Firefox, true),
        ];
        items.sort_by_key(|item| priority(item, None, Some(BrowserKind::Firefox)));
        assert_eq!(items[0].kind, BrowserKind::Firefox);
    }

    #[test]
    fn explicit_preference_overrides_chrome() {
        let mut items = vec![
            installation(BrowserKind::Chrome, false),
            installation(BrowserKind::Safari, true),
        ];
        items.sort_by_key(|item| {
            priority(item, Some(BrowserKind::Safari), Some(BrowserKind::Safari))
        });
        assert_eq!(items[0].kind, BrowserKind::Safari);
    }
}
