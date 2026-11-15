//! Read-only desktop integration for our Iced caption buttons. This follows
//! Electron's freedesktop icon/button conventions, not KWin decoration plugins.
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Minimize,
    Maximize,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layout {
    pub left: Vec<Control>,
    pub right: Vec<Control>,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            left: vec![],
            right: vec![Control::Minimize, Control::Maximize, Control::Close],
        }
    }
}
impl Layout {
    fn kde(left: Option<&str>, right: Option<&str>) -> Self {
        fn controls(value: &str) -> Vec<Control> {
            value
                .chars()
                .filter_map(|c| match c {
                    'I' => Some(Control::Minimize),
                    'A' => Some(Control::Maximize),
                    'X' => Some(Control::Close),
                    _ => None,
                })
                .collect()
        }
        Self {
            left: controls(left.unwrap_or("")),
            right: controls(right.unwrap_or("IAX")),
        }
        .deduplicated()
    }
    fn gtk(value: &str) -> Option<Self> {
        let (left, right) = value.split_once(':')?;
        fn controls(value: &str) -> Vec<Control> {
            value
                .split(',')
                .filter_map(|c| match c.trim() {
                    "minimize" => Some(Control::Minimize),
                    "maximize" => Some(Control::Maximize),
                    "close" => Some(Control::Close),
                    _ => None,
                })
                .collect()
        }
        Some(
            Self {
                left: controls(left),
                right: controls(right),
            }
            .deduplicated(),
        )
    }
    fn deduplicated(mut self) -> Self {
        let mut seen = Vec::new();
        for side in [&mut self.left, &mut self.right] {
            side.retain(|control| {
                if seen.contains(control) {
                    false
                } else {
                    seen.push(*control);
                    true
                }
            });
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Appearance {
    Breeze,
    Adwaita,
}

#[derive(Debug, Clone)]
pub(super) struct DesktopTheme {
    pub appearance: Appearance,
    pub layout: Layout,
    pub icons: [Option<PathBuf>; 4],
    pub icon_theme: String,
}

impl DesktopTheme {
    pub async fn load() -> Self {
        let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let config = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let mut configs = vec![config.clone(), config.join("kdedefaults")];
        configs.extend(env::split_paths(
            &env::var_os("XDG_CONFIG_DIRS").unwrap_or_else(|| "/etc/xdg".into()),
        ));
        let desktop = format!(
            "{}:{}",
            env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            env::var("DESKTOP_SESSION").unwrap_or_default()
        )
        .to_lowercase();
        let kde = desktop.contains("kde") || desktop.contains("plasma");
        let gnome = desktop.contains("gnome");
        let gtk_files: Vec<_> = configs
            .iter()
            .flat_map(|dir| {
                [
                    dir.join("gtk-3.0/settings.ini"),
                    dir.join("gtk-4.0/settings.ini"),
                ]
            })
            .collect();
        let kde_files: Vec<_> = configs.iter().map(|dir| dir.join("kdeglobals")).collect();
        let kwin_files: Vec<_> = configs.iter().map(|dir| dir.join("kwinrc")).collect();
        // GSettings has a bounded wait and is queried once at startup, never in view().
        let gsettings = if !kde {
            setting(&["list-recursively", "org.gnome.desktop.interface"]).await
        } else {
            None
        };
        let gtk = |key| first_ini(&gtk_files, "Settings", key);
        let interface = |key: &str| {
            gsettings.as_deref().and_then(|value| {
                value.lines().find_map(|line| {
                    let mut fields = line.splitn(3, char::is_whitespace);
                    let _schema = fields.next()?;
                    (fields.next()? == key).then(|| unquote(fields.next().unwrap_or_default()))
                })
            })
        };
        let icon_theme = if kde {
            first_ini(&kde_files, "Icons", "Theme").or_else(|| gtk("gtk-icon-theme-name"))
        } else {
            interface("icon-theme").or_else(|| gtk("gtk-icon-theme-name"))
        }
        .unwrap_or_else(|| if kde { "breeze" } else { "Adwaita" }.into());
        let gtk_theme = env::var("GTK_THEME")
            .ok()
            .or_else(|| gtk("gtk-theme-name"))
            .or_else(|| interface("gtk-theme"));
        let appearance = if gtk_theme
            .as_deref()
            .is_some_and(|name| name.to_lowercase().starts_with("breeze"))
            || (gtk_theme.is_none() && kde)
        {
            Appearance::Breeze
        } else {
            Appearance::Adwaita
        };
        let layout = if kde {
            Layout::kde(
                first_ini(&kwin_files, "org.kde.kdecoration2", "ButtonsOnLeft").as_deref(),
                first_ini(&kwin_files, "org.kde.kdecoration2", "ButtonsOnRight").as_deref(),
            )
        } else {
            let native = if gnome {
                setting(&["get", "org.gnome.desktop.wm.preferences", "button-layout"])
                    .await
                    .map(|s| unquote(&s))
            } else {
                None
            };
            native
                .or_else(|| gtk("gtk-decoration-layout"))
                .and_then(|s| Layout::gtk(&s))
                .unwrap_or_else(|| {
                    if gnome {
                        Layout {
                            left: vec![],
                            right: vec![Control::Close],
                        }
                    } else {
                        Layout::default()
                    }
                })
        };
        let roots = icon_roots(&home);
        let names = [
            "window-minimize-symbolic",
            "window-maximize-symbolic",
            "window-restore-symbolic",
            "window-close-symbolic",
        ];
        let chain = theme_chain(&icon_theme, |theme| {
            roots
                .iter()
                .filter_map(|root| fs::read_to_string(root.join(theme).join("index.theme")).ok())
                .filter_map(|text| ini(&text, "Icon Theme", "Inherits"))
                .flat_map(|parents| {
                    parents
                        .split(',')
                        .map(str::trim)
                        .map(String::from)
                        .collect::<Vec<_>>()
                })
                .collect()
        });
        let icons = names.map(|name| lookup_svg(name, &chain, &roots));
        Self {
            appearance,
            layout,
            icons,
            icon_theme,
        }
    }
}

async fn setting(args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new("gsettings");
    command.args(args).kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_millis(750), command.output())
        .await
        .ok()?
        .ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}
fn unquote(value: &str) -> String {
    value.trim().trim_matches(['\'', '"']).to_owned()
}
fn first_ini(files: &[PathBuf], section: &str, key: &str) -> Option<String> {
    files.iter().find_map(|path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| ini(&text, section, key))
    })
}
fn ini(text: &str, section: &str, key: &str) -> Option<String> {
    let mut selected = false;
    let mut found = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with(['#', ';']) {
            continue;
        }
        if line.starts_with('[') {
            selected = line
                .strip_prefix('[')
                .and_then(|line| line.split_once(']'))
                .is_some_and(|(name, _)| name == section);
        } else if selected
            && let Some((name, value)) = line.split_once('=')
            && name.trim().trim_end_matches("[$i]") == key
        {
            found = Some(unquote(value));
        }
    }
    found
}
fn icon_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        home.join(".icons"),
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("icons"),
    ];
    roots.extend(
        env::split_paths(
            &env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
        )
        .map(|dir| dir.join("icons")),
    );
    roots
}
fn valid_theme(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}
// The crate searches immediate parents; walk deeper inheritance explicitly,
// preserving order and protecting against malformed themes with parent cycles.
fn theme_chain(theme: &str, mut parents: impl FnMut(&str) -> Vec<String>) -> Vec<String> {
    fn visit(
        name: &str,
        result: &mut Vec<String>,
        seen: &mut HashSet<String>,
        parents: &mut impl FnMut(&str) -> Vec<String>,
    ) {
        if !valid_theme(name)
            || name == "hicolor"
            || !seen.insert(name.into())
            || result.len() >= 64
        {
            return;
        }
        result.push(name.into());
        for parent in parents(name) {
            visit(&parent, result, seen, parents);
        }
    }
    let mut result = Vec::new();
    visit(theme, &mut result, &mut HashSet::new(), &mut parents);
    result.push("hicolor".into());
    result
}
fn lookup_svg(name: &str, themes: &[String], roots: &[PathBuf]) -> Option<PathBuf> {
    // The explicit per-theme check prevents the crate's hicolor fallback from
    // winning before a more distant inherited theme. SVGs can be recolored by Iced.
    for theme in themes {
        let path = freedesktop_icons::lookup(name)
            .with_theme(theme)
            .with_size(16)
            .force_svg()
            .with_cache()
            .find();
        if let Some(path) = path
            && path.extension().is_some_and(|extension| extension == "svg")
            && roots.iter().any(|root| path.starts_with(root.join(theme)))
        {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kde_preserves_order_omissions_and_explicit_empty() {
        assert_eq!(
            Layout::kde(Some("XM"), Some("AI")),
            Layout {
                left: vec![Control::Close],
                right: vec![Control::Maximize, Control::Minimize]
            }
        );
        assert_eq!(
            Layout::kde(Some(""), Some("")),
            Layout {
                left: vec![],
                right: vec![]
            }
        );
        assert_eq!(Layout::kde(None, None), Layout::default());
        assert_eq!(
            Layout::kde(Some("X"), Some("IXA")),
            Layout {
                left: vec![Control::Close],
                right: vec![Control::Minimize, Control::Maximize]
            }
        );
    }
    #[test]
    fn gtk_layout_includes_single_sided_and_no_buttons() {
        assert_eq!(
            Layout::gtk("close,minimize:maximize"),
            Some(Layout {
                left: vec![Control::Close, Control::Minimize],
                right: vec![Control::Maximize]
            })
        );
        assert_eq!(
            Layout::gtk(":close"),
            Some(Layout {
                left: vec![],
                right: vec![Control::Close]
            })
        );
        assert_eq!(
            Layout::gtk(":"),
            Some(Layout {
                left: vec![],
                right: vec![]
            })
        );
        assert_eq!(Layout::gtk("invalid"), None);
    }
    #[test]
    fn settings_distinguish_empty_missing_and_other_groups() {
        let text = "[other]\nButtonsOnRight=X\n[org.kde.kdecoration2]\nButtonsOnLeft=\nButtonsOnRight=IA\n";
        assert_eq!(
            ini(text, "org.kde.kdecoration2", "ButtonsOnLeft"),
            Some(String::new())
        );
        assert_eq!(
            ini(text, "org.kde.kdecoration2", "ButtonsOnRight"),
            Some("IA".into())
        );
        assert_eq!(ini(text, "org.kde.kdecoration2", "Missing"), None);
    }
    #[test]
    fn inherits_recursively_in_order_with_cycle_and_hicolor_fallback() {
        let chain = theme_chain("custom", |theme| match theme {
            "custom" => vec!["parent".into(), "other".into()],
            "parent" => vec!["grandparent".into()],
            "grandparent" => vec!["custom".into(), "hicolor".into()],
            _ => vec![],
        });
        assert_eq!(
            chain,
            ["custom", "parent", "grandparent", "other", "hicolor"]
        );
        assert_eq!(
            theme_chain("../invalid", |_| panic!("invalid theme visited")),
            ["hicolor"]
        );
    }
}
